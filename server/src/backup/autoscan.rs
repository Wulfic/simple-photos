//! Automatic filesystem scanner that registers new **native** media files
//! into the database.
//!
//! Runs as a background task on a configurable interval and can also be
//! triggered on-demand via `POST /api/admin/photos/auto-scan`.  Files are
//! assigned to the first admin user; duplicates are handled gracefully with
//! `INSERT OR IGNORE` to avoid race conditions with concurrent scans.
//!
//! Only browser-native formats are handled here.  After native files are
//! imported and encrypted, the ingest engine ([`crate::ingest`]) runs a
//! separate conversion pass for non-native formats.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;
use axum::extract::State;
use axum::Json;
use futures_util::stream::{self, StreamExt};
use futures_util::TryStreamExt;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::state::AppState;

/// Background task: automatically scan the storage directory for new files
/// on a configurable interval (or when triggered by an API call).
///
/// Reads the **current** storage root from `ArcSwap` on every iteration so
/// that runtime storage-path changes (via the setup wizard or admin API) are
/// picked up immediately — no server restart required.
pub async fn background_auto_scan_task(
    pool: sqlx::SqlitePool,
    storage_root: Arc<ArcSwap<PathBuf>>,
    interval_secs: u64,
    scan_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    jwt_secret: String,
    geo_trigger: Arc<tokio::sync::Notify>,
) {
    if interval_secs == 0 {
        tracing::info!("Background auto-scan disabled (interval = 0)");
        return;
    }

    // Run an initial scan shortly after startup
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    tracing::info!("[DIAG:AUTOSCAN] Running startup auto-scan...");
    let root = (**storage_root.load()).clone();
    let count = if let Ok(_guard) = scan_lock.try_lock() {
        run_auto_scan(&pool, &root).await
    } else {
        tracing::info!("[DIAG:AUTOSCAN] Startup scan skipped — another scan is in progress");
        0
    };
    tracing::info!(
        "[DIAG:AUTOSCAN] Startup auto-scan complete: registered {} new files",
        count
    );
    update_last_scan_time(&pool).await;

    if count > 0 {
        crate::audit::log_background(
            &pool,
            crate::audit::AuditEvent::AutoScanComplete,
            Some(serde_json::json!({"trigger": "startup", "new_count": count})),
        );
        // Newly registered files may carry GPS — wake the geo processor now
        // instead of leaving them for its next (≤5-min) poll tick.
        geo_trigger.notify_one();
    }

    // After startup scan, trigger encryption then conversion ingest engine.
    // Sequencing: native encrypt FIRST → conversion → encrypt converted.
    {
        let pool_clone = pool.clone();
        let root_clone = root.clone();
        let jwt_clone = jwt_secret.clone();
        tokio::spawn(async move {
            if count > 0 {
                crate::photos::server_migrate::auto_migrate_after_scan(
                    pool_clone.clone(),
                    root_clone.clone(),
                    jwt_clone.clone(),
                )
                .await;
            }
            crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_clone).await;
        });
    }

    // Then scan on a configurable interval
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    tracing::info!("Auto-scan interval: every {} seconds", interval_secs);

    // How many idle interval ticks (scan found nothing new) to skip before doing
    // a full-tree conversion sweep anyway, so a pure NON-native drop-in (e.g. a
    // HEIC copied straight into the storage folder, which the native scan never
    // registers) is still eventually converted. `run_conversion_pass` walks the
    // ENTIRE tree + queries every known path; doing that every tick on a large
    // HDD library was the "after import the server thrashes the disk with
    // nothing processing" report — an idle conversion pass that always finds
    // zero work but re-walks tens of thousands of files. We keep the walk
    // immediate whenever a scan registered new files (an import is in flight, so
    // there are likely accompanying convertibles), and otherwise throttle the
    // idle sweep to roughly hourly.
    let idle_sweep_every_ticks: u32 = ((3600 / interval_secs.max(1)) as u32).max(1);
    let mut idle_ticks: u32 = 0;

    loop {
        interval.tick().await;
        let root = (**storage_root.load()).clone();
        let count = if let Ok(_guard) = scan_lock.try_lock() {
            run_auto_scan(&pool, &root).await
        } else {
            tracing::info!("[DIAG:AUTOSCAN] Interval scan skipped — another scan is in progress");
            0
        };
        tracing::info!(
            "[DIAG:AUTOSCAN] Interval auto-scan complete: registered {} new files",
            count
        );
        update_last_scan_time(&pool).await;

        if count > 0 {
            crate::audit::log_background(
                &pool,
                crate::audit::AuditEvent::AutoScanComplete,
                Some(serde_json::json!({"trigger": "interval", "new_count": count})),
            );
            // Newly registered files may carry GPS — wake the geo processor now
            // instead of leaving them for its next (≤5-min) poll tick.
            geo_trigger.notify_one();
        }

        // Decide whether this tick should run the (disk-heavy) conversion sweep.
        // New native files → run now (encrypt them + pick up any convertibles
        // that arrived with them). Otherwise only sweep every ~hour so an idle
        // server isn't re-walking the whole library every few minutes.
        let run_conversion = if count > 0 {
            idle_ticks = 0;
            true
        } else {
            idle_ticks += 1;
            if idle_ticks >= idle_sweep_every_ticks {
                idle_ticks = 0;
                true
            } else {
                tracing::debug!(
                    idle_ticks,
                    idle_sweep_every_ticks,
                    "[DIAG:AUTOSCAN] Idle tick — skipping conversion sweep to spare disk I/O"
                );
                false
            }
        };

        // Trigger encryption then conversion ingest engine.
        if run_conversion {
            let pool_clone = pool.clone();
            let root_clone = root.clone();
            let jwt_clone = jwt_secret.clone();
            tokio::spawn(async move {
                if count > 0 {
                    crate::photos::server_migrate::auto_migrate_after_scan(
                        pool_clone.clone(),
                        root_clone.clone(),
                        jwt_clone.clone(),
                    )
                    .await;
                }
                crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_clone).await;
            });
        }
    }
}

