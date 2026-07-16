//! Google Photos Takeout directory scanning and bulk import.
//!
//! Google Takeout exports photos into per-album directories with sidecar
//! `.json` files containing metadata (timestamps, geo-location, description).
//!
//! - `GET  /api/admin/import/google-photos/scan`  — recursively scan a
//!   Takeout directory, returning discovered photos with their sidecar metadata.
//! - `POST /api/admin/import/google-photos`       — import discovered photos
//!   into Simple Photos, applying sidecar metadata and generating thumbnails.
//!
//! Both endpoints are admin-only. The scan path is validated against path
//! traversal via `sanitize::validate_relative_path()`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::blobs::storage as blob_storage;
use crate::error::AppError;
use crate::media::is_media_file;
use crate::photos::utils::compute_photo_hash_streaming;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::{google_photos, sidecar};

// ── Scan Google Photos Takeout directory ─────────────────────────────────────

/// Query parameters for scanning a Google Photos Takeout directory for importable media.
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
///
/// Pairing runs through the same shared walk + resolver the import itself uses
/// ([`walk_takeout_tree`] → [`crate::import::sidecar`]). It previously re-
/// implemented the pairing inline as "`NAME.EXT.supplemental-metadata.json` or
/// `NAME.json`", which knows none of the naming rules that actually break Takeout
/// imports — duplicate-counter displacement (`IMG_1(1).JPG` → `IMG_1.JPG(1).json`),
/// length truncation, `-edited` inheritance, case differences — so this report
/// undercounted `paired` and told users their export was worse than it was. Two
/// implementations of one rule is the bug class; there is now one.
pub async fn scan_takeout(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<TakeoutScanQuery>,
) -> Result<Json<TakeoutScanResponse>, AppError> {
    require_admin(&state, &auth).await?;

    if query.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let scan_path = std::path::PathBuf::from(&query.path);
    let canonical = tokio::fs::canonicalize(&scan_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot resolve path '{}': {}", query.path, e))
    })?;

    let walk = walk_takeout_tree(&canonical).await;

    // A sidecar counts as paired once ANY media file resolved to it — an
    // "-edited" copy deliberately shares its original's sidecar, so matching is
    // many-to-one and a per-file count would report the same sidecar unpaired.
    let mut paired = 0usize;
    let mut unpaired_media: Vec<String> = Vec::new();
    let mut matched: HashSet<&Path> = HashSet::new();
    for file in &walk.media {
        match &file.sidecar {
            Some(sp) => {
                paired += 1;
                matched.insert(sp.as_path());
            }
            None => unpaired_media.push(file.path.display().to_string()),
        }
    }

    let unpaired_sidecars: Vec<String> = walk
        .sidecars
        .iter()
        .filter(|s| !matched.contains(s.as_path()))
        .map(|s| s.display().to_string())
        .collect();

    tracing::info!(
        "Takeout scan: {} media, {} sidecars, {} paired in {:?}",
        walk.media.len(),
        walk.sidecars.len(),
        paired,
        canonical
    );

    Ok(Json(TakeoutScanResponse {
        directory: canonical.display().to_string(),
        media_files: walk.media.len(),
        sidecar_files: walk.sidecars.len(),
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
    /// Number of (photo, album) memberships recorded from parent-folder names.
    pub albums_recorded: usize,
    pub errors: Vec<String>,
}

// ── Shared Takeout tree walk ─────────────────────────────────────────────────

/// One media file found in a Takeout tree, with everything its per-directory
/// [`sidecar::TakeoutDirContext`] resolved for it.
struct TakeoutMediaFile {
    path: PathBuf,
    name: String,
    /// The album folder this file lives in, or `None` for date/container folders
    /// and any directory that isn't a Takeout export (the `is_takeout` gate).
    /// This is the album's **identity** — clients key the deterministic album id
    /// off it, so it must stay the mangled-but-stable folder name.
    album: Option<String>,
    /// The album's real title from its `metadata.json`, when it has one. Display
    /// only; `None` falls back to the folder name.
    album_title: Option<String>,
    /// The paired Google sidecar, resolved through the shared naming rules.
    sidecar: Option<PathBuf>,
    /// True when this is an unedited original whose `-edited` sibling sits in the
    /// same folder. The scan/autoscan import keeps the edited copy and drops this
    /// one (#19), so a backfill must expect it to have no photo row of its own.
    shadowed_original: bool,
}

/// Everything one walk of a Takeout tree found.
struct TakeoutWalk {
    media: Vec<TakeoutMediaFile>,
    /// Every per-photo Google sidecar path seen (album-level `metadata.json` and
    /// other stray JSON excluded). Only the scan *report* needs these, to name
    /// the sidecars that pair with nothing.
    sidecars: Vec<PathBuf>,
}

/// Walk a Takeout tree and resolve every media file's sidecar + album folder
/// through [`crate::import::sidecar`] — the single source of truth shared with
/// the filesystem-scan import path. Names in a directory are collected first
/// because a streaming walk can't look ahead to a sidecar that sorts after its
/// media file.
async fn walk_takeout_tree(root: &Path) -> TakeoutWalk {
    let mut out: Vec<TakeoutMediaFile> = Vec::new();
    let mut sidecars: Vec<PathBuf> = Vec::new();
    let mut queue = vec![root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        let mut media_names: Vec<String> = Vec::new();
        let mut json_names: Vec<String> = Vec::new();

        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = ?dir, error = %e, "Skipping unreadable Takeout directory");
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
            } else if ft.is_file() && name.to_lowercase().ends_with(".json") {
                json_names.push(name);
            } else if ft.is_file() && is_media_file(&name) {
                media_names.push(name);
            }
        }

        let shadowed =
            crate::media::edited_shadowed_originals(media_names.iter().map(|s| s.as_str()));
        sidecars.extend(
            json_names
                .iter()
                .filter(|n| sidecar::is_photo_sidecar_name(n))
                .map(|n| dir.join(n)),
        );
        let ctx = sidecar::TakeoutDirContext::new(json_names, &dir);
        let album = ctx.album_name().map(|s| s.to_string());
        // One read per album directory (and none for the rest), not per file.
        let album_title = ctx.resolve_album_title(&dir).await;

        for name in media_names {
            out.push(TakeoutMediaFile {
                path: dir.join(&name),
                sidecar: ctx.resolve_sidecar(&name).map(|j| dir.join(j)),
                shadowed_original: shadowed.contains(&name.to_lowercase()),
                album: album.clone(),
                album_title: album_title.clone(),
                name,
            });
        }
    }

    TakeoutWalk {
        media: out,
        sidecars,
    }
}

