//! Photo management endpoints: listing, registration, favorites, and crop.
//!
//! File serving (originals, thumbnails, web previews) with Range/ETag support
//! lives in [`super::serve`].

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::*;
use super::utils::{normalize_iso_timestamp, utc_now_iso};

// ── Photo Endpoints ───────────────────────────────────────────────────────────

/// Query parameters for `GET /api/photos`.
#[derive(Debug, Deserialize)]
pub struct PhotoListQuery {
    /// Cursor for reverse-chronological pagination (taken_at or created_at).
    pub after: Option<String>,
    /// Maximum items to return (default 100, max 500).
    pub limit: Option<i64>,
    /// Filter by media type: "photo", "video", "gif", "audio".
    pub media_type: Option<String>,
    /// When `true`, return only favorited photos.
    pub favorites_only: Option<bool>,
    /// Filter by photo subtype: "motion", "panorama", "equirectangular", "hdr", "burst".
    pub subtype: Option<String>,
    /// When `true`, collapse burst sequences to show only the representative photo.
    pub collapse_bursts: Option<bool>,
}

/// GET /api/photos
/// List photos in the photos table for the authenticated user.
pub async fn list_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<PhotoListQuery>,
) -> Result<Json<PhotoListResponse>, AppError> {
    let limit = params.limit.unwrap_or(100).min(500);
    let fav_only = params.favorites_only.unwrap_or(false);

    let collapse = params.collapse_bursts.unwrap_or(false);

    // Build dynamic query
    let mut sql = if collapse {
        // When collapsing bursts, use a window function to pick the first photo
        // in each burst group and count the group size.
        String::from(
            "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
             duration_secs, taken_at, latitude, longitude, thumb_path, created_at, is_favorite, \
             crop_metadata, camera_model, photo_hash, photo_subtype, burst_id, motion_video_blob_id, \
             burst_count FROM (\
             SELECT *, \
             CASE WHEN burst_id IS NOT NULL THEN \
               (SELECT COUNT(*) FROM photos p2 WHERE p2.user_id = photos.user_id AND p2.burst_id = photos.burst_id) \
             ELSE NULL END AS burst_count, \
             CASE WHEN burst_id IS NOT NULL THEN \
               ROW_NUMBER() OVER (PARTITION BY burst_id ORDER BY COALESCE(taken_at, created_at) ASC) \
             ELSE 1 END AS rn \
             FROM photos WHERE user_id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL)",
        )
    } else {
        String::from(
            "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
             duration_secs, taken_at, latitude, longitude, thumb_path, created_at, is_favorite, \
             crop_metadata, camera_model, photo_hash, photo_subtype, burst_id, motion_video_blob_id, \
             NULL AS burst_count \
             FROM photos WHERE user_id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL)",
        )
    };
    let mut binds: Vec<String> = vec![auth.user_id.clone()];

    if let Some(ref mt) = params.media_type {
        sql.push_str(" AND media_type = ?");
        binds.push(mt.clone());
    }

    if let Some(ref st) = params.subtype {
        sql.push_str(" AND photo_subtype = ?");
        binds.push(st.clone());
    }

    if fav_only {
        sql.push_str(" AND is_favorite = 1");
    }

    if let Some(ref after) = params.after {
        sql.push_str(" AND COALESCE(taken_at, created_at) < ?");
        binds.push(after.clone());
    }

    if collapse {
        sql.push_str(") WHERE rn = 1 ORDER BY COALESCE(taken_at, created_at) DESC, filename ASC LIMIT ?");
    } else {
        sql.push_str(" ORDER BY COALESCE(taken_at, created_at) DESC, filename ASC LIMIT ?");
    }
    binds.push((limit + 1).to_string());

    let mut query = sqlx::query_as::<_, PhotoRecord>(&sql);
    for (i, val) in binds.iter().enumerate() {
        if i == binds.len() - 1 {
            // Last bind is the limit (integer)
            query = query.bind(val.parse::<i64>().unwrap_or(limit + 1));
        } else {
            query = query.bind(val);
        }
    }

    let photos = query.fetch_all(&state.read_pool).await?;

    let next_cursor = if photos.len() as i64 > limit {
        photos
            .last()
            .map(|p| p.taken_at.clone().unwrap_or_else(|| p.created_at.clone()))
    } else {
        None
    };

    let photos: Vec<PhotoRecord> = photos.into_iter().take(limit as usize).collect();

    Ok(Json(PhotoListResponse {
        photos,
        next_cursor,
    }))
}

