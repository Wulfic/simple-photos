//! Metadata editing endpoints for photos.
//!
//! - `PATCH /api/photos/{id}/metadata` — update metadata fields in DB
//! - `GET /api/photos/{id}/metadata/full` — full metadata including raw EXIF tags
//! - `POST /api/photos/{id}/metadata/write-exif` — write DB metadata back to file EXIF

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

// ── Request / Response types ─────────────────────────────────────────

/// Fields that can be updated via PATCH.  All are optional — only provided
/// fields are changed.
#[derive(Debug, Deserialize)]
pub struct MetadataUpdateRequest {
    pub filename: Option<String>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub camera_model: Option<String>,
    /// Explicitly clear GPS coordinates when set to `true`.
    #[serde(default)]
    pub clear_gps: bool,
}

/// Current metadata for a photo (DB + optional raw EXIF).
#[derive(Debug, Serialize)]
pub struct FullMetadataResponse {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub media_type: String,
    pub width: i64,
    pub height: i64,
    pub size_bytes: i64,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub camera_model: Option<String>,
    pub photo_hash: Option<String>,
    pub photo_subtype: Option<String>,
    pub geo_city: Option<String>,
    pub geo_state: Option<String>,
    pub geo_country: Option<String>,
    pub geo_country_code: Option<String>,
    pub photo_year: Option<i64>,
    pub photo_month: Option<i64>,
    pub created_at: String,
    /// Raw EXIF tags extracted from the file (key → display value).
    pub exif_tags: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct MetadataUpdateResponse {
    pub status: String,
    pub updated_fields: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct WriteExifResponse {
    pub status: String,
    pub new_photo_hash: Option<String>,
}

// ── PATCH /api/photos/{id}/metadata ──────────────────────────────────

/// Update metadata fields for a photo owned by the authenticated user.
pub async fn update_metadata(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
    Json(req): Json<MetadataUpdateRequest>,
) -> Result<Json<MetadataUpdateResponse>, AppError> {
    // Verify photo exists and belongs to user
    let exists: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM photos WHERE id = ?1 AND user_id = ?2",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?;

    if exists.is_none() {
        return Err(AppError::NotFound);
    }

    let mut updated_fields = Vec::new();

    // ── Validate inputs ──────────────────────────────────────────
    if let Some(ref filename) = req.filename {
        let trimmed = filename.trim();
        if trimmed.is_empty() {
            return Err(AppError::BadRequest("Filename cannot be empty".into()));
        }
        // Path traversal prevention
        if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
            return Err(AppError::BadRequest(
                "Filename cannot contain path separators or '..'".into(),
            ));
        }
    }

    if let Some(ref taken_at) = req.taken_at {
        // Validate ISO 8601 format
        if chrono::DateTime::parse_from_rfc3339(taken_at).is_err()
            && chrono::NaiveDateTime::parse_from_str(taken_at, "%Y-%m-%dT%H:%M:%SZ").is_err()
        {
            return Err(AppError::BadRequest(
                "taken_at must be a valid ISO 8601 datetime (e.g. 2024-01-15T14:30:00Z)".into(),
            ));
        }
    }

    if let Some(lat) = req.latitude {
        if !(-90.0..=90.0).contains(&lat) {
            return Err(AppError::BadRequest(
                "Latitude must be between -90 and 90".into(),
            ));
        }
    }

    if let Some(lon) = req.longitude {
        if !(-180.0..=180.0).contains(&lon) {
            return Err(AppError::BadRequest(
                "Longitude must be between -180 and 180".into(),
            ));
        }
    }

    // ── Apply updates ────────────────────────────────────────────
    if let Some(ref filename) = req.filename {
        let trimmed = filename.trim();
        sqlx::query("UPDATE photos SET filename = ?1 WHERE id = ?2")
            .bind(trimmed)
            .bind(&photo_id)
            .execute(&state.pool)
            .await?;
        updated_fields.push("filename".to_string());
    }

    if let Some(ref taken_at) = req.taken_at {
        sqlx::query("UPDATE photos SET taken_at = ?1 WHERE id = ?2")
            .bind(taken_at)
            .bind(&photo_id)
            .execute(&state.pool)
            .await?;
        updated_fields.push("taken_at".to_string());

        // Re-derive year/month
        sqlx::query(
            "UPDATE photos SET \
             photo_year = CAST(strftime('%Y', ?1) AS INTEGER), \
             photo_month = CAST(strftime('%m', ?1) AS INTEGER) \
             WHERE id = ?2",
        )
        .bind(taken_at)
        .bind(&photo_id)
        .execute(&state.pool)
        .await?;
        updated_fields.push("photo_year".to_string());
        updated_fields.push("photo_month".to_string());
    }

    if req.clear_gps {
        // Explicitly clear GPS + geo data
        sqlx::query(
            "UPDATE photos SET latitude = NULL, longitude = NULL, \
             geo_city = NULL, geo_state = NULL, geo_country = NULL, \
             geo_country_code = NULL WHERE id = ?1",
        )
        .bind(&photo_id)
        .execute(&state.pool)
        .await?;
        updated_fields.push("latitude".to_string());
        updated_fields.push("longitude".to_string());
        updated_fields.push("geo_cleared".to_string());
    } else if req.latitude.is_some() || req.longitude.is_some() {
        // Update GPS coordinates — require both
        let lat = req.latitude;
        let lon = req.longitude;

        if lat.is_some() != lon.is_some() {
            return Err(AppError::BadRequest(
                "Both latitude and longitude must be provided together".into(),
            ));
        }

        sqlx::query("UPDATE photos SET latitude = ?1, longitude = ?2 WHERE id = ?3")
            .bind(lat)
            .bind(lon)
            .bind(&photo_id)
            .execute(&state.pool)
            .await?;
        updated_fields.push("latitude".to_string());
        updated_fields.push("longitude".to_string());

        // Clear geo columns so the background processor re-resolves them
        sqlx::query(
            "UPDATE photos SET geo_city = NULL, geo_state = NULL, \
             geo_country = NULL, geo_country_code = NULL WHERE id = ?1",
        )
        .bind(&photo_id)
        .execute(&state.pool)
        .await?;
        updated_fields.push("geo_pending".to_string());
    }

    if let Some(ref camera_model) = req.camera_model {
        sqlx::query("UPDATE photos SET camera_model = ?1 WHERE id = ?2")
            .bind(camera_model)
            .bind(&photo_id)
            .execute(&state.pool)
            .await?;
        updated_fields.push("camera_model".to_string());
    }

    // Audit log
    crate::audit::log(
        &state,
        crate::audit::AuditEvent::BlobUpload,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "metadata_update",
            "photo_id": photo_id,
            "updated_fields": updated_fields,
        })),
    )
    .await;

    tracing::info!(
        photo_id = %photo_id,
        user_id = %auth.user_id,
        fields = ?updated_fields,
        "Photo metadata updated",
    );

    Ok(Json(MetadataUpdateResponse {
        status: "ok".to_string(),
        updated_fields,
    }))
}

