//! Metadata editing endpoints for photos.
//!
//! - `PATCH /api/photos/{id}/metadata` — update metadata fields in DB
//! - `GET /api/photos/{id}/metadata/full` — full metadata including raw EXIF tags
//! - `POST /api/photos/{id}/metadata/write-exif` — write DB metadata back to file EXIF

use axum::extract::{Path, State};
use axum::http::HeaderMap;
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
    // ── Extended EXIF fields ──────────────────────────────────────
    pub camera_make: Option<String>,
    pub lens_model: Option<String>,
    pub iso_speed: Option<i64>,
    pub f_number: Option<f64>,
    pub exposure_time: Option<String>,
    pub focal_length: Option<f64>,
    pub flash: Option<String>,
    pub white_balance: Option<String>,
    pub exposure_program: Option<String>,
    pub metering_mode: Option<String>,
    pub orientation: Option<i64>,
    pub software: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    pub description: Option<String>,
    pub user_comment: Option<String>,
    pub color_space: Option<String>,
    pub exposure_bias: Option<f64>,
    pub scene_type: Option<String>,
    pub digital_zoom: Option<f64>,
    /// Arbitrary EXIF tag overrides (tag name → value).
    pub exif_overrides: Option<std::collections::HashMap<String, String>>,
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
    // ── Extended EXIF fields ──────────────────────────────────────
    pub camera_make: Option<String>,
    pub lens_model: Option<String>,
    pub iso_speed: Option<i64>,
    pub f_number: Option<f64>,
    pub exposure_time: Option<String>,
    pub focal_length: Option<f64>,
    pub flash: Option<String>,
    pub white_balance: Option<String>,
    pub exposure_program: Option<String>,
    pub metering_mode: Option<String>,
    pub orientation: Option<i64>,
    pub software: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    pub description: Option<String>,
    pub user_comment: Option<String>,
    pub color_space: Option<String>,
    pub exposure_bias: Option<f64>,
    pub scene_type: Option<String>,
    pub digital_zoom: Option<f64>,
    pub exif_overrides: Option<std::collections::HashMap<String, String>>,
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

    // ── Extended EXIF field updates ──────────────────────────────
    macro_rules! update_text_field {
        ($field:ident, $col:expr) => {
            if let Some(ref val) = req.$field {
                sqlx::query(&format!("UPDATE photos SET {} = ?1 WHERE id = ?2", $col))
                    .bind(val)
                    .bind(&photo_id)
                    .execute(&state.pool)
                    .await?;
                updated_fields.push($col.to_string());
            }
        };
    }
    macro_rules! update_num_field {
        ($field:ident, $col:expr) => {
            if let Some(val) = req.$field {
                sqlx::query(&format!("UPDATE photos SET {} = ?1 WHERE id = ?2", $col))
                    .bind(val)
                    .bind(&photo_id)
                    .execute(&state.pool)
                    .await?;
                updated_fields.push($col.to_string());
            }
        };
    }

    update_text_field!(camera_make, "camera_make");
    update_text_field!(lens_model, "lens_model");
    update_num_field!(iso_speed, "iso_speed");
    update_num_field!(f_number, "f_number");
    update_text_field!(exposure_time, "exposure_time");
    update_num_field!(focal_length, "focal_length");
    update_text_field!(flash, "flash");
    update_text_field!(white_balance, "white_balance");
    update_text_field!(exposure_program, "exposure_program");
    update_text_field!(metering_mode, "metering_mode");
    update_num_field!(orientation, "orientation");
    update_text_field!(software, "software");
    update_text_field!(artist, "artist");
    update_text_field!(copyright, "copyright");
    update_text_field!(description, "description");
    update_text_field!(user_comment, "user_comment");
    update_text_field!(color_space, "color_space");
    update_num_field!(exposure_bias, "exposure_bias");
    update_text_field!(scene_type, "scene_type");
    update_num_field!(digital_zoom, "digital_zoom");

    if let Some(ref overrides) = req.exif_overrides {
        let json = serde_json::to_string(overrides)
            .map_err(|e| AppError::BadRequest(format!("Invalid exif_overrides: {}", e)))?;
        sqlx::query("UPDATE photos SET exif_overrides = ?1 WHERE id = ?2")
            .bind(&json)
            .bind(&photo_id)
            .execute(&state.pool)
            .await?;
        updated_fields.push("exif_overrides".to_string());
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

    // Third query: extended EXIF columns
    let exif_ext: (
        Option<String>,  // camera_make
        Option<String>,  // lens_model
        Option<i64>,     // iso_speed
        Option<f64>,     // f_number
        Option<String>,  // exposure_time
        Option<f64>,     // focal_length
        Option<String>,  // flash
        Option<String>,  // white_balance
        Option<String>,  // exposure_program
        Option<String>,  // metering_mode
        Option<i64>,     // orientation
        Option<String>,  // software
        Option<String>,  // artist
        Option<String>,  // copyright
        Option<String>,  // description
        Option<String>,  // user_comment
    ) = sqlx::query_as(
        "SELECT camera_make, lens_model, iso_speed, f_number, exposure_time, \
         focal_length, flash, white_balance, exposure_program, metering_mode, \
         orientation, software, artist, copyright, description, user_comment \
         FROM photos WHERE id = ?1",
    )
    .bind(&photo_id)
    .fetch_one(&state.read_pool)
    .await?;

    // Fourth query: remaining extended EXIF columns
    let exif_ext2: (
        Option<String>,  // color_space
        Option<f64>,     // exposure_bias
        Option<String>,  // scene_type
        Option<f64>,     // digital_zoom
        Option<String>,  // exif_overrides (JSON)
    ) = sqlx::query_as(
        "SELECT color_space, exposure_bias, scene_type, digital_zoom, exif_overrides \
         FROM photos WHERE id = ?1",
    )
    .bind(&photo_id)
    .fetch_one(&state.read_pool)
    .await?;

    let exif_overrides_parsed: Option<std::collections::HashMap<String, String>> =
        exif_ext2.4.as_ref().and_then(|s| serde_json::from_str(s).ok());

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
        camera_make: exif_ext.0,
        lens_model: exif_ext.1,
        iso_speed: exif_ext.2,
        f_number: exif_ext.3,
        exposure_time: exif_ext.4,
        focal_length: exif_ext.5,
        flash: exif_ext.6,
        white_balance: exif_ext.7,
        exposure_program: exif_ext.8,
        metering_mode: exif_ext.9,
        orientation: exif_ext.10,
        software: exif_ext.11,
        artist: exif_ext.12,
        copyright: exif_ext.13,
        description: exif_ext.14,
        user_comment: exif_ext.15,
        color_space: exif_ext2.0,
        exposure_bias: exif_ext2.1,
        scene_type: exif_ext2.2,
        digital_zoom: exif_ext2.3,
        exif_overrides: exif_overrides_parsed,
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
    // Fetch photo record — core fields
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

    // Fetch extended EXIF fields
    let ext: (
        Option<String>, Option<String>, Option<i64>, Option<f64>,
        Option<String>, Option<f64>, Option<String>, Option<String>,
        Option<String>, Option<String>, Option<i64>, Option<String>,
        Option<String>, Option<String>, Option<String>, Option<String>,
    ) = sqlx::query_as(
        "SELECT camera_make, lens_model, iso_speed, f_number, exposure_time, \
         focal_length, flash, white_balance, exposure_program, metering_mode, \
         orientation, software, artist, copyright, description, user_comment \
         FROM photos WHERE id = ?1",
    )
    .bind(&photo_id)
    .fetch_one(&state.read_pool)
    .await?;

    let ext2: (Option<String>, Option<f64>, Option<String>, Option<f64>, Option<String>) =
        sqlx::query_as(
            "SELECT color_space, exposure_bias, scene_type, digital_zoom, exif_overrides \
             FROM photos WHERE id = ?1",
        )
        .bind(&photo_id)
        .fetch_one(&state.read_pool)
        .await?;

    let exif_overrides: Option<std::collections::HashMap<String, String>> =
        ext2.4.as_ref().and_then(|s| serde_json::from_str(s).ok());

    // Build the full EXIF write fields struct
    let write_fields = ExifWriteFields {
        taken_at: row.2.clone(),
        latitude: row.3,
        longitude: row.4,
        camera_model: row.5.clone(),
        camera_make: ext.0,
        lens_model: ext.1,
        iso_speed: ext.2,
        f_number: ext.3,
        exposure_time: ext.4,
        focal_length: ext.5,
        flash: ext.6,
        white_balance: ext.7,
        exposure_program: ext.8,
        metering_mode: ext.9,
        orientation: ext.10,
        software: ext.11,
        artist: ext.12,
        copyright: ext.13,
        description: ext.14,
        user_comment: ext.15,
        color_space: ext2.0,
        exposure_bias: ext2.1,
        scene_type: ext2.2,
        digital_zoom: ext2.3,
        exif_overrides,
    };

    let path_clone = abs_path.clone();
    let write_result = tokio::task::spawn_blocking(move || {
        write_exif_fields_full(&path_clone, &write_fields)
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

/// All metadata fields that can be written to EXIF.
struct ExifWriteFields {
    taken_at: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    camera_model: Option<String>,
    camera_make: Option<String>,
    lens_model: Option<String>,
    iso_speed: Option<i64>,
    f_number: Option<f64>,
    exposure_time: Option<String>,
    focal_length: Option<f64>,
    flash: Option<String>,
    white_balance: Option<String>,
    exposure_program: Option<String>,
    metering_mode: Option<String>,
    orientation: Option<i64>,
    software: Option<String>,
    artist: Option<String>,
    copyright: Option<String>,
    description: Option<String>,
    user_comment: Option<String>,
    color_space: Option<String>,
    exposure_bias: Option<f64>,
    scene_type: Option<String>,
    digital_zoom: Option<f64>,
    exif_overrides: Option<std::collections::HashMap<String, String>>,
}

/// Write all metadata fields to a JPEG file's EXIF using exiftool.
fn write_exif_fields_full(
    file_path: &std::path::Path,
    fields: &ExifWriteFields,
) -> Result<(), String> {
    let mut args = vec!["-overwrite_original".to_string()];

    if let Some(ref dt) = fields.taken_at {
        let exif_dt = dt
            .replace('-', ":")
            .replace('T', " ")
            .trim_end_matches('Z')
            .to_string();
        args.push(format!("-DateTimeOriginal={}", exif_dt));
    }

    if let (Some(lat), Some(lon)) = (fields.latitude, fields.longitude) {
        let lat_ref = if lat >= 0.0 { "N" } else { "S" };
        let lon_ref = if lon >= 0.0 { "E" } else { "W" };
        args.push(format!("-GPSLatitude={}", lat.abs()));
        args.push(format!("-GPSLatitudeRef={}", lat_ref));
        args.push(format!("-GPSLongitude={}", lon.abs()));
        args.push(format!("-GPSLongitudeRef={}", lon_ref));
    }

    macro_rules! push_str_tag {
        ($field:expr, $tag:expr) => {
            if let Some(ref val) = $field {
                if !val.is_empty() {
                    args.push(format!("-{}={}", $tag, val));
                }
            }
        };
    }
    macro_rules! push_num_tag {
        ($field:expr, $tag:expr) => {
            if let Some(val) = $field {
                args.push(format!("-{}={}", $tag, val));
            }
        };
    }

    push_str_tag!(fields.camera_model, "Model");
    push_str_tag!(fields.camera_make, "Make");
    push_str_tag!(fields.lens_model, "LensModel");
    push_num_tag!(fields.iso_speed, "ISO");
    push_num_tag!(fields.f_number, "FNumber");
    push_str_tag!(fields.exposure_time, "ExposureTime");
    push_num_tag!(fields.focal_length, "FocalLength");
    push_str_tag!(fields.flash, "Flash");
    push_str_tag!(fields.white_balance, "WhiteBalance");
    push_str_tag!(fields.exposure_program, "ExposureProgram");
    push_str_tag!(fields.metering_mode, "MeteringMode");
    push_num_tag!(fields.orientation, "Orientation");
    push_str_tag!(fields.software, "Software");
    push_str_tag!(fields.artist, "Artist");
    push_str_tag!(fields.copyright, "Copyright");
    push_str_tag!(fields.description, "ImageDescription");
    push_str_tag!(fields.user_comment, "UserComment");
    push_str_tag!(fields.color_space, "ColorSpace");
    push_num_tag!(fields.exposure_bias, "ExposureCompensation");
    push_str_tag!(fields.scene_type, "SceneCaptureType");
    push_num_tag!(fields.digital_zoom, "DigitalZoomRatio");

    // Apply arbitrary overrides
    if let Some(ref overrides) = fields.exif_overrides {
        for (tag, val) in overrides {
            // Sanitise tag name: only allow alphanumeric + underscore
            let clean_tag: String = tag.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !clean_tag.is_empty() && !val.is_empty() {
                args.push(format!("-{}={}", clean_tag, val));
            }
        }
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