/// POST /api/photos/register
/// Register a file on disk as a photo in the database.
/// The file must already exist at the given path within the storage root.
pub async fn register_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<RegisterPhotoRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Security: ensure file_path is a safe relative path (no traversal, no absolute)
    sanitize::validate_relative_path(&req.file_path)
        .map_err(|reason| AppError::BadRequest(format!("Invalid file_path: {}", reason)))?;

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&req.file_path);

    // Verify the file actually exists
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err(AppError::BadRequest(format!(
            "File does not exist: {}",
            req.file_path
        )));
    }

    let photo_id = Uuid::new_v4().to_string();
    let now = utc_now_iso();
    let media_type = req.media_type.unwrap_or_else(|| {
        if req.mime_type.starts_with("video/") {
            "video".to_string()
        } else if req.mime_type.starts_with("audio/") {
            "audio".to_string()
        } else if req.mime_type == "image/gif" {
            "gif".to_string()
        } else {
            "photo".to_string()
        }
    });

    // Compute content-based hash using streaming I/O (64 KB chunks) so large
    // files never need to be buffered entirely in memory.
    let photo_hash = super::utils::compute_photo_hash_streaming(&full_path)
        .await
        .unwrap_or_default();

    // ── Hash-based dedup ────────────────────────────────────────────────────
    // If a photo with the same content hash already exists for this user,
    // return the existing record instead of inserting a duplicate.  This
    // mirrors the upload endpoint's dedup behaviour.
    if !photo_hash.is_empty() {
        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT id, file_path FROM photos \
             WHERE user_id = ? AND photo_hash = ? LIMIT 1",
        )
        .bind(&auth.user_id)
        .bind(&photo_hash)
        .fetch_optional(&state.read_pool)
        .await?;

        if let Some((eid, _epath)) = existing {
            tracing::info!(
                user_id = %auth.user_id,
                existing_photo_id = %eid,
                photo_hash = %photo_hash,
                "Duplicate registration detected (hash match) — returning existing record"
            );
            return Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "photo_id": eid,
                    "photo_hash": photo_hash,
                    "duplicate": true,
                })),
            ));
        }
    }

    // Generate thumbnail path (will be created by a separate endpoint/process)
    let thumb_filename = format!("{}.thumb.jpg", photo_id);
    let thumb_rel = format!(".thumbnails/{}", thumb_filename);

    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path, created_at, photo_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&req.filename)
    .bind(&req.file_path)
    .bind(&req.mime_type)
    .bind(&media_type)
    .bind(req.size_bytes)
    .bind(req.width.unwrap_or(0))
    .bind(req.height.unwrap_or(0))
    .bind(req.duration_secs)
    .bind(req.taken_at.as_ref().map(|t| normalize_iso_timestamp(t)))
    .bind(req.latitude)
    .bind(req.longitude)
    .bind(&thumb_rel)
    .bind(&now)
    .bind(&photo_hash)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state,
        AuditEvent::PhotoRegister,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "photo_id": photo_id,
            "filename": req.filename,
            "media_type": media_type,
        })),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "photo_id": photo_id,
            "thumb_path": thumb_rel,
            "photo_hash": photo_hash,
        })),
    ))
}