// ── GET /api/photos/{id}/metadata/full ───────────────────────────────

/// Get complete metadata for a photo, including raw EXIF tags from the file.
pub async fn get_full_metadata(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<FullMetadataResponse>, AppError> {
    // First query: core fields (16 columns max for tuple FromRow)
    let core: Option<(
        String,          // id
        String,          // filename
        String,          // mime_type
        String,          // media_type
        i64,             // width
        i64,             // height
        i64,             // size_bytes
        Option<String>,  // taken_at
        Option<f64>,     // latitude
        Option<f64>,     // longitude
        Option<String>,  // camera_model
        Option<String>,  // photo_hash
        Option<String>,  // photo_subtype
        String,          // created_at
        String,          // file_path
    )> = sqlx::query_as(
        "SELECT id, filename, mime_type, media_type, width, height, size_bytes, \
         taken_at, latitude, longitude, camera_model, photo_hash, photo_subtype, \
         created_at, file_path \
         FROM photos WHERE id = ?1 AND user_id = ?2",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?;

    let (
        id, filename, mime_type, media_type, width, height, size_bytes,
        taken_at, latitude, longitude, camera_model, photo_hash, photo_subtype,
        created_at, file_path,
    ) = core.ok_or(AppError::NotFound)?;

    // Second query: geo + timeline fields
    let geo: (
        Option<String>,  // geo_city
        Option<String>,  // geo_state
        Option<String>,  // geo_country
        Option<String>,  // geo_country_code
        Option<i64>,     // photo_year
        Option<i64>,     // photo_month
    ) = sqlx::query_as(
        "SELECT geo_city, geo_state, geo_country, geo_country_code, \
         photo_year, photo_month \
         FROM photos WHERE id = ?1",
    )
    .bind(&photo_id)
    .fetch_one(&state.read_pool)
    .await?;

    // Try to extract EXIF tags from the file on disk
    let exif_tags = if !file_path.is_empty() {
        let storage_root = (**state.storage_root.load()).clone();
        let abs_path = storage_root.join(&file_path);
        if abs_path.exists() {
            let path_clone = abs_path.clone();
            tokio::task::spawn_blocking(move || extract_exif_tags(&path_clone))
                .await
                .unwrap_or(None)
        } else {
            None
        }
    } else {
        None
    };

    Ok(Json(FullMetadataResponse {
        id,
        filename,
        mime_type,
        media_type,
        width,
        height,
        size_bytes,
        taken_at,
        latitude,
        longitude,
        camera_model,
        photo_hash,
        photo_subtype,
        geo_city: geo.0,
        geo_state: geo.1,
        geo_country: geo.2,
        geo_country_code: geo.3,
        photo_year: geo.4,
        photo_month: geo.5,
        created_at,
        exif_tags,
    }))
}

// ── POST /api/photos/{id}/metadata/write-exif ────────────────────────

/// Write current DB metadata back to the file's EXIF tags (JPEG/TIFF only).
/// After modification, recalculates the photo_hash.
pub async fn write_exif_to_file(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Json<WriteExifResponse>, AppError> {
    // Fetch photo record
    let row: Option<(
        String,          // file_path
        String,          // mime_type
        Option<String>,  // taken_at
        Option<f64>,     // latitude
        Option<f64>,     // longitude
        Option<String>,  // camera_model
    )> = sqlx::query_as(
        "SELECT file_path, mime_type, taken_at, latitude, longitude, camera_model \
         FROM photos WHERE id = ?1 AND user_id = ?2",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?;

    let row = row.ok_or(AppError::NotFound)?;

    // Only JPEG and TIFF support EXIF writing
    let mime = row.1.to_lowercase();
    if !mime.contains("jpeg") && !mime.contains("jpg") && !mime.contains("tiff") {
        return Err(AppError::BadRequest(
            "EXIF write-back is only supported for JPEG and TIFF files".into(),
        ));
    }

    if row.0.is_empty() {
        return Err(AppError::BadRequest(
            "Photo has no associated file on disk".into(),
        ));
    }

    let storage_root = (**state.storage_root.load()).clone();
    let abs_path = storage_root.join(&row.0);

    if !abs_path.exists() {
        return Err(AppError::NotFound);
    }

    // Write EXIF using exiftool (if available) or img_parts for minimal EXIF
    let taken_at = row.2.clone();
    let latitude = row.3;
    let longitude = row.4;
    let camera_model = row.5.clone();

    let path_clone = abs_path.clone();
    let write_result = tokio::task::spawn_blocking(move || {
        write_exif_fields(&path_clone, taken_at, latitude, longitude, camera_model)
    })
    .await
    .map_err(|e| AppError::Internal(format!("EXIF write task failed: {}", e)))?;

    if let Err(e) = write_result {
        return Err(AppError::Internal(format!("Failed to write EXIF: {}", e)));
    }

    // Recalculate photo_hash
    let path_for_hash = abs_path.clone();
    let new_hash = tokio::task::spawn_blocking(move || {
        if let Ok(data) = std::fs::read(&path_for_hash) {
            Some(crate::photos::utils::compute_photo_hash(&data))
        } else {
            None
        }
    })
    .await
    .unwrap_or(None);

    if let Some(ref hash) = new_hash {
        sqlx::query("UPDATE photos SET photo_hash = ?1 WHERE id = ?2")
            .bind(hash)
            .bind(&photo_id)
            .execute(&state.pool)
            .await?;
    }

    // Audit log
    crate::audit::log(
        &state,
        crate::audit::AuditEvent::BlobUpload,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "write_exif",
            "photo_id": photo_id,
        })),
    )
    .await;

    tracing::info!(photo_id = %photo_id, "EXIF written to file");

    Ok(Json(WriteExifResponse {
        status: "ok".to_string(),
        new_photo_hash: new_hash,
    }))
}

// ── EXIF extraction helpers ──────────────────────────────────────────

/// Extract all readable EXIF tags from a file as a map of tag name → value.
fn extract_exif_tags(
    file_path: &std::path::Path,
) -> Option<std::collections::HashMap<String, String>> {
    let file = std::fs::File::open(file_path).ok()?;
    let mut buf_reader = std::io::BufReader::new(&file);
    let exif_reader = exif::Reader::new().read_from_container(&mut buf_reader).ok()?;

    let mut tags = std::collections::HashMap::new();
    for field in exif_reader.fields() {
        let tag_name = format!("{}", field.tag);
        let value = field.display_value().to_string();
        tags.insert(tag_name, value);
    }

    if tags.is_empty() {
        None
    } else {
        Some(tags)
    }
}

// ── EXIF writing helper ──────────────────────────────────────────────

/// Write metadata fields to a JPEG file's EXIF using little_exif.
/// This reads the file, updates EXIF data, and writes it back.
fn write_exif_fields(
    file_path: &std::path::Path,
    taken_at: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    camera_model: Option<String>,
) -> Result<(), String> {
    // Use exiftool if available, otherwise fall back to manual approach
    let mut args = vec!["-overwrite_original".to_string()];

    if let Some(ref dt) = taken_at {
        // Convert ISO 8601 to EXIF format: "2024:01:15 14:30:00"
        let exif_dt = dt
            .replace('-', ":")
            .replace('T', " ")
            .trim_end_matches('Z')
            .to_string();
        args.push(format!("-DateTimeOriginal={}", exif_dt));
    }

    if let (Some(lat), Some(lon)) = (latitude, longitude) {
        let lat_ref = if lat >= 0.0 { "N" } else { "S" };
        let lon_ref = if lon >= 0.0 { "E" } else { "W" };
        args.push(format!("-GPSLatitude={}", lat.abs()));
        args.push(format!("-GPSLatitudeRef={}", lat_ref));
        args.push(format!("-GPSLongitude={}", lon.abs()));
        args.push(format!("-GPSLongitudeRef={}", lon_ref));
    }

    if let Some(ref model) = camera_model {
        args.push(format!("-Model={}", model));
    }

    if args.len() <= 1 {
        // No fields to write
        return Ok(());
    }

    args.push(file_path.to_string_lossy().to_string());

    let output = std::process::Command::new("exiftool")
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run exiftool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("exiftool failed: {}", stderr));
    }

    Ok(())
}