/// Find the user's already-imported photo for a Takeout file, using the same
/// dedup keys as the import path: content hash first (catches renamed copies —
/// Takeout stores the same bytes in the date folder AND every album folder),
/// falling back to filename+size when the file couldn't be hashed.
async fn find_existing_photo(
    pool: &SqlitePool,
    user_id: &str,
    filename: &str,
    size: i64,
    photo_hash: Option<&str>,
) -> Option<String> {
    match photo_hash {
        Some(ph) => {
            sqlx::query_scalar("SELECT id FROM photos WHERE user_id = ? AND photo_hash = ? LIMIT 1")
                .bind(user_id)
                .bind(ph)
                .fetch_optional(pool)
                .await
                .unwrap_or(None)
        }
        None => sqlx::query_scalar(
            "SELECT id FROM photos WHERE user_id = ? AND filename = ? AND size_bytes = ? LIMIT 1",
        )
        .bind(user_id)
        .bind(filename)
        .bind(size)
        .fetch_optional(pool)
        .await
        .unwrap_or(None),
    }
}

/// What a [`record_source_album`] call actually changed. Distinguishing these
/// keeps the reported counts honest: a re-run over a fully-recorded library must
/// report zero recovered members, but may legitimately still repair titles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordOutcome {
    /// A new `(photo, album)` membership row.
    Inserted,
    /// The membership already existed; only its display title changed.
    TitleUpdated,
    /// Already recorded, nothing to change.
    Unchanged,
}

