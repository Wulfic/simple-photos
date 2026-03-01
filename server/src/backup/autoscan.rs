use axum::extract::State;
use axum::Json;
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::state::AppState;

/// Compute short content-based hash: first 12 hex chars of SHA-256.
fn compute_photo_hash(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex::encode(&digest[..6])
}

/// Background task: automatically scan the storage directory for new files
/// every 24 hours (or when triggered by an API call).
pub async fn background_auto_scan_task(
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
    interval_secs: u64,
) {
    if interval_secs == 0 {
        tracing::info!("Background auto-scan disabled (interval = 0)");
        return;
    }

    // Run an initial scan shortly after startup
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    tracing::info!("Running startup auto-scan...");
    let count = run_auto_scan(&pool, &storage_root).await;
    if count > 0 {
        tracing::info!("Startup auto-scan: registered {} new files", count);
    }
    update_last_scan_time(&pool).await;

    // Then scan on a configurable interval
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    tracing::info!("Auto-scan interval: every {} seconds", interval_secs);

    loop {
        interval.tick().await;

        let count = run_auto_scan(&pool, &storage_root).await;
        if count > 0 {
            tracing::info!("Auto-scan: registered {} new files", count);
        }
        update_last_scan_time(&pool).await;
    }
}

async fn update_last_scan_time(pool: &sqlx::SqlitePool) {
    let now = Utc::now().to_rfc3339();
    let _ = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('last_auto_scan', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&now)
    .execute(pool)
    .await;
}

/// POST /api/admin/photos/auto-scan
/// Trigger an immediate auto-scan (called when web UI or app opens).
/// Runs synchronously so the client can await completion before loading photos.
pub async fn trigger_auto_scan(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state.pool.clone();
    let storage_root = state.storage_root.read().await.clone();

    let count = run_auto_scan(&pool, &storage_root).await;
    if count > 0 {
        tracing::info!("On-demand scan: registered {} new files", count);
    }

    // Update last scan time
    update_last_scan_time(&pool).await;

    Ok(Json(serde_json::json!({
        "message": "Scan complete",
        "new_count": count,
    })))
}

/// Scan storage directory and register any unregistered media files for ALL users.
async fn run_auto_scan(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) -> i64 {
    // Get the first admin user to assign new photos to
    let admin_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let admin_id = match admin_id {
        Some(id) => id,
        None => return 0, // No admin user yet
    };

    // Get already-registered file paths
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM photos",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let existing_set: std::collections::HashSet<String> = existing.into_iter().collect();

    let mut new_count = 0i64;
    let mut queue = vec![storage_root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && is_media_file(&name) {
                    let abs_path = entry.path();
                    // Normalize to forward slashes so DB paths are consistent across OS
                    let rel_path = abs_path
                        .strip_prefix(storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    if existing_set.contains(&rel_path) {
                        continue;
                    }

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                        })
                    });

                    let mime = mime_from_extension(&name).to_string();
                    let media_type = if mime.starts_with("video/") {
                        "video"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    let photo_id = Uuid::new_v4().to_string();
                    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                    let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

                    // Compute content-based hash for cross-platform alignment
                    let photo_hash = match tokio::fs::read(&abs_path).await {
                        Ok(bytes) => Some(compute_photo_hash(&bytes)),
                        Err(_) => None,
                    };

                    let _ = sqlx::query(
                        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                         size_bytes, width, height, taken_at, thumb_path, created_at, photo_hash) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?)",
                    )
                    .bind(&photo_id)
                    .bind(&admin_id)
                    .bind(&name)
                    .bind(&rel_path)
                    .bind(&mime)
                    .bind(media_type)
                    .bind(size)
                    .bind(&modified)
                    .bind(&thumb_rel)
                    .bind(&now)
                    .bind(&photo_hash)
                    .execute(pool)
                    .await;

                    new_count += 1;
                }
            }
        }
    }

    new_count
}
