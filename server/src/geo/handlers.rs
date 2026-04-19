//! HTTP handlers for geolocation & timeline endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

// ── Response types ───────────────────────────────────────────────────

#[derive(Serialize)]
pub struct GeoStatusResponse {
    pub enabled: bool,
    pub scrub_on_upload: bool,
    pub photos_with_location: i64,
    pub photos_without_location: i64,
    pub unique_countries: i64,
    pub unique_cities: i64,
}

#[derive(Serialize)]
pub struct LocationEntry {
    pub city: String,
    pub state: Option<String>,
    pub country: String,
    pub country_code: String,
    pub photo_count: i64,
}

#[derive(Serialize)]
pub struct CountryEntry {
    pub country: String,
    pub country_code: String,
    pub photo_count: i64,
}

#[derive(Serialize)]
pub struct TimelineYearEntry {
    pub year: i64,
    pub photo_count: i64,
}

#[derive(Serialize)]
pub struct TimelineMonthEntry {
    pub year: i64,
    pub month: i64,
    pub photo_count: i64,
}

#[derive(Serialize)]
pub struct PhotoSummary {
    pub id: String,
    pub filename: String,
    pub thumb_path: Option<String>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Deserialize)]
pub struct GeoSettingsRequest {
    pub enabled: Option<bool>,
    pub scrub_on_upload: Option<bool>,
}

#[derive(Deserialize)]
pub struct ScrubConfirmRequest {
    pub confirm: bool,
}

// ── Settings ─────────────────────────────────────────────────────────

/// GET /api/settings/geo — current geo settings for this user.
pub async fn get_geo_settings(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GeoStatusResponse>, AppError> {
    let enabled = get_user_setting(&state.pool, &auth.user_id, "geo_enabled")
        .await
        .map(|v| v == "true")
        .unwrap_or(state.config.geo.enabled);

    let scrub = get_user_setting(&state.pool, &auth.user_id, "geo_scrub_on_upload")
        .await
        .map(|v| v == "true")
        .unwrap_or(false);

    let with_loc: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM photos WHERE user_id = ?1 AND latitude IS NOT NULL"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let without_loc: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM photos WHERE user_id = ?1 AND latitude IS NULL"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let countries: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT geo_country) FROM photos WHERE user_id = ?1 AND geo_country IS NOT NULL"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let cities: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT geo_city || ',' || geo_country_code) FROM photos \
         WHERE user_id = ?1 AND geo_city IS NOT NULL"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(GeoStatusResponse {
        enabled,
        scrub_on_upload: scrub,
        photos_with_location: with_loc.0,
        photos_without_location: without_loc.0,
        unique_countries: countries.0,
        unique_cities: cities.0,
    }))
}

/// POST /api/settings/geo — update geo settings for this user.
pub async fn update_geo_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<GeoSettingsRequest>,
) -> Result<StatusCode, AppError> {
    if let Some(enabled) = body.enabled {
        upsert_user_setting(&state.pool, &auth.user_id, "geo_enabled", if enabled { "true" } else { "false" }).await?;
    }
    if let Some(scrub) = body.scrub_on_upload {
        upsert_user_setting(&state.pool, &auth.user_id, "geo_scrub_on_upload", if scrub { "true" } else { "false" }).await?;
    }
    Ok(StatusCode::OK)
}

// ── Location endpoints ──────────────────────────────────────────────