/// Record one `(photo, album)` membership captured from a Takeout folder, with
/// the album's real title (`None` → clients fall back to the folder name).
///
/// Idempotent via the `(photo_id, album_name)` primary key, so importing or
/// backfilling twice never duplicates.
///
/// Titles are applied as a second, narrow UPDATE rather than folded into the
/// insert, because `INSERT OR IGNORE` silently drops the whole row on conflict —
/// which would mean every membership recorded before titles existed (i.e. all of
/// them) could never acquire one, and a re-run would look like a no-op while
/// leaving the albums still mis-named. The UPDATE only ever fills in or corrects
/// `album_title`, never touches membership, and no-ops when the title already
/// matches.
///
/// Shared by every writer of `photo_source_albums`: the bulk Takeout import, the
/// filesystem-scan register path, and the album backfill.
pub(crate) async fn record_source_album(
    pool: &SqlitePool,
    user_id: &str,
    photo_id: &str,
    album: &str,
    album_title: Option<&str>,
    now: &str,
) -> Result<RecordOutcome, sqlx::Error> {
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO photo_source_albums \
         (photo_id, user_id, album_name, album_title, source, created_at) \
         VALUES (?, ?, ?, ?, 'google_takeout', ?)",
    )
    .bind(photo_id)
    .bind(user_id)
    .bind(album)
    .bind(album_title)
    .bind(now)
    .execute(pool)
    .await
    .inspect_err(|e| {
        tracing::warn!(
            photo_id = %photo_id,
            album = %album,
            error = %e,
            "Failed to record Takeout source album"
        );
    })?
    .rows_affected()
        > 0;

    if inserted {
        return Ok(RecordOutcome::Inserted);
    }

    // Row already present. Fill in / correct its title, but never blank an
    // existing one back out: a `None` here means "this export didn't tell us the
    // title", not "the album has no title".
    let Some(title) = album_title else {
        return Ok(RecordOutcome::Unchanged);
    };
    let updated = sqlx::query(
        "UPDATE photo_source_albums SET album_title = ? \
         WHERE photo_id = ? AND album_name = ? \
           AND (album_title IS NULL OR album_title <> ?)",
    )
    .bind(title)
    .bind(photo_id)
    .bind(album)
    .bind(title)
    .execute(pool)
    .await
    .inspect_err(|e| {
        tracing::warn!(
            photo_id = %photo_id,
            album = %album,
            error = %e,
            "Failed to update Takeout source album title"
        );
    })?
    .rows_affected()
        > 0;

    Ok(if updated {
        RecordOutcome::TitleUpdated
    } else {
        RecordOutcome::Unchanged
    })
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
    require_admin(&state, &auth).await?;

    if req.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let scan_path = std::path::PathBuf::from(&req.path);
    let canonical = tokio::fs::canonicalize(&scan_path)
        .await
        .map_err(|e| AppError::BadRequest(format!("Cannot resolve path '{}': {}", req.path, e)))?;

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();

    // Walk the tree through the shared resolver (crate::import::sidecar) — the
    // single source of truth also used by the filesystem-scan import path. Each
    // file arrives with its sidecar (supplemental/legacy/truncated/duplicate-
    // counter/-edited naming already handled) and its album folder resolved.
    let media_files = walk_takeout_tree(&canonical).await.media;

    let mut photos_imported = 0usize;
    let mut metadata_imported = 0usize;
    let mut albums_recorded = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for file in &media_files {
        let media_path = &file.path;
        let filename = &file.name;
        let sidecar_path = &file.sidecar;

        let mut mime = crate::media::mime_from_extension(filename).to_string();
        let mut media_type = crate::media::media_type_from_mime(&mime);
        // Content-based GIF rescue (#14): Takeout occasionally exports GIFs under a
        // non-`.gif` name, so the extension-derived classification tags them `photo`
        // and they never reach the GIF smart album. Sniff the leading bytes and fix.
        if let Some(header) = crate::photos::register::read_header_bytes(media_path).await {
            if let Some((m, t)) = crate::media::gif_override(media_type, &header) {
                tracing::info!(file = %filename, "Reclassified Takeout file as GIF from content signature");
                mime = m.to_string();
                media_type = t;
            }
        }

        let file_meta = tokio::fs::metadata(media_path).await.ok();
        let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
        // Normalise the mtime to the canonical `...sssZ` form. This value is used
        // as the `taken_at` fallback below, and gallery ordering is a *string*
        // `ORDER BY COALESCE(taken_at, created_at)` — a bare `to_rfc3339()`
        // ("+00:00", seconds precision) sorts incorrectly against the canonical
        // millis-Z timestamps every other write path produces (issue #13).
        let modified = file_meta.and_then(|m| {
            m.modified().ok().map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                crate::photos::utils::normalize_iso_timestamp(&dt.to_rfc3339())
            })
        });

        // Check for duplicate by content hash (preferred) or filename+size fallback
        let photo_hash = compute_photo_hash_streaming(media_path).await;

        let existing = find_existing_photo(
            &state.pool,
            &auth.user_id,
            filename,
            size,
            photo_hash.as_deref(),
        )
        .await;

        let photo_id = if let Some(eid) = existing {
            // Already imported, just use the existing ID for metadata pairing
            eid
        } else {
            // Register the photo in the photos table
            let rel_path = media_path
                .strip_prefix(&storage_root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| {
                    // File is outside storage root; copy it into uploads/
                    String::new()
                });

            let photo_id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();
            // GIFs keep an animated GIF thumbnail; everything else gets a JPEG.
            let thumb_ext = if media_type == "gif" { "gif" } else { "jpg" };
            let thumb_rel = format!(".thumbnails/{photo_id}.thumb.{thumb_ext}");

            // Try to read taken_at from sidecar if available
            let mut taken_at = modified.clone();
            let mut latitude: Option<f64> = None;
            let mut longitude: Option<f64> = None;

            // Check for sidecar and extract taken_at / geo if present.
            if let Some(ref sp) = sidecar_path {
                if let Ok(sidecar_bytes) = tokio::fs::read(sp).await {
                    if let Ok(gp) = google_photos::parse_sidecar(&sidecar_bytes) {
                        if sidecar::is_photo_sidecar(&gp) {
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
                }
            }

            if rel_path.is_empty() {
                // File is outside storage root — copy it in
                let uploads_dir = storage_root.join("uploads");
                tokio::fs::create_dir_all(&uploads_dir).await.ok();
                let dest = uploads_dir.join(filename);
                if let Err(e) = tokio::fs::copy(media_path, &dest).await {
                    errors.push(format!("Failed to copy {filename}: {e}"));
                    continue;
                }
                let rel = format!("uploads/{filename}");

                sqlx::query(
                    "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                     size_bytes, width, height, taken_at, latitude, longitude, thumb_path, created_at, photo_hash) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&photo_id)
                .bind(&auth.user_id)
                .bind(filename)
                .bind(&rel)
                .bind(&mime)
                .bind(media_type)
                .bind(size)
                .bind(&taken_at)
                .bind(latitude)
                .bind(longitude)
                .bind(&thumb_rel)
                .bind(&now)
                .bind(&photo_hash)
                .execute(&state.pool)
                .await?;
            } else {
                sqlx::query(
                    "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                     size_bytes, width, height, taken_at, latitude, longitude, thumb_path, created_at, photo_hash) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&photo_id)
                .bind(&auth.user_id)
                .bind(filename)
                .bind(&rel_path)
                .bind(&mime)
                .bind(media_type)
                .bind(size)
                .bind(&taken_at)
                .bind(latitude)
                .bind(longitude)
                .bind(&thumb_rel)
                .bind(&now)
                .bind(&photo_hash)
                .execute(&state.pool)
                .await?;
            }

            photos_imported += 1;
            photo_id
        };

        // Capture the album membership authoritatively, keyed by photo_id, from
        // the parent folder. Runs for both freshly-imported and already-existing
        // (deduped) photos so a re-import backfills albums. The walk resolved the
        // album through the per-directory context, so this honours the same
        // `is_takeout` gate as the scan/autoscan paths — a folder with no Google
        // sidecars is a plain user folder and must NOT be turned into a spurious
        // album (part of #11: albums not faithful).
        if let Some(ref album_name) = file.album {
            let now = Utc::now().to_rfc3339();
            match record_source_album(
                &state.pool,
                &auth.user_id,
                &photo_id,
                album_name,
                file.album_title.as_deref(),
                &now,
            )
            .await
            {
                Ok(RecordOutcome::Inserted) => albums_recorded += 1,
                // Already recorded — idempotent no-op (a title repair included).
                Ok(_) => {}
                Err(e) => errors.push(format!("Album record failed for {filename}: {e}")),
            }
        }

        // Store the paired Google Photos sidecar's metadata, if any.
        //
        // Skipped when this photo already has a Takeout metadata row: unlike the
        // photo insert (which dedups on the content hash), this insert had no
        // existence check, so every re-run — and re-running is the whole point of
        // an idempotent import — appended another identical row, silently
        // growing the table and duplicating the info panel's source data.
        let has_metadata = sidecar_path.is_some()
            && sqlx::query_scalar::<_, i64>(
                "SELECT 1 FROM photo_metadata WHERE photo_id = ? AND source = ? LIMIT 1",
            )
            .bind(&photo_id)
            .bind(google_photos::SOURCE)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None)
            .is_some();

        if let (Some(ref sp), false) = (sidecar_path, has_metadata) {
            if let Ok(sidecar_bytes) = tokio::fs::read(sp).await {
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
                          storage_path, imported_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                    .bind(&record.imported_at)
                    .execute(&state.pool)
                    .await;

                        match insert_result {
                            Ok(_) => metadata_imported += 1,
                            Err(e) => {
                                errors
                                    .push(format!("Metadata DB insert failed for {filename}: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("Failed to parse sidecar for {filename}: {e}"));
                    }
                }
            }
        }
    }

    // New photos were inserted — invalidate the cached count summary.
    state.summary_cache.invalidate(&auth.user_id);

    audit::log(
        &state,
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
        albums = albums_recorded,
        errors = errors.len(),
        "Google Photos Takeout import complete"
    );

    Ok(Json(TakeoutImportResponse {
        photos_imported,
        metadata_imported,
        albums_recorded,
        errors,
    }))
}

// ── Backfill album membership for an already-imported library ────────────────

#[derive(Debug, Deserialize)]
pub struct BackfillAlbumsRequest {
    pub path: String,
}

#[derive(Debug, serde::Serialize)]
pub struct BackfillAlbumsResponse {
    pub directory: String,
    /// Distinct Takeout album folders found under `directory`.
    pub albums_seen: usize,
    /// NEW `(photo, album)` membership rows written by this run. Zero on a
    /// re-run — the write is idempotent.
    pub albums_recorded: usize,
    /// Albums whose real title was filled in or corrected from their
    /// `metadata.json` on rows that already existed. Counts distinct *albums*,
    /// not membership rows, so it reads as a number of albums to a human.
    /// Non-zero on the first run after titles shipped (or after a re-export
    /// renamed an album), zero on every run after.
    pub albums_retitled: usize,
    /// Album files matched to a photo already in the library.
    pub photos_matched: usize,
    /// Album files with no photo row at all — never imported, trashed, or moved
    /// to the secure gallery. A backfill cannot recover these.
    pub photos_unmatched: usize,
    /// Unedited originals whose `-edited` sibling was imported instead (#19).
    /// Expected, not a failure — the edited copy carries the membership.
    pub shadowed_skipped: usize,
    /// Sample of failures (capped); `errors_total` is the true count.
    pub errors: Vec<String>,
    pub errors_total: usize,
}

/// Cap on error strings kept for the response, so a systematically failing run
/// over a large library can't build a multi-megabyte JSON body.
const MAX_REPORTED_ERRORS: usize = 100;

#[derive(Debug, Default)]
struct BackfillOutcome {
    albums_seen: usize,
    albums_recorded: usize,
    /// Distinct albums whose title we repaired — a set, because the repair fires
    /// once per membership row and an album has many.
    retitled_albums: HashSet<String>,
    photos_matched: usize,
    photos_unmatched: usize,
    shadowed_skipped: usize,
    errors: Vec<String>,
    errors_total: usize,
}

/// Re-walk a Takeout tree and record album membership for photos that are
/// ALREADY in the library. Writes nothing but `photo_source_albums` rows — no
/// photo and no metadata inserts — so it is cheap and cannot duplicate anything.
///
/// This is the recovery path for libraries imported before album capture
/// existed: `photos/scan.rs` skips any file whose path is already registered,
/// *before* the album-recording code can run, so re-running a scan can never
/// backfill membership. Matching uses the same dedup keys as the import path, so
/// it finds the existing photo whichever physical copy (date folder or album
/// folder) was registered first.
async fn backfill_albums_from_tree(
    pool: &SqlitePool,
    user_id: &str,
    root: &Path,
) -> BackfillOutcome {
    let files = walk_takeout_tree(root).await.media;
    let mut out = BackfillOutcome::default();
    let mut seen_albums: HashSet<String> = HashSet::new();
    let now = Utc::now().to_rfc3339();

    for file in &files {
        // Only files in a genuine Takeout album folder carry membership. Skipping
        // the rest early is also what keeps this cheap: the "Photos from YYYY"
        // date folders hold a copy of the entire library and never need hashing.
        let Some(ref album) = file.album else {
            continue;
        };
        seen_albums.insert(album.clone());

        let size = tokio::fs::metadata(&file.path)
            .await
            .map(|m| m.len() as i64)
            .unwrap_or(0);
        let photo_hash = compute_photo_hash_streaming(&file.path).await;
        let existing =
            find_existing_photo(pool, user_id, &file.name, size, photo_hash.as_deref()).await;

        match existing {
            Some(photo_id) => {
                out.photos_matched += 1;
                match record_source_album(
                    pool,
                    user_id,
                    &photo_id,
                    album,
                    file.album_title.as_deref(),
                    &now,
                )
                .await
                {
                    Ok(RecordOutcome::Inserted) => out.albums_recorded += 1,
                    Ok(RecordOutcome::TitleUpdated) => {
                        out.retitled_albums.insert(album.clone());
                    }
                    Ok(RecordOutcome::Unchanged) => {} // already recorded — no-op
                    Err(e) => {
                        out.errors_total += 1;
                        if out.errors.len() < MAX_REPORTED_ERRORS {
                            out.errors
                                .push(format!("Album record failed for {}: {e}", file.name));
                        }
                    }
                }
            }
            // The unedited original was deliberately dropped at import in favour
            // of its "-edited" sibling, which carries the membership instead.
            None if file.shadowed_original => out.shadowed_skipped += 1,
            None => {
                out.photos_unmatched += 1;
                tracing::debug!(
                    file = %file.name,
                    album = %album,
                    "Album backfill: no imported photo matches this file"
                );
            }
        }
    }

    out.albums_seen = seen_albums.len();
    out
}

/// POST /api/admin/import/google-photos/backfill-albums
///
/// Rebuild `photo_source_albums` for a Takeout directory whose photos are
/// already imported. This exists because album membership was only captured at
/// import time from Jul 2026 onward — every library imported before that has
/// sparse or absent membership, so clients reconstruct partial/empty albums, and
/// no other code path can repair it (the scan skips already-registered paths;
/// the hash-duplicate backfill only fires for a new physical copy at a new path).
///
/// Safe to re-run: it only ever `INSERT OR IGNORE`s membership rows.
pub async fn backfill_takeout_albums(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<BackfillAlbumsRequest>,
) -> Result<Json<BackfillAlbumsResponse>, AppError> {
    require_admin(&state, &auth).await?;

    if req.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let canonical = tokio::fs::canonicalize(PathBuf::from(&req.path))
        .await
        .map_err(|e| AppError::BadRequest(format!("Cannot resolve path '{}': {}", req.path, e)))?;
    let meta = tokio::fs::metadata(&canonical).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot access '{}': {}", canonical.display(), e))
    })?;
    if !meta.is_dir() {
        return Err(AppError::BadRequest(format!(
            "'{}' is not a directory",
            canonical.display()
        )));
    }

    let outcome = backfill_albums_from_tree(&state.pool, &auth.user_id, &canonical).await;

    tracing::info!(
        user_id = %auth.user_id,
        directory = %canonical.display(),
        albums_seen = outcome.albums_seen,
        albums_recorded = outcome.albums_recorded,
        albums_retitled = outcome.retitled_albums.len(),
        matched = outcome.photos_matched,
        unmatched = outcome.photos_unmatched,
        shadowed_skipped = outcome.shadowed_skipped,
        errors = outcome.errors_total,
        "Takeout album backfill complete"
    );

    audit::log(
        &state,
        AuditEvent::PhotoRegister,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "backfill_takeout_albums",
            "directory": canonical.display().to_string(),
            "albums_recorded": outcome.albums_recorded,
            "albums_retitled": outcome.retitled_albums.len(),
            "photos_matched": outcome.photos_matched,
            "errors": outcome.errors_total,
        })),
    )
    .await;

    Ok(Json(BackfillAlbumsResponse {
        directory: canonical.display().to_string(),
        albums_seen: outcome.albums_seen,
        albums_recorded: outcome.albums_recorded,
        albums_retitled: outcome.retitled_albums.len(),
        photos_matched: outcome.photos_matched,
        photos_unmatched: outcome.photos_unmatched,
        shadowed_skipped: outcome.shadowed_skipped,
        errors: outcome.errors,
        errors_total: outcome.errors_total,
    }))
}

