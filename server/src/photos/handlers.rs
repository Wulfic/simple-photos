//! Photo management endpoints: listing, registration, favorites, and crop.
//!
//! File serving (originals, thumbnails, web previews) with Range/ETag support
//! lives in [`super::serve`].

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

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

    // Build dynamic query
    let mut sql = String::from(
        "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
         duration_secs, taken_at, latitude, longitude, thumb_path, created_at, is_favorite, crop_metadata, camera_model, photo_hash \
         FROM photos WHERE user_id = ?"
    );
    let mut binds: Vec<String> = vec![auth.user_id.clone()];

    if let Some(ref mt) = params.media_type {
        sql.push_str(" AND media_type = ?");
        binds.push(mt.clone());
    }

    if fav_only {
        sql.push_str(" AND is_favorite = 1");
    }

    if let Some(ref after) = params.after {
        sql.push_str(" AND COALESCE(taken_at, created_at) < ?");
        binds.push(after.clone());
    }

    sql.push_str(" ORDER BY COALESCE(taken_at, created_at) DESC, filename ASC LIMIT ?");
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

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "photo_id": photo_id,
            "thumb_path": thumb_rel,
            "photo_hash": photo_hash,
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

    Ok(Json(serde_json::json!({
        "id": photo_id,
        "is_favorite": is_favorite,
    })))
}

// ── Crop Metadata ───────────────────────────────────────────────────────────

/// Request body for `PUT /api/photos/{id}/crop`.
/// `crop_metadata` is a JSON string describing the crop rectangle as percentage
/// coordinates: `{"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 0}`.
/// Send `null` to clear the crop.
#[derive(Debug, Deserialize)]
pub struct SetCropRequest {
    pub crop_metadata: Option<String>,
}

/// PUT /api/photos/:id/crop
/// Set (or clear) crop metadata for a photo.
/// crop_metadata is a JSON string describing the crop rectangle:
/// {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 0}
/// Values are percentages (0.0-1.0) of original dimensions.
/// Send null to clear the crop.
pub async fn set_crop(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<SetCropRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate crop_metadata is valid JSON if provided, and limit size
    if let Some(ref crop) = req.crop_metadata {
        let crop = sanitize::sanitize_freeform(crop, 1024);
        if serde_json::from_str::<serde_json::Value>(&crop).is_err() {
            return Err(AppError::BadRequest(
                "crop_metadata must be valid JSON".into(),
            ));
        }
    }

    let rows = sqlx::query("UPDATE photos SET crop_metadata = ? WHERE id = ? AND user_id = ?")
        .bind(
            req.crop_metadata
                .as_ref()
                .map(|c| sanitize::sanitize_freeform(c, 1024)),
        )
        .bind(&photo_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({
        "id": photo_id,
        "crop_metadata": req.crop_metadata,
    })))
}