/// POST /api/photos/register-encrypted
/// Create a photos record linked to already-uploaded encrypted blobs.
/// This is called by mobile clients after uploading blobs to bridge the
/// gap between the blobs table and the photos table.
pub async fn register_encrypted_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<RegisterEncryptedPhotoRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Verify the blob actually exists and belongs to this user.
    let blob_exists: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&req.encrypted_blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?;

    if blob_exists.is_none() {
        return Err(AppError::BadRequest(format!(
            "Blob {} not found or does not belong to user",
            req.encrypted_blob_id
        )));
    }

    // Dedup: if a photo already references this blob, return it.
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM photos WHERE user_id = ? AND encrypted_blob_id = ? LIMIT 1",
    )
    .bind(&auth.user_id)
    .bind(&req.encrypted_blob_id)
    .fetch_optional(&state.read_pool)
    .await?;

    if let Some((eid,)) = existing {
        tracing::info!(
            user_id = %auth.user_id,
            existing_photo_id = %eid,
            blob_id = %req.encrypted_blob_id,
            "register-encrypted: photo already exists for this blob"
        );
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "photo_id": eid,
                "duplicate": true,
            })),
        ));
    }

    // Content-hash dedup.
    if let Some(ref hash) = req.photo_hash {
        if !hash.is_empty() {
            let existing: Option<(String,)> = sqlx::query_as(
                "SELECT id FROM photos WHERE user_id = ? AND photo_hash = ? LIMIT 1",
            )
            .bind(&auth.user_id)
            .bind(hash)
            .fetch_optional(&state.read_pool)
            .await?;

            if let Some((eid,)) = existing {
                tracing::info!(
                    user_id = %auth.user_id,
                    existing_photo_id = %eid,
                    photo_hash = %hash,
                    "register-encrypted: duplicate hash — returning existing"
                );
                return Ok((
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "photo_id": eid,
                        "duplicate": true,
                    })),
                ));
            }
        }
    }

    let photo_id = Uuid::new_v4().to_string();
    let now = utc_now_iso();
    let media_type = req.media_type.unwrap_or_else(|| {
        if req.mime_type.starts_with("video/") {
            "video".to_string()
        } else if req.mime_type == "image/gif" {
            "gif".to_string()
        } else {
            "photo".to_string()
        }
    });

    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
         size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
         thumb_path, created_at, encrypted_blob_id, encrypted_thumb_blob_id, photo_hash, \
         photo_subtype, burst_id, motion_video_blob_id) \
         VALUES (?, ?, ?, '', ?, ?, 0, ?, ?, ?, ?, ?, ?, '', ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&req.filename)
    .bind(&req.mime_type)
    .bind(&media_type)
    .bind(req.width.unwrap_or(0))
    .bind(req.height.unwrap_or(0))
    .bind(req.duration_secs)
    .bind(req.taken_at.as_ref().map(|t| normalize_iso_timestamp(t)))
    .bind(req.latitude)
    .bind(req.longitude)
    .bind(&now)
    .bind(&req.encrypted_blob_id)
    .bind(&req.encrypted_thumb_blob_id)
    .bind(&req.photo_hash)
    .bind(&req.photo_subtype)
    .bind(&req.burst_id)
    .bind(&req.motion_video_blob_id)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state,
        AuditEvent::PhotoRegister,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "photo_id": photo_id,
            "filename": req.filename,
            "encrypted_blob_id": req.encrypted_blob_id,
        })),
    )
    .await;

    tracing::info!(
        user_id = %auth.user_id,
        photo_id = %photo_id,
        filename = %req.filename,
        blob_id = %req.encrypted_blob_id,
        "register-encrypted: created photos row"
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "photo_id": photo_id,
        })),
    ))
}

// ── Favorite Toggle ─────────────────────────────────────────────────────────

/// PUT /api/photos/:id/favorite
/// Toggle the is_favorite flag on a photo.
///
/// **Performance:** Uses `RETURNING` (SQLite 3.35+) to get the new value in
/// the same statement, eliminating a second SELECT round-trip.
pub async fn toggle_favorite(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Toggle and return new value in a single statement (RETURNING, SQLite 3.35+).
    // Eliminates a second SELECT query that was previously needed to read back
    // the toggled value.
    let new_fav: Option<bool> = sqlx::query_scalar(
        "UPDATE photos SET is_favorite = 1 - is_favorite \
         WHERE id = ? AND user_id = ? \
         RETURNING is_favorite",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let is_favorite = new_fav.ok_or(AppError::NotFound)?;

    audit::log(
        &state,
        AuditEvent::PhotoFavorite,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "photo_id": photo_id,
            "is_favorite": is_favorite,
        })),
    )
    .await;

    Ok(Json(serde_json::json!({
        "id": photo_id,
        "is_favorite": is_favorite,
    })))
}