/// GET /api/geo/locations — all unique locations with photo counts.
pub async fn list_locations(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<LocationEntry>>, AppError> {
    let rows: Vec<(String, Option<String>, String, String, i64)> = sqlx::query_as(
        "SELECT geo_city, geo_state, geo_country, geo_country_code, COUNT(*) as cnt \
         FROM photos WHERE user_id = ?1 AND geo_city IS NOT NULL \
         GROUP BY geo_city, geo_state, geo_country, geo_country_code \
         ORDER BY cnt DESC"
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let entries = rows.into_iter().map(|(city, state, country, code, count)| LocationEntry {
        city, state, country, country_code: code, photo_count: count,
    }).collect();

    Ok(Json(entries))
}

/// GET /api/geo/locations/:country/:city — photos from a specific location.
pub async fn list_location_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((country, city)): Path<(String, String)>,
) -> Result<Json<Vec<PhotoSummary>>, AppError> {
    let rows: Vec<(String, String, Option<String>, Option<String>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, filename, thumb_path, taken_at, latitude, longitude \
         FROM photos WHERE user_id = ?1 AND geo_country_code = ?2 AND geo_city = ?3 \
         ORDER BY taken_at DESC"
    )
    .bind(&auth.user_id)
    .bind(&country)
    .bind(&city)
    .fetch_all(&state.pool)
    .await?;

    let photos = rows.into_iter().map(|(id, filename, thumb, taken, lat, lon)| PhotoSummary {
        id, filename, thumb_path: thumb, taken_at: taken, latitude: lat, longitude: lon,
    }).collect();

    Ok(Json(photos))
}

/// GET /api/geo/countries — countries with photo counts.
pub async fn list_countries(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<CountryEntry>>, AppError> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT geo_country, geo_country_code, COUNT(*) as cnt \
         FROM photos WHERE user_id = ?1 AND geo_country IS NOT NULL \
         GROUP BY geo_country, geo_country_code \
         ORDER BY cnt DESC"
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let entries = rows.into_iter().map(|(country, code, count)| CountryEntry {
        country, country_code: code, photo_count: count,
    }).collect();

    Ok(Json(entries))
}

/// GET /api/geo/map — photos with coordinates for map display.
pub async fn list_map_photos(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<PhotoSummary>>, AppError> {
    let rows: Vec<(String, String, Option<String>, Option<String>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, filename, thumb_path, taken_at, latitude, longitude \
         FROM photos WHERE user_id = ?1 AND latitude IS NOT NULL AND longitude IS NOT NULL \
         ORDER BY taken_at DESC"
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let photos = rows.into_iter().map(|(id, filename, thumb, taken, lat, lon)| PhotoSummary {
        id, filename, thumb_path: thumb, taken_at: taken, latitude: lat, longitude: lon,
    }).collect();

    Ok(Json(photos))
}

// ── Timeline endpoints ──────────────────────────────────────────────

/// GET /api/geo/timeline — photos grouped by year.
pub async fn list_timeline(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<TimelineYearEntry>>, AppError> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT photo_year, COUNT(*) FROM photos \
         WHERE user_id = ?1 AND photo_year IS NOT NULL \
         GROUP BY photo_year ORDER BY photo_year DESC"
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let entries = rows.into_iter().map(|(year, count)| TimelineYearEntry {
        year, photo_count: count,
    }).collect();

    Ok(Json(entries))
}

/// GET /api/geo/timeline/:year — months within a year with photo counts.
pub async fn list_timeline_year(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(year): Path<i64>,
) -> Result<Json<Vec<TimelineMonthEntry>>, AppError> {
    let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT photo_year, photo_month, COUNT(*) FROM photos \
         WHERE user_id = ?1 AND photo_year = ?2 AND photo_month IS NOT NULL \
         GROUP BY photo_year, photo_month ORDER BY photo_month"
    )
    .bind(&auth.user_id)
    .bind(year)
    .fetch_all(&state.pool)
    .await?;

    let entries = rows.into_iter().map(|(y, m, count)| TimelineMonthEntry {
        year: y, month: m, photo_count: count,
    }).collect();

    Ok(Json(entries))
}

/// GET /api/geo/timeline/:year/:month — photos from a specific month.
pub async fn list_timeline_month_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((year, month)): Path<(i64, i64)>,
) -> Result<Json<Vec<PhotoSummary>>, AppError> {
    let rows: Vec<(String, String, Option<String>, Option<String>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, filename, thumb_path, taken_at, latitude, longitude \
         FROM photos WHERE user_id = ?1 AND photo_year = ?2 AND photo_month = ?3 \
         ORDER BY taken_at DESC"
    )
    .bind(&auth.user_id)
    .bind(year)
    .bind(month)
    .fetch_all(&state.pool)
    .await?;

    let photos = rows.into_iter().map(|(id, filename, thumb, taken, lat, lon)| PhotoSummary {
        id, filename, thumb_path: thumb, taken_at: taken, latitude: lat, longitude: lon,
    }).collect();

    Ok(Json(photos))
}

// ── Memories (auto-generated photo clusters by location + date) ──────

/// A Memory represents a group of photos taken at a specific location
/// during a contiguous period (same day or within a few days).
#[derive(Serialize)]
pub struct Memory {
    pub id: String,
    pub name: String,
    pub city: String,
    pub country: String,
    pub date_label: String,
    pub photo_count: i64,
    pub first_photo_id: Option<String>,
    pub first_thumb_path: Option<String>,
}

/// GET /api/geo/memories — auto-generated smart albums from location + date clusters.
///
/// Groups photos that share the same city and were taken on the same date.
/// Returns them sorted by most recent first with descriptive names.
pub async fn list_memories(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<Memory>>, AppError> {
    // Group photos by city + date, returning the count, first photo ID, and thumb path
    let rows: Vec<(String, String, String, String, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT geo_city, geo_country, geo_country_code, DATE(taken_at) as photo_date, \
                COUNT(*) as cnt, \
                MIN(id) as first_id, \
                (SELECT thumb_path FROM photos p2 WHERE p2.user_id = photos.user_id \
                 AND p2.geo_city = photos.geo_city AND DATE(p2.taken_at) = DATE(photos.taken_at) \
                 AND p2.thumb_path IS NOT NULL LIMIT 1) as thumb \
         FROM photos \
         WHERE user_id = ?1 AND geo_city IS NOT NULL AND taken_at IS NOT NULL \
         GROUP BY geo_city, geo_country, DATE(taken_at) \
         HAVING cnt >= 3 \
         ORDER BY photo_date DESC \
         LIMIT 100"
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let memories = rows.into_iter().map(|(city, country, _code, date_str, count, first_id, thumb)| {
        // Format a human-readable date label
        let date_label = format_memory_date(&date_str);
        let name = format!("{} on {}", city, date_label);
        let id = format!("{}_{}", city.to_lowercase().replace(' ', "-"), date_str);

        Memory {
            id,
            name,
            city,
            country,
            date_label,
            photo_count: count,
            first_photo_id: first_id,
            first_thumb_path: thumb,
        }
    }).collect();

    Ok(Json(memories))
}

/// GET /api/geo/memories/:id/photos — photos in a specific memory cluster.
pub async fn list_memory_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(memory_id): Path<String>,
) -> Result<Json<Vec<PhotoSummary>>, AppError> {
    // Parse memory_id back into city + date components
    // Format: "city-name_YYYY-MM-DD"
    let parts: Vec<&str> = memory_id.rsplitn(2, '_').collect();
    if parts.len() != 2 {
        return Err(AppError::BadRequest("Invalid memory ID format".into()));
    }
    let date_str = parts[0];
    let city_slug = parts[1];

    // Look up the actual city name by matching the slug
    let city_row: Option<(String,)> = sqlx::query_as(
        "SELECT DISTINCT geo_city FROM photos \
         WHERE user_id = ?1 AND geo_city IS NOT NULL AND \
               LOWER(REPLACE(geo_city, ' ', '-')) = ?2 \
         LIMIT 1"
    )
    .bind(&auth.user_id)
    .bind(city_slug)
    .fetch_optional(&state.pool)
    .await?;

    let city = city_row
        .map(|(c,)| c)
        .ok_or_else(|| AppError::NotFound)?;

    let rows: Vec<(String, String, Option<String>, Option<String>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, filename, thumb_path, taken_at, latitude, longitude \
         FROM photos WHERE user_id = ?1 AND geo_city = ?2 AND DATE(taken_at) = ?3 \
         ORDER BY taken_at ASC"
    )
    .bind(&auth.user_id)
    .bind(&city)
    .bind(date_str)
    .fetch_all(&state.pool)
    .await?;

    if rows.is_empty() {
        return Err(AppError::NotFound);
    }

    let photos = rows.into_iter().map(|(id, filename, thumb, taken, lat, lon)| PhotoSummary {
        id, filename, thumb_path: thumb, taken_at: taken, latitude: lat, longitude: lon,
    }).collect();

    Ok(Json(photos))
}

/// Format a date string (YYYY-MM-DD) into a readable label like "Jun 6, 2025".
fn format_memory_date(date_str: &str) -> String {
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() != 3 {
        return date_str.to_string();
    }
    let month = match parts[1] {
        "01" => "Jan", "02" => "Feb", "03" => "Mar", "04" => "Apr",
        "05" => "May", "06" => "Jun", "07" => "Jul", "08" => "Aug",
        "09" => "Sep", "10" => "Oct", "11" => "Nov", "12" => "Dec",
        _ => parts[1],
    };
    let day = parts[2].trim_start_matches('0');
    format!("{} {}, {}", month, day, parts[0])
}

// ── Scrub ────────────────────────────────────────────────────────────

/// POST /api/geo/scrub — scrub all geolocation data for this user.
pub async fn scrub_geo_data(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ScrubConfirmRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !body.confirm {
        return Err(AppError::BadRequest("Must confirm scrub with {\"confirm\": true}".into()));
    }

    let count = super::scrub::scrub_geolocation_for_user(&state.pool, &auth.user_id).await
        .map_err(|e| AppError::Internal(format!("Scrub failed: {}", e)))?;

    Ok(Json(serde_json::json!({
        "scrubbed_photos": count,
    })))
}

// ── Helpers ──────────────────────────────────────────────────────────

async fn get_user_setting(pool: &SqlitePool, user_id: &str, key: &str) -> Option<String> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT value FROM user_settings WHERE user_id = ?1 AND key = ?2"
    )
    .bind(user_id)
    .bind(key)
    .fetch_optional(pool)
    .await
    .ok()?;
    row.map(|(v,)| v)
}

async fn upsert_user_setting(
    pool: &SqlitePool,
    user_id: &str,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_settings (user_id, key, value, updated_at) \
         VALUES (?1, ?2, ?3, datetime('now')) \
         ON CONFLICT(user_id, key) DO UPDATE SET value = ?3, updated_at = datetime('now')"
    )
    .bind(user_id)
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}
