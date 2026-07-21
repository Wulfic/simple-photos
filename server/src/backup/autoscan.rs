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
            crate::ingest::run_conversion_pass(
                pool_clone.clone(),
                root_clone.clone(),
                jwt_clone.clone(),
            )
            .await;
            // Ladder rungs last (#49): a secondary rendition must never delay a
            // video becoming playable in the first place.
            crate::transcode::rung_generate::generate_rungs_after_scan(
                pool_clone, root_clone, jwt_clone,
            )
            .await;
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
                crate::ingest::run_conversion_pass(
                    pool_clone.clone(),
                    root_clone.clone(),
                    jwt_clone.clone(),
                )
                .await;
                crate::transcode::rung_generate::generate_rungs_after_scan(
                    pool_clone, root_clone, jwt_clone,
                )
                .await;
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
            crate::ingest::run_conversion_pass(
                pool_clone.clone(),
                root_clone.clone(),
                jwt_secret.clone(),
            )
            .await;
            crate::transcode::rung_generate::generate_rungs_after_scan(
                pool_clone, root_clone, jwt_secret,
            )
            .await;
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
    let scan_start = std::time::Instant::now();
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

    // Load previously scan-rejected paths (Takeout album-copy duplicates +
    // gallery-hidden originals) so the walk skips re-hashing them over the
    // storage mount every pass — the fix for the "server never goes idle after
    // import" disk thrash (migration 031). Keyed by rel_path → (size, mtime); a
    // hit with unchanged size+mtime means "known dead end, don't touch".
    let mut skip_map: std::collections::HashMap<String, crate::photos::scan_skip::SkipRow> =
        std::collections::HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, (String, i64, Option<String>, String, i64)>(
            "SELECT rel_path, size_bytes, mtime, reason, attempt_count \
             FROM scan_skipped_paths WHERE user_id = ?",
        )
        .bind(&admin_id)
        .fetch(pool);
        while let Some((rel_path, size_bytes, mtime, reason, attempt_count)) =
            rows.try_next().await.unwrap_or(None)
        {
            skip_map.insert(
                rel_path,
                crate::photos::scan_skip::SkipRow {
                    size_bytes,
                    mtime,
                    reason,
                    attempt_count,
                },
            );
        }
    }
    tracing::info!(
        "[DIAG:AUTOSCAN] run_auto_scan: {} known scan-skip paths loaded",
        skip_map.len()
    );

    // Per-pass tallies for the single summary line (Phase 3 log hygiene).
    let mut walked: u64 = 0;
    let mut shadowed_count: u64 = 0;
    let mut already_registered: u64 = 0;
    let mut known_skipped: u64 = 0;
    // Skip rows whose file changed on disk (size/mtime differ) — cleared after
    // the walk so the candidate gets a fresh evaluation.
    let mut stale_skip_paths: Vec<String> = Vec::new();

    let mut candidates: Vec<NativeCandidate> = Vec::new();
    let mut queue = vec![storage_root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        // Google Photos Takeout: a first, names-only pass over this directory
        // serves two look-aheads a single streaming walk can't do —
        //   (1) "-edited" dedup (#19): spot the edited/original pair before
        //       registering, keep the edited copy, drop the unedited original;
        //   (2) sidecar + album pairing: index this folder's `.json` sidecars so
        //       each media file can resolve its Takeout metadata (capture date,
        //       GPS, album) below.
        let (shadowed_originals, takeout_ctx) = {
            let mut media_names: Vec<String> = Vec::new();
            let mut json_names: Vec<String> = Vec::new();
            if let Ok(mut probe) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(e)) = probe.next_entry().await {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.starts_with('.') {
                        continue;
                    }
                    if is_media_file(&n) {
                        media_names.push(n);
                    } else if n.to_lowercase().ends_with(".json") {
                        json_names.push(n);
                    }
                }
            }
            let shadowed =
                crate::media::edited_shadowed_originals(media_names.iter().map(|s| s.as_str()));
            let ctx = crate::import::sidecar::TakeoutDirContext::new(json_names, &dir);
            (shadowed, ctx)
        };
        // The album's real title, read once per directory (and only for genuine
        // album directories) rather than per file.
        let album_title = takeout_ctx.resolve_album_title(&dir).await;

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
            walked += 1;

            // Skip the unedited Google Photos original when its baked-in
            // "-edited" sibling is in this same folder (#19).
            if shadowed_originals.contains(&name.to_lowercase()) {
                shadowed_count += 1;
                tracing::debug!(
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
                already_registered += 1;
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

            // Known scan-reject (Takeout album copy / gallery-hidden) with an
            // identical size+mtime — skip the expensive re-hash entirely. This is
            // the change that stops the 4,254-file re-hash loop at idle. If either
            // size or mtime differs the file was replaced/edited, so we clear the
            // stale row and fall through to re-evaluate it.
            // The comparison moved into `photos::scan_skip::skip_verdict` when
            // #40 added a reason whose verdict depends on an attempt count. This
            // walk only ever writes terminal verdicts, so `Retry` is unreachable
            // here today — but it is handled rather than lumped in with `Skip`,
            // because the conversion walk does produce it and silently treating
            // a retryable row as terminal is precisely the one-strike bug the
            // shared function exists to prevent.
            if let Some(row) = skip_map.get(&rel_path) {
                match crate::photos::scan_skip::skip_verdict(row, size, modified.as_deref()) {
                    crate::photos::scan_skip::SkipVerdict::Skip => {
                        known_skipped += 1;
                        continue;
                    }
                    crate::photos::scan_skip::SkipVerdict::Stale => {
                        stale_skip_paths.push(rel_path.clone());
                    }
                    crate::photos::scan_skip::SkipVerdict::Retry => {}
                }
            }

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

            let sidecar_abs = takeout_ctx.resolve_sidecar(&name).map(|j| dir.join(j));
            let album_name = takeout_ctx.album_name().map(|s| s.to_string());

            candidates.push(NativeCandidate {
                abs_path,
                rel_path,
                name,
                mime,
                media_type,
                size,
                modified,
                sidecar_abs,
                album_name,
                album_title: album_title.clone(),
            });
        }
    }

    tracing::debug!(
        "[DIAG:AUTOSCAN] run_auto_scan: {} new candidate files to register",
        candidates.len()
    );

    // Clear skip rows whose file changed on disk since we last rejected it, so
    // the fresh evaluation below isn't shadowed by a stale "already a dup" row.
    // Rare (only genuinely-changed files land here), so a per-path delete is
    // fine; if the file is still a duplicate, register re-records the row.
    for stale in &stale_skip_paths {
        if let Err(e) = sqlx::query(
            "DELETE FROM scan_skipped_paths WHERE user_id = ? AND rel_path = ?",
        )
        .bind(&admin_id)
        .bind(stale)
        .execute(pool)
        .await
        {
            tracing::warn!(rel_path = %stale, error = %e, "Failed to clear stale scan-skip row");
        }
    }

    // ── Phase 2: register candidates with bounded concurrency ──
    // Registration is memory-light (header metadata, streaming hash, one INSERT,
    // a subprocess thumbnail); encryption is a separate, memory-budgeted pass —
    // so scaling this with CPU cores speeds a large import up without adding the
    // decode/OOM pressure a naive fan-out would. Shared with the manual `/scan`
    // path via crate::photos::register (single source of truth).
    let candidates_len = candidates.len();
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

    let registered = new_count.load(Ordering::Relaxed);
    // One summary line per pass instead of thousands of per-file INFO lines. At
    // idle steady-state this reports 0 candidates / 0 registered and finishes in
    // well under a second — the signal that the disk-thrash loop is dead.
    tracing::info!(
        "[DIAG:AUTOSCAN] scan pass: {} media walked, {} shadowed, {} already-registered, \
         {} known-dups skipped, {} stale-rechecked, {} candidates, {} registered, took {:.1}s",
        walked,
        shadowed_count,
        already_registered,
        known_skipped,
        stale_skip_paths.len(),
        candidates_len,
        registered,
        scan_start.elapsed().as_secs_f64(),
    );
    registered
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// In-memory DB with all migrations + one admin user (the scan assigns new
    /// photos to the first admin). `max_connections(1)` keeps the single
    /// in-memory database alive across the whole test.
    async fn test_pool() -> sqlx::SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, role) \
             VALUES ('admin-1', 'admin', 'x', '2020-01-01T00:00:00.000Z', 'admin')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    /// THE disk-thrash fix, end to end: a Takeout date-folder photo and its
    /// identical album-folder copy. The first scan registers one photo and
    /// remembers the copy; the second scan must find and skip the copy WITHOUT
    /// re-hashing it — proven by a sentinel on the skip row that a re-run of
    /// `register` (which `INSERT OR REPLACE`s the row) would have overwritten.
    #[tokio::test]
    async fn scan_remembers_dup_and_skips_it_next_pass() {
        let pool = test_pool().await;
        let root = std::env::temp_dir().join(format!("sp-autoscan-{}", uuid::Uuid::new_v4()));
        let year = root.join("Photos from 2020");
        let album = root.join("Cats");
        tokio::fs::create_dir_all(&year).await.unwrap();
        tokio::fs::create_dir_all(&album).await.unwrap();
        let bytes = b"identical-cat-bytes-in-date-and-album-folders";
        tokio::fs::write(year.join("cat.jpg"), bytes).await.unwrap();
        tokio::fs::write(album.join("cat.jpg"), bytes).await.unwrap();

        let first = run_auto_scan(&pool, &root).await;
        assert_eq!(first, 1, "identical bytes dedup to a single new photo");

        let (photos,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(photos, 1);

        let (skips,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM scan_skipped_paths")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(skips, 1, "the duplicate copy is remembered exactly once");
        let reason: String = sqlx::query_scalar("SELECT reason FROM scan_skipped_paths")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(reason, "hash_duplicate");

        // If the next pass re-hashes the copy, `register` runs again and its
        // INSERT OR REPLACE overwrites this sentinel. Survival == it was skipped.
        sqlx::query("UPDATE scan_skipped_paths SET created_at = 'SENTINEL'")
            .execute(&pool)
            .await
            .unwrap();

        let second = run_auto_scan(&pool, &root).await;
        assert_eq!(second, 0, "steady state registers nothing new");

        let (survived,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM scan_skipped_paths WHERE created_at = 'SENTINEL'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            survived, 1,
            "the copy was skipped without re-hashing (register never re-ran)"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// A skip row is a cache keyed by size+mtime: if the file on disk changes, it
    /// must be re-evaluated, never wrongly suppressed. Change the remembered copy
    /// to unique content and it must register as a new photo.
    #[tokio::test]
    async fn changed_file_is_reevaluated_despite_stale_skip() {
        let pool = test_pool().await;
        let root = std::env::temp_dir().join(format!("sp-autoscan-{}", uuid::Uuid::new_v4()));
        let year = root.join("Photos from 2020");
        let album = root.join("Cats");
        tokio::fs::create_dir_all(&year).await.unwrap();
        tokio::fs::create_dir_all(&album).await.unwrap();
        let bytes = b"identical-cat-bytes-A";
        tokio::fs::write(year.join("cat.jpg"), bytes).await.unwrap();
        tokio::fs::write(album.join("cat.jpg"), bytes).await.unwrap();

        assert_eq!(run_auto_scan(&pool, &root).await, 1);

        // Change whichever copy got the skip row (registration order is racy).
        let skipped_rel: String = sqlx::query_scalar("SELECT rel_path FROM scan_skipped_paths")
            .fetch_one(&pool)
            .await
            .unwrap();
        tokio::fs::write(
            root.join(&skipped_rel),
            b"now-a-totally-different-and-much-longer-image-payload",
        )
        .await
        .unwrap();

        // Size differs from the remembered row → stale → re-evaluated → unique
        // content now → registers.
        assert_eq!(
            run_auto_scan(&pool, &root).await,
            1,
            "a changed file must not stay wrongly skipped"
        );

        let (photos,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(photos, 2);

        // Stale row cleared; new content is unique so no fresh dup skip.
        let (skips,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM scan_skipped_paths")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(skips, 0);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// Invalidation (migration 031 trigger): deleting the photo a copy deduped
    /// against clears the copy's skip row, so the copy can register again — the
    /// skip cache must never change observable scan behaviour.
    #[tokio::test]
    async fn deleting_the_deduped_photo_clears_the_skip_row() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, created_at, photo_hash) \
             VALUES ('p1','admin-1','cat.jpg','Photos from 2020/cat.jpg','image/jpeg','2020-01-01T00:00:00.000Z','HASH-1')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO scan_skipped_paths \
             (user_id, rel_path, size_bytes, mtime, reason, photo_hash, created_at) \
             VALUES ('admin-1','Cats/cat.jpg',10,'2020-01-01T00:00:00.000Z','hash_duplicate','HASH-1','2020-01-01T00:00:00.000Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM photos WHERE id = 'p1'")
            .execute(&pool)
            .await
            .unwrap();

        let (skips,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM scan_skipped_paths")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(skips, 0, "the photo-delete trigger cleared the copy's skip row");
    }

    /// The gallery-hidden analogue: removing a secure-gallery item un-hides its
    /// content hash, so a matching file on disk must be allowed back next scan.
    /// (Bare rows, FKs off — the trigger doesn't care about the parent graph.)
    #[tokio::test]
    async fn removing_a_gallery_item_clears_the_skip_row() {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at, original_photo_hash) \
             VALUES ('egi-1','g-1','b-1','2020-01-01T00:00:00.000Z','HASH-9')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO scan_skipped_paths \
             (user_id, rel_path, size_bytes, mtime, reason, photo_hash, created_at) \
             VALUES ('admin-1','Secret/x.jpg',10,'t','gallery_hidden','HASH-9','t')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM encrypted_gallery_items WHERE id = 'egi-1'")
            .execute(&pool)
            .await
            .unwrap();

        let (skips,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM scan_skipped_paths")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(skips, 0, "the egi-delete trigger un-hides the file for re-scan");
    }
}