// ── Favorite sync (lightweight delta for cross-device sync) ───────────────────

/// Lightweight record returned by the favorite-sync endpoint.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FavSyncRecord {
    pub id: String,
    pub is_favorite: bool,
}

/// GET /api/photos/favorite-sync
///
/// Returns `{id, is_favorite}` for **all** of the user's photos.
/// Android clients poll this during periodic sync so favorites
/// toggled on the web (or another device) are reflected locally.
pub async fn favorite_sync(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<FavSyncRecord>>, AppError> {
    let records = sqlx::query_as::<_, FavSyncRecord>(
        "SELECT id, is_favorite FROM photos WHERE user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(records))
}

// ── Batch dimension update ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DimensionUpdate {
    pub photo_id: Option<String>,
    pub blob_id: Option<String>,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Deserialize)]
pub struct BatchDimensionUpdateRequest {
    pub updates: Vec<DimensionUpdate>,
}

/// PATCH /api/photos/dimensions
/// Batch-update width/height for photos owned by the authenticated user.
/// Each item can identify the photo by `photo_id` or `blob_id` (encrypted_blob_id).
pub async fn batch_update_dimensions(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<BatchDimensionUpdateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut updated = 0u64;
    for item in &req.updates {
        if item.width <= 0 || item.height <= 0 {
            continue;
        }
        let rows = if let Some(ref pid) = item.photo_id {
            sqlx::query(
                "UPDATE photos SET width = ?, height = ? WHERE id = ? AND user_id = ?",
            )
            .bind(item.width)
            .bind(item.height)
            .bind(pid)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?
            .rows_affected()
        } else if let Some(ref bid) = item.blob_id {
            sqlx::query(
                "UPDATE photos SET width = ?, height = ? WHERE encrypted_blob_id = ? AND user_id = ?",
            )
            .bind(item.width)
            .bind(item.height)
            .bind(bid)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?
            .rows_affected()
        } else {
            0
        };
        updated += rows;
    }

    Ok(Json(serde_json::json!({ "updated": updated })))
}

// ── Burst photo listing ───────────────────────────────────────────────────────

/// GET /api/photos/burst/{burst_id}
/// Returns all photos in a burst sequence, ordered by taken_at ascending.
pub async fn list_burst_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(burst_id): Path<String>,
) -> Result<Json<Vec<PhotoRecord>>, AppError> {
    let photos = sqlx::query_as::<_, PhotoRecord>(
        "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
         duration_secs, taken_at, latitude, longitude, thumb_path, created_at, is_favorite, \
         crop_metadata, camera_model, photo_hash, photo_subtype, burst_id, motion_video_blob_id, \
         (SELECT COUNT(*) FROM photos p2 WHERE p2.user_id = photos.user_id AND p2.burst_id = photos.burst_id) AS burst_count \
         FROM photos WHERE user_id = ? AND burst_id = ? \
         ORDER BY COALESCE(taken_at, created_at) ASC",
    )
    .bind(&auth.user_id)
    .bind(&burst_id)
    .fetch_all(&state.read_pool)
    .await?;

    if photos.is_empty() {
        return Err(AppError::NotFound);
    }

    Ok(Json(photos))
}

/// POST /api/photos/detect-bursts
/// Manually trigger timestamp-based burst detection for the current user.
pub async fn detect_bursts(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let groups = crate::photos::burst::detect_bursts_for_user(&state.pool, &auth.user_id)
        .await
        .map_err(|e| AppError::Internal(format!("Burst detection failed: {}", e)))?;

    Ok(Json(serde_json::json!({
        "burst_groups_created": groups,
    })))
}