// ── List authoritative source albums ─────────────────────────────────────────

/// A single source album with its authoritative photo-id membership.
#[derive(Debug, serde::Serialize)]
pub struct SourceAlbum {
    /// The Takeout folder name. This is the album's **identity** — clients key
    /// the deterministic album id off it, so it is deliberately the mangled name
    /// and must stay stable even when the title changes.
    pub name: String,
    /// The album's real Google Photos title, when the export carried one.
    /// Clients display this in preference to `name`; `null` → use `name`.
    pub title: Option<String>,
    pub source: String,
    pub photo_ids: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct SourceAlbumsResponse {
    pub albums: Vec<SourceAlbum>,
}

/// The client-side album id both platforms derive for a source album:
/// `"src-" + sha256_hex("<source> <album_name>")`.
///
/// Duplicated from `web/src/utils/takeoutAlbums.ts` and Android's
/// `AlbumRepository.recreateAlbumsFromServer` — by design, since the whole point
/// is that all three agree. The server needs it to resolve a `src-…` id back to
/// the album identity it was derived from (see [`dismiss_source_album`]), which
/// is a one-way hash: only recomputation can invert it.
fn source_album_id(source: &str, album_name: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!("{source} {album_name}").as_bytes());
    format!("src-{:x}", hasher.finalize())
}

