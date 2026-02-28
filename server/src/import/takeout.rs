//! Google Photos Takeout directory scanning and bulk import.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::blobs::storage as blob_storage;
use crate::error::AppError;
use crate::media::is_media_file;
use crate::state::AppState;

use super::google_photos;

// ── Scan Google Photos Takeout directory ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TakeoutScanQuery {
    pub path: String,
}

#[derive(Debug, serde::Serialize)]
pub struct TakeoutScanResponse {
    pub directory: String,
    pub media_files: usize,
    pub sidecar_files: usize,
    pub paired: usize,
    pub unpaired_media: Vec<String>,
    pub unpaired_sidecars: Vec<String>,
}

/// GET /api/admin/import/google-photos/scan?path=/path/to/takeout
///
/// Scan a Google Photos Takeout directory, find media files + JSON sidecars,
/// and report which files are paired (media + matching .json sidecar).
pub async fn scan_takeout(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<TakeoutScanQuery>,
) -> Result<Json<TakeoutScanResponse>, AppError> {
    // Admin check
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    if query.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let scan_path = std::path::PathBuf::from(&query.path);
    let canonical = tokio::fs::canonicalize(&scan_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot resolve path '{}': {}", query.path, e))
    })?;

    let mut media_files: Vec<String> = Vec::new();
    let mut sidecar_files: Vec<String> = Vec::new();
    let mut queue = vec![canonical.clone()];

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
                } else if ft.is_file() {
                    let path_str = entry.path().display().to_string();
                    if name.ends_with(".supplemental-metadata.json")
                        || (name.ends_with(".json")
                            && !name.ends_with(".supplemental-metadata.json")
                            && is_google_photos_json(&name))
                    {
                        sidecar_files.push(path_str);
                    } else if is_media_file(&name) {
                        media_files.push(path_str);
                    }
                }
            }
        }
    }

    // Pair media with sidecars:
    // Google Takeout pattern: "photo.jpg" → "photo.jpg.supplemental-metadata.json"
    // or simply:              "photo.jpg" → "photo.json"
    let mut paired = 0usize;
    let mut unpaired_media = Vec::new();
    let sidecar_set: std::collections::HashSet<String> =
        sidecar_files.iter().cloned().collect();

    for media in &media_files {
        let supplemental = format!("{}.supplemental-metadata.json", media);
        let simple_json = format!(
            "{}.json",
            media
                .rsplit_once('.')
                .map(|(base, _)| base)
                .unwrap_or(media)
        );

        if sidecar_set.contains(&supplemental) || sidecar_set.contains(&simple_json) {
            paired += 1;
        } else {
            unpaired_media.push(media.clone());
        }
    }

    // Find sidecars that don't match any media file
    let media_set: std::collections::HashSet<String> =
        media_files.iter().cloned().collect();
    let unpaired_sidecars: Vec<String> = sidecar_files
        .iter()
        .filter(|s| {
            // Strip the sidecar suffix to find the base media path
            let base = s
                .strip_suffix(".supplemental-metadata.json")
                .or_else(|| s.strip_suffix(".json"));
            match base {
                Some(b) => !media_set.contains(b),
                None => true,
            }
        })
        .cloned()
        .collect();

    tracing::info!(
        "Takeout scan: {} media, {} sidecars, {} paired in {:?}",
        media_files.len(),
        sidecar_files.len(),
        paired,
        canonical
    );

    Ok(Json(TakeoutScanResponse {
        directory: canonical.display().to_string(),
        media_files: media_files.len(),
        sidecar_files: sidecar_files.len(),
        paired,
        unpaired_media,
        unpaired_sidecars,
    }))
}

// ── Import entire Takeout directory ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TakeoutImportRequest {
    pub path: String,
}

#[derive(Debug, serde::Serialize)]
pub struct TakeoutImportResponse {
    pub photos_imported: usize,
    pub metadata_imported: usize,
    pub errors: Vec<String>,
}