async fn update_last_scan_time(pool: &sqlx::SqlitePool) {
    let now = crate::photos::utils::utc_now_iso();
    if let Err(e) = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('last_auto_scan', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&now)
    .execute(pool)
    .await
    {
        tracing::warn!("Failed to update last_auto_scan timestamp: {}", e);
    }
}

/// POST /api/admin/photos/auto-scan
/// Trigger an immediate auto-scan (called when web UI or app opens).
/// Runs synchronously so the client can await completion before loading photos.
/// Admin only — the route is under `/api/admin/`.
pub async fn trigger_auto_scan(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    crate::setup::admin::require_admin(&state, &auth).await?;

    // Serialize with other scan operations (manual scan, background autoscan).
    let _scan_guard = state.scan_lock.lock().await;

    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();

    let count = run_auto_scan(&pool, &storage_root).await;
    tracing::info!(
        "[DIAG:AUTOSCAN] On-demand scan complete: registered {} new files",
        count
    );

    // Update last scan time
    update_last_scan_time(&pool).await;

    crate::audit::log(
        &state,
        crate::audit::AuditEvent::AutoScanComplete,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "trigger": "manual",
            "new_count": count,
        })),
    )
    .await;

    // Trigger encryption then conversion ingest engine.
    {
        let pool_clone = pool.clone();
        let root_clone = storage_root.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            if count > 0 {
                crate::photos::server_migrate::auto_migrate_after_scan(
                    pool_clone.clone(),
                    root_clone.clone(),
                    jwt_secret.clone(),
                )
                .await;
            }
            crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_secret).await;
        });
    }

    Ok(Json(serde_json::json!({
        "message": "Scan complete",
        "new_count": count,
    })))
}

/// Scan storage directory and register any unregistered media files for ALL users.
///
/// Public alias so other modules (e.g. encryption key storage) can trigger a
/// scan without going through the HTTP handler.
pub async fn run_auto_scan_public(pool: &sqlx::SqlitePool, storage_root: &std::path::Path) -> i64 {
    run_auto_scan(pool, storage_root).await
}