/// GET /api/photos/source-albums
///
/// Returns the authoritative `album_name → [photo_id]` mapping captured at
/// import time (see [`import_takeout`]). Clients use this to rebuild album
/// manifests deterministically — keyed by photo id, so it survives filename
/// collisions and `-edited` dedup, and works identically on web and Android.
/// This is *not* admin-gated: each user reads only their own albums.
///
/// Albums the user has deleted are filtered out here (see
/// [`dismiss_source_album`]), so *every* client's reconstruction respects the
/// deletion without either of them having to know about tombstones.
pub async fn list_source_albums(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<SourceAlbumsResponse>, AppError> {
    let rows: Vec<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT psa.album_name, psa.album_title, psa.source, psa.photo_id \
         FROM photo_source_albums psa \
         WHERE psa.user_id = ? \
           AND NOT EXISTS ( \
             SELECT 1 FROM dismissed_source_albums d \
             WHERE d.user_id = psa.user_id \
               AND d.source = psa.source \
               AND d.album_name = psa.album_name \
           ) \
         ORDER BY psa.album_name ASC, psa.photo_id ASC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    // Group consecutively by album_name (rows are ordered by album_name).
    let mut albums: Vec<SourceAlbum> = Vec::new();
    for (name, title, source, photo_id) in rows {
        match albums.last_mut() {
            Some(last) if last.name == name => {
                last.photo_ids.push(photo_id);
                // The title is per-membership-row, so a partially-titled album
                // (some rows written before titles shipped, some after) still
                // reports its title rather than whichever row happened to sort
                // first.
                last.title = last.title.take().or(title);
            }
            _ => albums.push(SourceAlbum {
                name,
                title,
                source,
                photo_ids: vec![photo_id],
            }),
        }
    }

    Ok(Json(SourceAlbumsResponse { albums }))
}

// ── Dismiss (tombstone) a reconstructed album ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DismissSourceAlbumRequest {
    /// The client-side album id — `"src-" + sha256(source + " " + album_name)`.
    pub album_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct DismissSourceAlbumResponse {
    /// False when `album_id` matched none of this user's source albums — i.e. it
    /// was an ordinary user-created album, which needs no tombstone.
    pub dismissed: bool,
    /// The album identity that was tombstoned, when one matched.
    pub name: Option<String>,
}

/// POST /api/photos/source-albums/dismiss
///
/// Record that the user deleted a Takeout-reconstructed album, so reconstruction
/// stops recreating it.
///
/// Without this, deleting a reconstructed album was impossible: the delete
/// removed the local album + its manifest blob, and the next reconstruction pass
/// rebuilt it from the same untouched `photo_source_albums` rows — on every
/// device, forever. The user's curation lost to the importer every time.
///
/// The client identifies the album by the id it already has, and the server
/// resolves it back to `(source, album_name)` by recomputing the same hash both
/// clients derive. That keeps the tombstone keyed on the album's real identity
/// (so web and Android agree, and a retitle can't resurrect it) without either
/// client needing to remember the Takeout folder name it was built from.
///
/// Membership is deliberately untouched: the photos stay in the library, which
/// is what "delete album, keep photos" means. Idempotent.
pub async fn dismiss_source_album(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<DismissSourceAlbumRequest>,
) -> Result<Json<DismissSourceAlbumResponse>, AppError> {
    // Only this user's own albums are candidates, so one user can never tombstone
    // another's — and the id space is per-user small (distinct album names).
    let albums: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT source, album_name FROM photo_source_albums WHERE user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    let matched = albums
        .into_iter()
        .find(|(source, name)| source_album_id(source, name) == req.album_id);

    let Some((source, album_name)) = matched else {
        tracing::debug!(
            user_id = %auth.user_id,
            album_id = %req.album_id,
            "Dismiss requested for an id that is not a source album — nothing to tombstone"
        );
        return Ok(Json(DismissSourceAlbumResponse {
            dismissed: false,
            name: None,
        }));
    };

    sqlx::query(
        "INSERT OR IGNORE INTO dismissed_source_albums \
         (user_id, source, album_name, dismissed_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&auth.user_id)
    .bind(&source)
    .bind(&album_name)
    .bind(Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await
    .inspect_err(|e| {
        tracing::error!(
            user_id = %auth.user_id,
            album = %album_name,
            error = %e,
            "Failed to tombstone dismissed source album — it will be recreated"
        );
    })?;

    tracing::info!(
        user_id = %auth.user_id,
        album = %album_name,
        "Tombstoned Takeout source album; reconstruction will skip it"
    );

    Ok(Json(DismissSourceAlbumResponse {
        dismissed: true,
        name: Some(album_name),
    }))
}

// ── Helpers ──────────────────────────────────────────────────────────────────
//
// Album-name derivation, sidecar-name detection, and the definitive per-file
// sidecar resolver all live in [`crate::import::sidecar`], shared with the
// filesystem-scan import path.

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// The scan *report* pairs through this same walk. The old inline matcher
    /// only knew "NAME.EXT.supplemental-metadata.json" / "NAME.json", so every
    /// Takeout naming quirk read as unpaired and the report understated the
    /// export. One walk, one resolver — these are the cases it must now get.
    #[tokio::test]
    async fn walk_pairs_the_naming_rules_the_old_scan_matcher_missed() {
        let root = temp_root("walk-pairing");
        let dir = root.join("Takeout/Google Photos/Trip to Rome");
        // Duplicate counter: the counter moves AFTER the extension on the sidecar.
        write_file(&dir.join("IMG_1(1).jpg"), b"a").await;
        write_file(&dir.join("IMG_1.jpg(1).json"), SIDECAR).await;
        // "-edited" inherits the original's sidecar (it has none of its own).
        write_file(&dir.join("IMG_5.jpg"), b"b").await;
        write_file(&dir.join("IMG_5-edited.jpg"), b"c").await;
        write_file(&dir.join("IMG_5.jpg.supplemental-metadata.json"), SIDECAR).await;
        // Genuinely unpaired, both directions.
        write_file(&dir.join("IMG_9.jpg"), b"d").await;
        write_file(&dir.join("IMG_404.jpg.json"), SIDECAR).await;
        // Album metadata is not a per-photo sidecar and must not be counted.
        write_file(&dir.join("metadata.json"), ALBUM_META).await;

        let walk = walk_takeout_tree(&root).await;

        let paired: Vec<&str> = walk
            .media
            .iter()
            .filter(|f| f.sidecar.is_some())
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(paired.len(), 3, "got {paired:?}");
        for name in ["IMG_1(1).jpg", "IMG_5.jpg", "IMG_5-edited.jpg"] {
            assert!(paired.contains(&name), "{name} must pair; got {paired:?}");
        }

        let unpaired: Vec<&str> = walk
            .media
            .iter()
            .filter(|f| f.sidecar.is_none())
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(unpaired, vec!["IMG_9.jpg"]);

        assert_eq!(
            walk.sidecars.len(),
            3,
            "album metadata.json is not a photo sidecar; got {:?}",
            walk.sidecars
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    // ── Album backfill ───────────────────────────────────────────────────────
    //
    // The backfill is the recovery path for every library imported before album
    // capture existed, so these tests pin the properties that make it safe to run
    // against a live library: it recovers real members, invents nothing, and can
    // be re-run.

    /// A real per-photo sidecar — its presence is what marks a directory as a
    /// Takeout export (the `is_takeout` gate).
    const SIDECAR: &[u8] = br#"{"title":"IMG_1.jpg","photoTakenTime":{"timestamp":"1494963474"}}"#;

    fn temp_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sp-backfill-{tag}-{}", Uuid::new_v4()))
    }

    async fn write_file(path: &Path, bytes: &[u8]) {
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, bytes).await.unwrap();
    }

    /// In-memory DB with the real migrations. FKs off: we insert bare photo rows
    /// without the full users graph.
    async fn test_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// A photo row as a pre-album-capture import left it: content hash present,
    /// no `photo_source_albums` membership anywhere.
    async fn insert_photo(
        pool: &SqlitePool,
        id: &str,
        filename: &str,
        hash: &str,
        encrypted: bool,
    ) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, size_bytes, \
             created_at, photo_hash, encrypted_blob_id) \
             VALUES (?, 'user-1', ?, ?, 'image/jpeg', 0, '2026-01-01T00:00:00.000Z', ?, ?)",
        )
        .bind(id)
        .bind(filename)
        // An encrypted photo's plaintext is gone: file_path is emptied and the
        // bytes live in a blob. Only photo_hash still ties it to the source file.
        .bind(if encrypted { "" } else { "library/IMG_1.jpg" })
        .bind(hash)
        .bind(if encrypted { Some("blob-1") } else { None })
        .execute(pool)
        .await
        .unwrap();
    }

    async fn hash_of(path: &Path) -> String {
        compute_photo_hash_streaming(path).await.unwrap()
    }

    async fn albums_in_db(pool: &SqlitePool) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT photo_id, album_name FROM photo_source_albums ORDER BY photo_id, album_name",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn titles_in_db(pool: &SqlitePool) -> Vec<(String, Option<String>)> {
        sqlx::query_as(
            "SELECT album_name, album_title FROM photo_source_albums ORDER BY album_name",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// The headline case: a photo imported before album capture existed gets its
    /// membership recovered. Takeout stores the SAME bytes in the album folder and
    /// in "Photos from YYYY"; only the album folder may become an album, and the
    /// date-folder copy must not add a second (or duplicate) membership.
    #[tokio::test]
    async fn backfill_recovers_membership_for_already_imported_photo() {
        let root = temp_root("basic");
        let album_dir = root.join("Takeout/Google Photos/Trip to Rome");
        let date_dir = root.join("Takeout/Google Photos/Photos from 2021");
        let bytes = b"colosseum-bytes";
        write_file(&album_dir.join("IMG_1.jpg"), bytes).await;
        write_file(
            &album_dir.join("IMG_1.jpg.supplemental-metadata.json"),
            SIDECAR,
        )
        .await;
        write_file(&date_dir.join("IMG_1.jpg"), bytes).await;
        write_file(
            &date_dir.join("IMG_1.jpg.supplemental-metadata.json"),
            SIDECAR,
        )
        .await;

        let pool = test_pool().await;
        let hash = hash_of(&album_dir.join("IMG_1.jpg")).await;
        insert_photo(&pool, "photo-1", "IMG_1.jpg", &hash, false).await;

        let out = backfill_albums_from_tree(&pool, "user-1", &root).await;

        assert_eq!(out.albums_recorded, 1, "the album member must be recovered");
        assert_eq!(out.photos_matched, 1);
        assert_eq!(out.photos_unmatched, 0);
        assert_eq!(out.albums_seen, 1, "the date folder is not an album");
        assert!(out.errors.is_empty(), "unexpected errors: {:?}", out.errors);
        assert_eq!(
            albums_in_db(&pool).await,
            vec![("photo-1".to_string(), "Trip to Rome".to_string())]
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// Safe to re-run against a live library: the second pass writes nothing new
    /// and never duplicates a membership row.
    #[tokio::test]
    async fn backfill_is_idempotent_on_rerun() {
        let root = temp_root("idempotent");
        let album_dir = root.join("Takeout/Google Photos/Trip to Rome");
        write_file(&album_dir.join("IMG_1.jpg"), b"colosseum-bytes").await;
        write_file(
            &album_dir.join("IMG_1.jpg.supplemental-metadata.json"),
            SIDECAR,
        )
        .await;

        let pool = test_pool().await;
        let hash = hash_of(&album_dir.join("IMG_1.jpg")).await;
        insert_photo(&pool, "photo-1", "IMG_1.jpg", &hash, false).await;

        let first = backfill_albums_from_tree(&pool, "user-1", &root).await;
        assert_eq!(first.albums_recorded, 1);

        let second = backfill_albums_from_tree(&pool, "user-1", &root).await;
        assert_eq!(
            second.albums_recorded, 0,
            "a re-run must record nothing new"
        );
        assert_eq!(
            second.photos_matched, 1,
            "the photo is still matched, just already recorded"
        );
        assert_eq!(
            albums_in_db(&pool).await.len(),
            1,
            "membership must not duplicate"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// The `is_takeout` gate: a plain user folder (media, but no Google sidecars)
    /// must never become an album. Without this the backfill would turn every
    /// directory in someone's library into a bogus album.
    #[tokio::test]
    async fn backfill_skips_non_takeout_folders() {
        let root = temp_root("gate");
        let plain_dir = root.join("Vacation Photos");
        write_file(&plain_dir.join("IMG_1.jpg"), b"holiday-bytes").await;

        let pool = test_pool().await;
        let hash = hash_of(&plain_dir.join("IMG_1.jpg")).await;
        insert_photo(&pool, "photo-1", "IMG_1.jpg", &hash, false).await;

        let out = backfill_albums_from_tree(&pool, "user-1", &root).await;

        assert_eq!(
            out.albums_seen, 0,
            "a folder with no sidecars is not Takeout"
        );
        assert_eq!(out.albums_recorded, 0);
        assert_eq!(
            out.photos_matched, 0,
            "non-album files are never even hashed"
        );
        assert!(albums_in_db(&pool).await.is_empty());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// Matching is by plaintext content hash, which the encryption pass preserves
    /// on the photos row (it only sets encrypted_blob_id). So an already-encrypted
    /// library — i.e. the live one — still backfills.
    #[tokio::test]
    async fn backfill_matches_encrypted_photo_by_content_hash() {
        let root = temp_root("encrypted");
        let album_dir = root.join("Takeout/Google Photos/Trip to Rome");
        write_file(&album_dir.join("IMG_1.jpg"), b"colosseum-bytes").await;
        write_file(
            &album_dir.join("IMG_1.jpg.supplemental-metadata.json"),
            SIDECAR,
        )
        .await;

        let pool = test_pool().await;
        let hash = hash_of(&album_dir.join("IMG_1.jpg")).await;
        insert_photo(&pool, "photo-enc", "IMG_1.jpg", &hash, true).await;

        let out = backfill_albums_from_tree(&pool, "user-1", &root).await;

        assert_eq!(out.photos_matched, 1, "encrypted photos must still match");
        assert_eq!(
            albums_in_db(&pool).await,
            vec![("photo-enc".to_string(), "Trip to Rome".to_string())]
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// Honest reporting on the two ways an album file legitimately has no photo:
    /// its "-edited" sibling was imported instead (#19), or it was never imported
    /// / has been trashed. Neither is an error, but only the latter is a real gap.
    #[tokio::test]
    async fn backfill_separates_shadowed_originals_from_genuine_gaps() {
        let root = temp_root("gaps");
        let album_dir = root.join("Takeout/Google Photos/Trip to Rome");
        // The edited/original pair: import kept "-edited" and dropped the original.
        write_file(&album_dir.join("IMG_5.jpg"), b"original-bytes").await;
        write_file(&album_dir.join("IMG_5-edited.jpg"), b"edited-bytes").await;
        write_file(
            &album_dir.join("IMG_5.jpg.supplemental-metadata.json"),
            SIDECAR,
        )
        .await;
        // Never imported (or trashed): no photo row will match this one.
        write_file(&album_dir.join("IMG_9.jpg"), b"missing-bytes").await;

        let pool = test_pool().await;
        let edited_hash = hash_of(&album_dir.join("IMG_5-edited.jpg")).await;
        insert_photo(
            &pool,
            "photo-edited",
            "IMG_5-edited.jpg",
            &edited_hash,
            false,
        )
        .await;

        let out = backfill_albums_from_tree(&pool, "user-1", &root).await;

        assert_eq!(
            out.photos_matched, 1,
            "the surviving edited copy carries the membership"
        );
        assert_eq!(
            out.shadowed_skipped, 1,
            "the unedited original is expected to have no photo row"
        );
        assert_eq!(
            out.photos_unmatched, 1,
            "the never-imported file is a genuine, reportable gap"
        );
        assert_eq!(
            albums_in_db(&pool).await,
            vec![("photo-edited".to_string(), "Trip to Rome".to_string())]
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    // ── Dismissed-album tombstones ───────────────────────────────────────────

    /// The album id is computed independently by three codebases (this one, the
    /// web client, Android). If they ever disagree the damage is silent: a
    /// tombstone matches nothing and the deleted album comes back. This pins the
    /// formula against a hash computed from the literal spec — `"src-" +
    /// sha256_hex("<source> <album_name>")` — rather than against itself.
    #[test]
    fn source_album_id_matches_the_client_formula() {
        // echo -n "google_takeout Trip to Rome" | sha256sum
        assert_eq!(
            source_album_id("google_takeout", "Trip to Rome"),
            "src-03c6bc29608fa7bffdbdd7b46dab34de74aa131875c032e79ab581a44a29e672"
        );
    }

    #[tokio::test]
    async fn dismissing_an_album_hides_it_from_reconstruction() {
        let pool = test_pool().await;
        insert_photo(&pool, "photo-1", "IMG_1.jpg", "hash-1", false).await;
        record_source_album(&pool, "user-1", "photo-1", "Trip to Rome", None, "now")
            .await
            .unwrap();
        record_source_album(&pool, "user-1", "photo-1", "Birthday", None, "now")
            .await
            .unwrap();

        // The tombstone the dismiss endpoint writes.
        sqlx::query(
            "INSERT INTO dismissed_source_albums (user_id, source, album_name, dismissed_at) \
             VALUES ('user-1', 'google_takeout', 'Trip to Rome', 'now')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // The read `list_source_albums` performs.
        let visible: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT psa.album_name FROM photo_source_albums psa \
             WHERE psa.user_id = 'user-1' \
               AND NOT EXISTS ( \
                 SELECT 1 FROM dismissed_source_albums d \
                 WHERE d.user_id = psa.user_id AND d.source = psa.source \
                   AND d.album_name = psa.album_name) \
             ORDER BY psa.album_name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        assert_eq!(
            visible,
            vec![("Birthday".to_string(),)],
            "a dismissed album must vanish from reconstruction; others must not"
        );

        // Membership itself is untouched — "delete album, keep photos".
        assert_eq!(albums_in_db(&pool).await.len(), 2);
    }

    // ── Real album titles ────────────────────────────────────────────────────

    /// Album-level metadata.json as Google writes it: the real title, and no
    /// photoTakenTime (which is what distinguishes it from a photo sidecar).
    const ALBUM_META: &[u8] = br#"{"title":"Mum & Dad's 40th","access":"protected"}"#;

    /// Takeout mangles the folder name; the true title survives only in the
    /// album's metadata.json. The folder name must remain the identity key (it
    /// derives the album id clients already materialized), with the title
    /// alongside as a display name.
    #[tokio::test]
    async fn backfill_records_the_real_album_title_keyed_by_folder_name() {
        let root = temp_root("title");
        // The name Google actually exports for "Mum & Dad's 40th".
        let album_dir = root.join("Takeout/Google Photos/Mum _ Dad_s 40th");
        write_file(&album_dir.join("IMG_1.jpg"), b"party-bytes").await;
        write_file(
            &album_dir.join("IMG_1.jpg.supplemental-metadata.json"),
            SIDECAR,
        )
        .await;
        write_file(&album_dir.join("metadata.json"), ALBUM_META).await;

        let pool = test_pool().await;
        let hash = hash_of(&album_dir.join("IMG_1.jpg")).await;
        insert_photo(&pool, "photo-1", "IMG_1.jpg", &hash, false).await;

        let out = backfill_albums_from_tree(&pool, "user-1", &root).await;

        assert_eq!(out.albums_recorded, 1);
        assert!(
            out.retitled_albums.is_empty(),
            "a fresh row carries its title already"
        );
        assert_eq!(
            titles_in_db(&pool).await,
            vec![(
                "Mum _ Dad_s 40th".to_string(),
                Some("Mum & Dad's 40th".to_string())
            )],
            "identity stays the folder name; the title rides alongside"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// The live-library case. Phase 1's backfill already wrote membership with no
    /// title (titles didn't exist yet). `INSERT OR IGNORE` alone would drop the
    /// title on the floor forever, leaving every album mis-named with no way to
    /// repair it. Re-running must fill the titles in — and must NOT duplicate or
    /// re-count the membership it already has.
    #[tokio::test]
    async fn backfill_fills_in_titles_for_membership_recorded_before_titles_existed() {
        let root = temp_root("title-repair");
        let album_dir = root.join("Takeout/Google Photos/Mum _ Dad_s 40th");
        write_file(&album_dir.join("IMG_1.jpg"), b"party-bytes").await;
        write_file(
            &album_dir.join("IMG_1.jpg.supplemental-metadata.json"),
            SIDECAR,
        )
        .await;

        let pool = test_pool().await;
        let hash = hash_of(&album_dir.join("IMG_1.jpg")).await;
        insert_photo(&pool, "photo-1", "IMG_1.jpg", &hash, false).await;

        // First run: no metadata.json in the export yet → membership, no title.
        let first = backfill_albums_from_tree(&pool, "user-1", &root).await;
        assert_eq!(first.albums_recorded, 1);
        assert_eq!(
            titles_in_db(&pool).await,
            vec![("Mum _ Dad_s 40th".to_string(), None)]
        );

        // Now the album metadata is available (titles shipped / re-export).
        write_file(&album_dir.join("metadata.json"), ALBUM_META).await;
        let second = backfill_albums_from_tree(&pool, "user-1", &root).await;

        assert_eq!(
            second.albums_recorded, 0,
            "membership already existed — must not be re-counted"
        );
        assert_eq!(
            second.retitled_albums.len(),
            1,
            "the title must be repaired in place"
        );
        assert_eq!(
            titles_in_db(&pool).await,
            vec![(
                "Mum _ Dad_s 40th".to_string(),
                Some("Mum & Dad's 40th".to_string())
            )]
        );
        assert_eq!(
            albums_in_db(&pool).await.len(),
            1,
            "membership must not duplicate"
        );

        // A third run changes nothing at all.
        let third = backfill_albums_from_tree(&pool, "user-1", &root).await;
        assert_eq!(third.albums_recorded, 0);
        assert!(third.retitled_albums.is_empty());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// A `None` title means "this export didn't tell us the title", not "this
    /// album has no title" — so backfilling from an older export that lacks
    /// metadata.json must never blank out a title we already know.
    #[tokio::test]
    async fn record_source_album_never_blanks_a_known_title() {
        let pool = test_pool().await;
        insert_photo(&pool, "photo-1", "IMG_1.jpg", "hash-1", false).await;
        let now = "2026-07-15T00:00:00.000Z";

        let first = record_source_album(
            &pool,
            "user-1",
            "photo-1",
            "Folder",
            Some("Real Title"),
            now,
        )
        .await
        .unwrap();
        assert_eq!(first, RecordOutcome::Inserted);

        let again = record_source_album(&pool, "user-1", "photo-1", "Folder", None, now)
            .await
            .unwrap();
        assert_eq!(again, RecordOutcome::Unchanged);
        assert_eq!(
            titles_in_db(&pool).await,
            vec![("Folder".to_string(), Some("Real Title".to_string()))],
            "an untitled re-import must not erase the known title"
        );
    }
}