/// POST /api/admin/import/google-photos
///
/// Import all media files and their paired Google Photos metadata from a Takeout
/// directory. Photos are registered (or uploaded if encrypted mode is active),
/// and metadata JSON sidecars are parsed and stored.
pub async fn import_takeout(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<TakeoutImportRequest>,
) -> Result<Json<TakeoutImportResponse>, AppError> {
    // Admin check
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    if req.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let scan_path = std::path::PathBuf::from(&req.path);
    let canonical = tokio::fs::canonicalize(&scan_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot resolve path '{}': {}", req.path, e))
    })?;

    let encryption_mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    let is_encrypted = encryption_mode == "encrypted";
    let storage_root = state.storage_root.read().await.clone();

    // Collect all files
    let mut media_files: Vec<std::path::PathBuf> = Vec::new();
    let mut queue = vec![canonical.clone()];

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
                    media_files.push(entry.path());
                }
            }
        }
    }

    let mut photos_imported = 0usize;
    let mut metadata_imported = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for media_path in &media_files {
        let filename = media_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let mime = crate::media::mime_from_extension(&filename).to_string();
        let media_type = if mime.starts_with("video/") {
            "video"
        } else if mime == "image/gif" {
            "gif"
        } else {
            "photo"
        };

        let file_meta = tokio::fs::metadata(media_path).await.ok();
        let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
        let modified = file_meta.and_then(|m| {
            m.modified().ok().map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
        });

        // Check for duplicate
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM photos WHERE user_id = ? AND filename = ? AND size_bytes = ? LIMIT 1",
        )
        .bind(&auth.user_id)
        .bind(&filename)
        .bind(size)
        .fetch_optional(&state.pool)
        .await
        .unwrap_or(None);

        let photo_id = if let Some(eid) = existing {
            // Already imported, just use the existing ID for metadata pairing
            eid
        } else {
            // Register the photo in plain mode
            let rel_path = media_path
                .strip_prefix(&storage_root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| {
                    // File is outside storage root; copy it into uploads/
                    String::new()
                });

            let photo_id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();
            let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

            // Try to read taken_at from sidecar if available
            let mut taken_at = modified.clone();
            let mut latitude: Option<f64> = None;
            let mut longitude: Option<f64> = None;

            // Check for sidecar and extract taken_at / geo if present
            let supplemental_path = media_path
                .with_file_name(format!("{}.supplemental-metadata.json", filename));
            if let Ok(sidecar_bytes) = tokio::fs::read(&supplemental_path).await {
                if let Ok(gp) = google_photos::parse_sidecar(&sidecar_bytes) {
                    let record = google_photos::normalise(
                        &gp,
                        String::new(),
                        String::new(),
                        None,
                        None,
                    );
                    if record.taken_at.is_some() {
                        taken_at = record.taken_at.clone();
                    }
                    latitude = record.latitude;
                    longitude = record.longitude;
                }
            }

            if rel_path.is_empty() {
                // File is outside storage root — copy it in
                let uploads_dir = storage_root.join("uploads");
                tokio::fs::create_dir_all(&uploads_dir).await.ok();
                let dest = uploads_dir.join(&filename);
                if let Err(e) = tokio::fs::copy(media_path, &dest).await {
                    errors.push(format!("Failed to copy {}: {}", filename, e));
                    continue;
                }
                let rel = format!("uploads/{}", filename);

                sqlx::query(
                    "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                     size_bytes, width, height, taken_at, latitude, longitude, thumb_path, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?, ?)",
                )
                .bind(&photo_id)
                .bind(&auth.user_id)
                .bind(&filename)
                .bind(&rel)
                .bind(&mime)
                .bind(media_type)
                .bind(size)
                .bind(&taken_at)
                .bind(latitude)
                .bind(longitude)
                .bind(&thumb_rel)
                .bind(&now)
                .execute(&state.pool)
                .await?;
            } else {
                sqlx::query(
                    "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                     size_bytes, width, height, taken_at, latitude, longitude, thumb_path, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?, ?)",
                )
                .bind(&photo_id)
                .bind(&auth.user_id)
                .bind(&filename)
                .bind(&rel_path)
                .bind(&mime)
                .bind(media_type)
                .bind(size)
                .bind(&taken_at)
                .bind(latitude)
                .bind(longitude)
                .bind(&thumb_rel)
                .bind(&now)
                .execute(&state.pool)
                .await?;
            }

            photos_imported += 1;
            photo_id
        };

        // Look for Google Photos sidecar: filename.supplemental-metadata.json
        let supplemental_path = media_path
            .with_file_name(format!("{}.supplemental-metadata.json", filename));

        if let Ok(sidecar_bytes) = tokio::fs::read(&supplemental_path).await {
            match google_photos::parse_sidecar(&sidecar_bytes) {
                Ok(gp_meta) => {
                    let meta_id = Uuid::new_v4().to_string();
                    let record = google_photos::normalise(
                        &gp_meta,
                        meta_id.clone(),
                        auth.user_id.clone(),
                        Some(photo_id.clone()),
                        None,
                    );

                    let storage_path = blob_storage::write_metadata(
                        &storage_root,
                        &auth.user_id,
                        &meta_id,
                        &sidecar_bytes,
                    )
                    .await?;

                    let insert_result = sqlx::query(
                        "INSERT INTO photo_metadata \
                         (id, user_id, photo_id, blob_id, source, title, description, taken_at, \
                          created_at_src, latitude, longitude, altitude, image_views, original_url, \
                          storage_path, is_encrypted, imported_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(&record.id)
                    .bind(&record.user_id)
                    .bind(&record.photo_id)
                    .bind(&record.blob_id)
                    .bind(&record.source)
                    .bind(&record.title)
                    .bind(&record.description)
                    .bind(&record.taken_at)
                    .bind(&record.created_at_src)
                    .bind(record.latitude)
                    .bind(record.longitude)
                    .bind(record.altitude)
                    .bind(record.image_views)
                    .bind(&record.original_url)
                    .bind(&storage_path)
                    .bind(is_encrypted as i32)
                    .bind(&record.imported_at)
                    .execute(&state.pool)
                    .await;

                    match insert_result {
                        Ok(_) => metadata_imported += 1,
                        Err(e) => {
                            errors.push(format!(
                                "Metadata DB insert failed for {}: {}",
                                filename, e
                            ));
                        }
                    }
                }
                Err(e) => {
                    errors.push(format!(
                        "Failed to parse sidecar for {}: {}",
                        filename, e
                    ));
                }
            }
        }
    }

    audit::log(
        &state.pool,
        AuditEvent::BlobUpload,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "import_takeout",
            "photos_imported": photos_imported,
            "metadata_imported": metadata_imported,
            "errors": errors.len(),
        })),
    )
    .await;

    tracing::info!(
        user_id = %auth.user_id,
        photos = photos_imported,
        metadata = metadata_imported,
        errors = errors.len(),
        "Google Photos Takeout import complete"
    );

    Ok(Json(TakeoutImportResponse {
        photos_imported,
        metadata_imported,
        errors,
    }))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Heuristic: is this filename a Google Photos sidecar JSON?
/// Google Takeout uses patterns like:
///   - photo.jpg.supplemental-metadata.json  (newer format)
///   - photo.json                             (older format, same stem as media)
fn is_google_photos_json(name: &str) -> bool {
    if name.ends_with(".supplemental-metadata.json") {
        return true;
    }
    // Check if this looks like a sidecar (basename matches common media extensions)
    if let Some(stem) = name.strip_suffix(".json") {
        // If the stem itself has a media extension, it's likely a sidecar
        is_media_file(stem)
    } else {
        false
    }
}
