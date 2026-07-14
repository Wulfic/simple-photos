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

use std::collections::HashMap;
use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
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
    let sidecar_set: std::collections::HashSet<String> = sidecar_files.iter().cloned().collect();

    for media in &media_files {
        let supplemental = format!("{media}.supplemental-metadata.json");
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
    let media_set: std::collections::HashSet<String> = media_files.iter().cloned().collect();
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
    /// Number of (photo, album) memberships recorded from parent-folder names.
    pub albums_recorded: usize,
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

    // Collect all media files, plus a per-directory index of `.json` sidecars so
    // each file resolves its Takeout metadata via the shared resolver
    // (crate::import::sidecar) — the single source of truth also used by the
    // filesystem-scan import path.
    let mut media_files: Vec<PathBuf> = Vec::new();
    let mut dir_json: HashMap<PathBuf, Vec<String>> = HashMap::new();
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
                } else if ft.is_file() && name.to_lowercase().ends_with(".json") {
                    dir_json.entry(dir.clone()).or_default().push(name);
                } else if ft.is_file() && is_media_file(&name) {
                    media_files.push(entry.path());
                }
            }
        }
    }

    // Build the per-directory Takeout contexts once, up front.
    let contexts: HashMap<PathBuf, sidecar::TakeoutDirContext> = dir_json
        .into_iter()
        .map(|(dir, names)| {
            let ctx = sidecar::TakeoutDirContext::new(names, &dir);
            (dir, ctx)
        })
        .collect();

    let mut photos_imported = 0usize;
    let mut metadata_imported = 0usize;
    let mut albums_recorded = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for media_path in &media_files {
        let filename = media_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Resolve the Takeout sidecar once (handles supplemental/legacy/truncated
        // /duplicate-counter/-edited naming) and reuse it for both the taken_at
        // pre-read and the metadata insert below.
        let sidecar_path: Option<PathBuf> = media_path
            .parent()
            .and_then(|d| contexts.get(d))
            .and_then(|ctx| ctx.resolve_sidecar(&filename))
            .map(|j| media_path.with_file_name(j));

        let mut mime = crate::media::mime_from_extension(&filename).to_string();
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

        let existing: Option<String> = if let Some(ref ph) = photo_hash {
            // Content-hash dedup: catches renamed duplicates of the same file
            sqlx::query_scalar("SELECT id FROM photos WHERE user_id = ? AND photo_hash = ? LIMIT 1")
                .bind(&auth.user_id)
                .bind(ph)
                .fetch_optional(&state.pool)
                .await
                .unwrap_or(None)
        } else {
            // Fallback: filename+size if hash couldn't be computed
            sqlx::query_scalar(
                "SELECT id FROM photos WHERE user_id = ? AND filename = ? AND size_bytes = ? LIMIT 1",
            )
            .bind(&auth.user_id)
            .bind(&filename)
            .bind(size)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None)
        };

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
                let dest = uploads_dir.join(&filename);
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
                .bind(&photo_hash)
                .execute(&state.pool)
                .await?;
            }

            photos_imported += 1;
            photo_id
        };

        // Capture the album membership authoritatively, keyed by photo_id, from
        // the parent folder. Runs for both freshly-imported and already-existing
        // (deduped) photos so a re-import backfills albums. Idempotent via the
        // (photo_id, album_name) primary key. Resolved through the per-directory
        // context so it honours the same `is_takeout` gate as the scan/autoscan
        // paths — a folder with no Google sidecars is a plain user folder and must
        // NOT be turned into a spurious album (part of #11: albums not faithful).
        if let Some(album_name) = media_path
            .parent()
            .and_then(|d| contexts.get(d))
            .and_then(|ctx| ctx.album_name())
            .map(|s| s.to_string())
        {
            let now = Utc::now().to_rfc3339();
            match sqlx::query(
                "INSERT OR IGNORE INTO photo_source_albums \
                 (photo_id, user_id, album_name, source, created_at) \
                 VALUES (?, ?, ?, 'google_takeout', ?)",
            )
            .bind(&photo_id)
            .bind(&auth.user_id)
            .bind(&album_name)
            .bind(&now)
            .execute(&state.pool)
            .await
            {
                Ok(res) if res.rows_affected() > 0 => albums_recorded += 1,
                Ok(_) => {} // already recorded — idempotent no-op
                Err(e) => {
                    tracing::warn!(
                        photo_id = %photo_id,
                        album = %album_name,
                        error = %e,
                        "Failed to record Takeout source album"
                    );
                    errors.push(format!("Album record failed for {filename}: {e}"));
                }
            }
        }

        // Store the paired Google Photos sidecar's metadata, if any.
        if let Some(ref sp) = sidecar_path {
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

// ── List authoritative source albums ─────────────────────────────────────────

/// A single source album with its authoritative photo-id membership.
#[derive(Debug, serde::Serialize)]
pub struct SourceAlbum {
    pub name: String,
    pub source: String,
    pub photo_ids: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct SourceAlbumsResponse {
    pub albums: Vec<SourceAlbum>,
}

/// GET /api/photos/source-albums
///
/// Returns the authoritative `album_name → [photo_id]` mapping captured at
/// import time (see [`import_takeout`]). Clients use this to rebuild album
/// manifests deterministically — keyed by photo id, so it survives filename
/// collisions and `-edited` dedup, and works identically on web and Android.
/// This is *not* admin-gated: each user reads only their own albums.
pub async fn list_source_albums(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<SourceAlbumsResponse>, AppError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT album_name, source, photo_id FROM photo_source_albums \
         WHERE user_id = ? ORDER BY album_name ASC, photo_id ASC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    // Group consecutively by album_name (rows are ordered by album_name).
    let mut albums: Vec<SourceAlbum> = Vec::new();
    for (name, source, photo_id) in rows {
        match albums.last_mut() {
            Some(last) if last.name == name => last.photo_ids.push(photo_id),
            _ => albums.push(SourceAlbum {
                name,
                source,
                photo_ids: vec![photo_id],
            }),
        }
    }

    Ok(Json(SourceAlbumsResponse { albums }))
}

// ── Helpers ──────────────────────────────────────────────────────────────────
//
// Album-name derivation and the definitive per-file sidecar resolver live in
// [`crate::import::sidecar`], shared with the filesystem-scan import path.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_photos_json_recognises_sidecar_names() {
        assert!(is_google_photos_json(
            "IMG_1.jpg.supplemental-metadata.json"
        ));
        assert!(is_google_photos_json("IMG_1.jpg.json"));
        assert!(!is_google_photos_json("metadata.json"));
        assert!(!is_google_photos_json("notes.txt"));
    }
}