/// Scan storage directory and register any unregistered media files for ALL users.
async fn run_auto_scan(pool: &sqlx::SqlitePool, storage_root: &std::path::Path) -> i64 {
    // Skip scanning while a disaster-recovery push is in-flight to avoid
    // creating duplicate photo rows that race with the incoming sync.
    let recovering: bool = sqlx::query_scalar(
        "SELECT value = 'true' FROM server_settings WHERE key = 'recovery_in_progress'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false);
    if recovering {
        tracing::info!("[DIAG:AUTOSCAN] run_auto_scan: recovery in progress, skipping");
        return 0;
    }

    // Get the first admin user to assign new photos to
    let admin_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let admin_id = match admin_id {
        Some(id) => {
            tracing::info!("[DIAG:AUTOSCAN] run_auto_scan: admin_id={}", id);
            id
        }
        None => {
            tracing::info!("[DIAG:AUTOSCAN] run_auto_scan: no admin user yet, skipping");
            return 0;
        }
    };

    // Check audio-backup toggle — skip audio files unless enabled.
    let audio_enabled: bool = crate::photos::utils::audio_backup_enabled(pool).await;

    // Panorama-detection sensitivity for dropped-in files, resolved once per
    // scan (item #7): precise thresholds unless AI categorisation is off.
    let pano_sensitivity =
        crate::photos::metadata::pano_sensitivity_for_user(pool, &admin_id).await;

    // Build set of already-registered paths (from both active photos and trash)
    // using a streaming cursor so we never hold the full Vec<String> + HashSet
    // simultaneously in memory.
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos WHERE file_path != '' \
             UNION SELECT source_path FROM photos WHERE source_path IS NOT NULL AND source_path != '' \
             UNION SELECT file_path FROM trash_items WHERE file_path != '' \
             UNION SELECT original_file_path FROM trash_items WHERE original_file_path IS NOT NULL AND original_file_path != ''"
        ).fetch(pool);

        while let Some(path) = rows.try_next().await.unwrap_or(None) {
            existing_set.insert(path);
        }
    }
    tracing::info!(
        "[DIAG:AUTOSCAN] run_auto_scan: {} existing photos in DB, scanning {:?}",
        existing_set.len(),
        storage_root
    );

    // Build set of content hashes belonging to gallery-hidden originals.
    // After recovery, the photos table doesn't have rows for these (excluded
    // from sync_photos), but their content hashes are stored in the egi table.
    // Any file on disk whose hash matches should NOT be registered — it belongs
    // to a secure gallery item and must stay hidden.
    let mut gallery_hashes = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT original_photo_hash FROM encrypted_gallery_items WHERE original_photo_hash IS NOT NULL"
        ).fetch(pool);

        while let Some(hash) = rows.try_next().await.unwrap_or(None) {
            gallery_hashes.insert(hash);
        }
    }
    if !gallery_hashes.is_empty() {
        tracing::info!(
            "[DIAG:AUTOSCAN] run_auto_scan: {} gallery-hidden hashes to exclude",
            gallery_hashes.len()
        );
    }

    // Probe the storage root up front. A failure here is the difference
    // between "no photos to import" and "the service account can't read the
    // chosen storage path" — the latter is a common Windows footgun (the
    // service runs as LocalSystem, which can't see mapped/SMB/OneDrive paths
    // under a user profile). Without this log the scan silently registers 0
    // files and users assume import is broken. See run_conversion_pass for the
    // matching probe on the conversion side.
    if let Err(e) = tokio::fs::read_dir(storage_root).await {
        tracing::error!(
            path = ?storage_root,
            error = %e,
            "[DIAG:AUTOSCAN] Cannot read storage root — no files will be \
             imported. On Windows the service runs as LocalSystem and cannot \
             read network drives or per-user (OneDrive) folders; point storage \
             at a path the service account can access, or run the service as a \
             user with access."
        );
        return 0;
    }

    // ── Phase 1: walk the tree and collect unregistered native files ──
    // The walk is cheap (directory reads + names/metadata), so it stays
    // sequential; the expensive per-file work (EXIF, full-file hashing, ffmpeg
    // thumbnails) is fanned out with bounded concurrency below. This was
    // previously ONE fully-sequential loop, which throttled a 100GB import to
    // many hours.
    use crate::photos::register::{register_native_file, NativeCandidate, RegisterContext};

    let mut candidates: Vec<NativeCandidate> = Vec::new();
    let mut queue = vec![storage_root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        // Google Photos Takeout dedup (#19): a first, names-only pass over this
        // directory lets us spot "-edited" pairs before registering — a single
        // streaming walk can't look ahead to a sibling that sorts later. Keeps
        // the edited copy, drops the unedited original — the same rule the
        // upload + ingest paths use.
        let shadowed_originals = {
            let mut names_in_dir: Vec<String> = Vec::new();
            if let Ok(mut probe) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(e)) = probe.next_entry().await {
                    let n = e.file_name().to_string_lossy().to_string();
                    if !n.starts_with('.') && is_media_file(&n) {
                        names_in_dir.push(n);
                    }
                }
            }
            crate::media::edited_shadowed_originals(names_in_dir.iter().map(|s| s.as_str()))
        };

        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = ?dir, error = %e, "[DIAG:AUTOSCAN] Skipping unreadable directory");
                continue;
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            let Ok(ft) = entry.file_type().await else {
                continue;
            };
            if ft.is_dir() {
                queue.push(entry.path());
                continue;
            }
            if !(ft.is_file() && is_media_file(&name)) {
                continue;
            }

            // Skip the unedited Google Photos original when its baked-in
            // "-edited" sibling is in this same folder (#19).
            if shadowed_originals.contains(&name.to_lowercase()) {
                tracing::info!(
                    file = %name,
                    "[DIAG:AUTOSCAN] Skipping unedited Google Photos original ('-edited' sibling present)"
                );
                continue;
            }

            let abs_path = entry.path();
            // Normalize to forward slashes so DB paths are consistent across OS.
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
                    crate::photos::utils::normalize_iso_timestamp(&dt.to_rfc3339())
                })
            });

            // Native format — determine MIME and media type directly.
            let mime = mime_from_extension(&name).to_string();
            let media_type: &'static str = if mime.starts_with("video/") {
                "video"
            } else if mime.starts_with("audio/") {
                "audio"
            } else if mime == "image/gif" {
                "gif"
            } else {
                "photo"
            };

            if media_type == "audio" && !audio_enabled {
                continue;
            }

            candidates.push(NativeCandidate {
                abs_path,
                rel_path,
                name,
                mime,
                media_type,
                size,
                modified,
            });
        }
    }

    tracing::info!(
        "[DIAG:AUTOSCAN] run_auto_scan: {} new candidate files to register",
        candidates.len()
    );

    // ── Phase 2: register candidates with bounded concurrency ──
    // Registration is memory-light (header metadata, streaming hash, one INSERT,
    // a subprocess thumbnail); encryption is a separate, memory-budgeted pass —
    // so scaling this with CPU cores speeds a large import up without adding the
    // decode/OOM pressure a naive fan-out would. Shared with the manual `/scan`
    // path via crate::photos::register (single source of truth).
    let ctx = Arc::new(RegisterContext {
        user_id: admin_id,
        pano_sensitivity,
        gallery_hashes: Arc::new(gallery_hashes),
    });
    let new_count = Arc::new(AtomicI64::new(0));

    stream::iter(candidates)
        .map(|cand| {
            let pool = pool.clone();
            let storage_root = storage_root.to_path_buf();
            let ctx = ctx.clone();
            let new_count = new_count.clone();
            async move {
                // Inner spawn keeps multi-core parallelism and isolates a
                // per-file panic from the rest of the pass.
                let _ = tokio::spawn(async move {
                    if register_native_file(&pool, &storage_root, &cand, &ctx).await {
                        new_count.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .await;
            }
        })
        .buffer_unordered(crate::photos::scan::scan_parallelism())
        .for_each(|_| async {})
        .await;

    new_count.load(Ordering::Relaxed)
}
