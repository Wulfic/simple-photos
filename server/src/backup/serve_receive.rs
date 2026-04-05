//! Backup receive endpoint — accepts files pushed from the primary server.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use percent_encoding::percent_decode_str;
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::photos::thumbnail::generate_thumbnail_file;
use crate::sanitize;
use crate::state::AppState;

use super::serve::validate_api_key;

// ── Backup Receive Endpoint ──────────────────────────────────────────────────

/// POST /api/backup/receive
/// Receives a file pushed from a primary server during sync.
/// Preserves all metadata (timestamps, GPS, dimensions, tags, user association)
/// and generates a thumbnail immediately so the backup is a 1:1 replica.
///
/// Required headers: X-API-Key, X-Photo-Id, X-File-Path, X-Source, X-Content-Hash
/// Metadata headers (all optional, sent by updated primary sync engine):
///   X-User-Id, X-Original-Created-At, X-Taken-At, X-Width, X-Height,
///   X-Latitude, X-Longitude, X-Duration-Secs, X-Camera-Model, X-Is-Favorite,
///   X-Photo-Hash, X-Crop-Metadata, X-Tags,
///   X-Deleted-At, X-Expires-At (trash only)
pub async fn backup_receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_api_key(&state, &headers).await?;

    // Extract required headers
    let photo_id = headers
        .get("X-Photo-Id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-Photo-Id header".into()))?
        .to_string();

    let raw_file_path = headers
        .get("X-File-Path")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-File-Path header".into()))?;

    // Percent-decode the path (the sync sender encodes non-ASCII chars)
    let file_path = percent_decode_str(raw_file_path)
        .decode_utf8()
        .map_err(|e| AppError::BadRequest(format!("Invalid UTF-8 in X-File-Path: {}", e)))?
        .to_string();

    // Security: validate the file_path is a safe relative path (no traversal, no absolute)
    sanitize::validate_relative_path(&file_path)
        .map_err(|reason| AppError::BadRequest(format!("Invalid X-File-Path: {}", reason)))?;

    let source = headers
        .get("X-Source")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("photos")
        .to_string();

    // Verify checksum if the sender provided one (X-Content-Hash: hex SHA-256)
    if let Some(expected_hash) = headers.get("X-Content-Hash").and_then(|v| v.to_str().ok()) {
        let actual_hash = hex::encode(Sha256::digest(&body));
        if !actual_hash.eq_ignore_ascii_case(expected_hash) {
            return Err(AppError::BadRequest(format!(
                "Content hash mismatch: expected {}, got {}",
                expected_hash, actual_hash
            )));
        }
    }

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&file_path);

    // Defense-in-depth: verify the resolved path is still within storage_root
    let canonical_root = storage_root
        .canonicalize()
        .unwrap_or_else(|_| storage_root.clone());
    // We can't canonicalize full_path yet (file doesn't exist), so check the parent
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create directories: {}", e)))?;
        let canonical_parent = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        if !canonical_parent.starts_with(&canonical_root) {
            return Err(AppError::BadRequest(
                "File path escapes storage root".into(),
            ));
        }
    }

    // Write the file to disk
    let size_bytes = body.len() as i64;
    tokio::fs::write(&full_path, &body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {}", e)))?;

    // ── Parse metadata headers ──────────────────────────────────────────────

    // Helpers to parse percent-encoded optional string / numeric headers
    fn hdr_str(headers: &HeaderMap, name: &str) -> Option<String> {
        headers.get(name).and_then(|v| v.to_str().ok()).map(|s| {
            percent_decode_str(s)
                .decode_utf8()
                .map(|d| d.to_string())
                .unwrap_or_else(|_| s.to_string())
        })
    }
    fn hdr_f64(headers: &HeaderMap, name: &str) -> Option<f64> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<f64>().ok())
    }
    fn hdr_i64(headers: &HeaderMap, name: &str) -> i64 {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0)
    }

    // Use the primary's user_id when the corresponding user exists on the backup;
    // otherwise fall back to the local admin so foreign-key constraints are met.
    let primary_user_id = hdr_str(&headers, "X-User-Id");
    let owner_id: String = if let Some(ref uid) = primary_user_id {
        let exists: bool = sqlx::query_scalar("SELECT COUNT(*) > 0 FROM users WHERE id = ?")
            .bind(uid)
            .fetch_one(&state.read_pool)
            .await
            .unwrap_or(false);
        if exists {
            uid.clone()
        } else {
            sqlx::query_scalar(
                "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
            )
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or_else(|| AppError::Internal("No admin user on backup server".into()))?
        }
    } else {
        sqlx::query_scalar(
            "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
        )
        .fetch_optional(&state.read_pool)
        .await?
        .ok_or_else(|| AppError::Internal("No admin user on backup server".into()))?
    };

    // Derive filename, mime_type, and media_type from headers (if provided by primary)
    // or fall back to deriving from the file path.
    let filename = hdr_str(&headers, "X-Filename").unwrap_or_else(|| {
        std::path::Path::new(&file_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| file_path.clone())
    });

    let mime_type = hdr_str(&headers, "X-Mime-Type")
        .unwrap_or_else(|| crate::media::mime_from_extension(&filename).to_string());

    let media_type = hdr_str(&headers, "X-Media-Type").unwrap_or_else(|| {
        if mime_type.starts_with("video/") {
            "video".to_string()
        } else if mime_type.starts_with("audio/") {
            "audio".to_string()
        } else if mime_type == "image/gif" {
            "gif".to_string()
        } else {
            "photo".to_string()
        }
    });

    // Preserve original timestamps; fall back to now only when absent
    let now = Utc::now().to_rfc3339();
    let created_at = hdr_str(&headers, "X-Original-Created-At").unwrap_or_else(|| now.clone());
    let taken_at = hdr_str(&headers, "X-Taken-At");
    let deleted_at = hdr_str(&headers, "X-Deleted-At").unwrap_or_else(|| now.clone());
    let expires_at = hdr_str(&headers, "X-Expires-At").unwrap_or_else(|| now.clone());

    let width = hdr_i64(&headers, "X-Width");
    let height = hdr_i64(&headers, "X-Height");
    let latitude = hdr_f64(&headers, "X-Latitude");
    let longitude = hdr_f64(&headers, "X-Longitude");
    let duration = hdr_f64(&headers, "X-Duration-Secs");
    let camera_model = hdr_str(&headers, "X-Camera-Model");
    let is_favorite = headers
        .get("X-Is-Favorite")
        .and_then(|v| v.to_str().ok())
        .map(|s| s == "1")
        .unwrap_or(false);
    let photo_hash = hdr_str(&headers, "X-Photo-Hash");
    let crop_metadata = hdr_str(&headers, "X-Crop-Metadata");

    // Tags: comma-separated, each tag percent-encoded
    let tags: Vec<String> = headers
        .get("X-Tags")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .filter(|t| !t.is_empty())
                .map(|t| {
                    percent_decode_str(t)
                        .decode_utf8()
                        .map(|d| d.to_string())
                        .unwrap_or_else(|_| t.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    // Thumbnail path — same convention as autoscan so serving code stays identical
    let thumb_ext = if mime_type == "image/gif" {
        "gif"
    } else {
        "jpg"
    };
    let thumb_rel = format!(".thumbnails/{}.thumb.{}", photo_id, thumb_ext);

    if source == "trash" {
        // ── Upsert into trash_items (full metadata) ───────────────────────────
        // Remove conflicting row with same file_path but different id (can
        // happen when auto-scan raced ahead of recovery sync).
        let _ = sqlx::query("DELETE FROM trash_items WHERE file_path = ? AND id != ?")
            .bind(&file_path)
            .bind(&photo_id)
            .execute(&state.pool)
            .await;

        sqlx::query(
            "INSERT INTO trash_items (
                id, user_id, photo_id, filename, file_path, mime_type, media_type,
                size_bytes, width, height, taken_at, latitude, longitude,
                duration_secs, camera_model, is_favorite, photo_hash, crop_metadata,
                thumb_path, deleted_at, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                file_path     = excluded.file_path,
                size_bytes    = excluded.size_bytes,
                taken_at      = COALESCE(excluded.taken_at,     taken_at),
                latitude      = COALESCE(excluded.latitude,     latitude),
                longitude     = COALESCE(excluded.longitude,    longitude),
                width         = CASE WHEN excluded.width  > 0 THEN excluded.width  ELSE width  END,
                height        = CASE WHEN excluded.height > 0 THEN excluded.height ELSE height END,
                duration_secs = COALESCE(excluded.duration_secs,  duration_secs),
                camera_model  = COALESCE(excluded.camera_model,   camera_model),
                is_favorite   = excluded.is_favorite,
                photo_hash    = COALESCE(excluded.photo_hash,     photo_hash),
                crop_metadata = COALESCE(excluded.crop_metadata,  crop_metadata),
                thumb_path    = COALESCE(excluded.thumb_path,     thumb_path),
                deleted_at    = excluded.deleted_at,
                expires_at    = excluded.expires_at",
        )
        .bind(&photo_id)
        .bind(&owner_id)
        .bind(&photo_id)
        .bind(&filename)
        .bind(&file_path)
        .bind(&mime_type)
        .bind(&media_type)
        .bind(size_bytes)
        .bind(width)
        .bind(height)
        .bind(&taken_at)
        .bind(latitude)
        .bind(longitude)
        .bind(duration)
        .bind(&camera_model)
        .bind(is_favorite)
        .bind(&photo_hash)
        .bind(&crop_metadata)
        .bind(&thumb_rel)
        .bind(&deleted_at)
        .bind(&expires_at)
        .execute(&state.pool)
        .await?;

        // Remove from the main photos table if it was previously synced as an
        // active photo — the item has been deleted on the primary and must not
        // appear in the gallery on the backup either.
        //
        // X-Original-Photo-Id contains the original photo UUID (photos.id on
        // the backup).  X-Photo-Id is the *trash row* UUID, which is a
        // different value — deleting by it would be a no-op.
        let gallery_id = hdr_str(&headers, "X-Original-Photo-Id")
            .unwrap_or_else(|| photo_id.clone());

        // Before deleting the gallery row, look up its thumb_path so we can
        // copy the existing thumbnail to the trash thumbnail path later.
        // This handles encrypted items where thumbnail generation from the
        // encrypted file will fail.
        let existing_thumb_path: Option<String> = sqlx::query_scalar(
            "SELECT thumb_path FROM photos WHERE id = ? OR encrypted_blob_id = ? OR file_path = ? LIMIT 1",
        )
        .bind(&gallery_id)
        .bind(&gallery_id)
        .bind(&file_path)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();

        // Delete by UUID, encrypted_blob_id, and file_path.  The UUID covers
        // the normal case; encrypted_blob_id covers encrypted items (photo_id
        // = blob_id); file_path covers a race where autoscan ran between
        // Phase-0a and this receive call and re-imported with a different UUID.
        if let Err(e) = sqlx::query(
            "DELETE FROM photos WHERE id = ? OR encrypted_blob_id = ? OR file_path = ?",
        )
        .bind(&gallery_id)
        .bind(&gallery_id)
        .bind(&file_path)
        .execute(&state.pool)
        .await
        {
            tracing::warn!(
                photo_id = %photo_id,
                gallery_id = %gallery_id,
                "Failed to remove photo from gallery after receiving as trash: {}",
                e
            );
        }
        // Clean up any dangling tags for the removed photo row.
        let _ = sqlx::query("DELETE FROM photo_tags WHERE photo_id = ?")
            .bind(&gallery_id)
            .execute(&state.pool)
            .await;

        // ── Trash thumbnail fallback ────────────────────────────────────
        // After the standard thumbnail generation (which may fail for
        // encrypted blobs), copy the original photo's thumbnail if it
        // exists.  This ensures trash thumbnails render on backup servers
        // where the encrypted file content cannot be decoded.
        {
            let thumb_abs = storage_root.join(&thumb_rel);
            let generated = generate_thumbnail_file(&full_path, &thumb_abs, &mime_type, None).await;
            if generated {
                tracing::debug!(photo_id = %photo_id, "Generated trash thumbnail on receive");
            } else if let Some(ref orig_thumb) = existing_thumb_path {
                let orig_abs = storage_root.join(orig_thumb);
                if tokio::fs::try_exists(&orig_abs).await.unwrap_or(false) {
                    if let Err(e) = tokio::fs::copy(&orig_abs, &thumb_abs).await {
                        tracing::warn!(
                            photo_id = %photo_id,
                            "Failed to copy original thumbnail for trash: {}", e
                        );
                    } else {
                        tracing::debug!(
                            photo_id = %photo_id,
                            "Copied original thumbnail for trash item"
                        );
                    }
                }
            }
        }
    } else {
        // ── Upsert into photos (full metadata) ───────────────────────────────
        // During recovery, auto-scan may have already registered the file from
        // disk with a different UUID. Remove the conflicting row so the upsert
        // on id can proceed.
        //
        // Only remove rows that are autoscan artifacts (photo_hash IS NOT NULL)
        // when the incoming photo also has a hash.  Photo copies created by
        // the "Save Copy" feature intentionally share a file_path with the
        // original and have photo_hash = NULL — they must not be deleted.
        if photo_hash.is_some() {
            let _ = sqlx::query(
                "DELETE FROM photos WHERE file_path = ? AND id != ? AND photo_hash IS NOT NULL",
            )
            .bind(&file_path)
            .bind(&photo_id)
            .execute(&state.pool)
            .await;
        }

        sqlx::query(
            "INSERT INTO photos (
                id, user_id, filename, file_path, mime_type, media_type,
                size_bytes, width, height, taken_at, latitude, longitude,
                duration_secs, camera_model, is_favorite, photo_hash, crop_metadata,
                thumb_path, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                file_path     = excluded.file_path,
                size_bytes    = excluded.size_bytes,
                taken_at      = COALESCE(excluded.taken_at,    taken_at),
                latitude      = COALESCE(excluded.latitude,    latitude),
                longitude     = COALESCE(excluded.longitude,   longitude),
                width         = CASE WHEN excluded.width  > 0 THEN excluded.width  ELSE width  END,
                height        = CASE WHEN excluded.height > 0 THEN excluded.height ELSE height END,
                duration_secs = COALESCE(excluded.duration_secs,  duration_secs),
                camera_model  = COALESCE(excluded.camera_model,   camera_model),
                is_favorite   = excluded.is_favorite,
                photo_hash    = COALESCE(excluded.photo_hash,     photo_hash),
                crop_metadata = COALESCE(excluded.crop_metadata,  crop_metadata),
                thumb_path    = COALESCE(excluded.thumb_path,     thumb_path)",
        )
        .bind(&photo_id)
        .bind(&owner_id)
        .bind(&filename)
        .bind(&file_path)
        .bind(&mime_type)
        .bind(&media_type)
        .bind(size_bytes)
        .bind(width)
        .bind(height)
        .bind(&taken_at)
        .bind(latitude)
        .bind(longitude)
        .bind(duration)
        .bind(&camera_model)
        .bind(is_favorite)
        .bind(&photo_hash)
        .bind(&crop_metadata)
        .bind(&thumb_rel)
        .bind(&created_at)
        .execute(&state.pool)
        .await?;

        // Replicate photo tags from the primary
        for tag in &tags {
            if let Err(e) = sqlx::query(
                "INSERT OR IGNORE INTO photo_tags (photo_id, user_id, tag, created_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&photo_id)
            .bind(&owner_id)
            .bind(tag)
            .bind(&created_at)
            .execute(&state.pool)
            .await
            {
                tracing::warn!(
                    photo_id = %photo_id,
                    tag = %tag,
                    "Failed to replicate photo tag during backup: {}",
                    e
                );
            }
        }

        // If this item previously existed in trash on the backup (e.g. it was
        // synced as deleted and has since been restored on the primary), remove
        // the stale trash entry so it only appears in the gallery.
        if let Err(e) = sqlx::query("DELETE FROM trash_items WHERE id = ?")
            .bind(&photo_id)
            .execute(&state.pool)
            .await
        {
            tracing::warn!(
                photo_id = %photo_id,
                "Failed to remove trash entry after receiving as active photo: {}",
                e
            );
        }
    }

    // ── Generate thumbnail immediately (photos only) ───────────────────────
    // Don't wait for the background autoscan pass — generate now so the
    // backup serves thumbnails right after the sync completes.
    // Audio files get a solid-black placeholder, matching the primary.
    // Trash items handle their own thumbnail logic above (with fallback to
    // the original photo's thumbnail for encrypted items).
    if source != "trash" {
        let thumb_abs = storage_root.join(&thumb_rel);
        let generated = generate_thumbnail_file(&full_path, &thumb_abs, &mime_type, None).await;
        if generated {
            tracing::debug!(photo_id = %photo_id, "Generated thumbnail on receive");
        } else {
            tracing::warn!(
                photo_id = %photo_id,
                "Thumbnail generation failed on receive; will be retried by autoscan"
            );
        }
    }

    tracing::debug!(
        "Received backup {} ({} bytes): {}",
        source,
        size_bytes,
        file_path
    );

    // Trigger encryption for the newly received file so it doesn't stay
    // as a plain-text entry.  Fire-and-forget — the response returns
    // immediately while encryption runs in the background.
    {
        let pool = state.pool.clone();
        let sr = storage_root.clone();
        let jwt = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            crate::photos::server_migrate::auto_migrate_after_scan(pool, sr, jwt).await;
        });
    }

    Ok(Json(serde_json::json!({
        "status": "ok",
        "photo_id": photo_id,
        "size_bytes": size_bytes,
    })))
}
