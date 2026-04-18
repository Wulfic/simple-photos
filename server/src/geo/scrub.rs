//! Geolocation scrubbing — strip GPS data from photos.
//!
//! Two modes:
//! 1. On-upload: null out lat/lon before DB insert (no file modification needed
//!    since we just don't store the coordinates)
//! 2. Retroactive: null out all geo columns for a user's existing photos

use sqlx::SqlitePool;

/// Scrub all geolocation data for a user's photos.
///
/// Sets latitude, longitude, geo_city, geo_state, geo_country, and
/// geo_country_code to NULL for all of the user's photos.
/// Returns the number of photos affected.
pub async fn scrub_geolocation_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE photos SET \
         latitude = NULL, longitude = NULL, \
         geo_city = NULL, geo_state = NULL, \
         geo_country = NULL, geo_country_code = NULL \
         WHERE user_id = ?1 AND (latitude IS NOT NULL OR geo_city IS NOT NULL)"
    )
    .bind(user_id)
    .execute(pool)
    .await?;

    let count = result.rows_affected();
    if count > 0 {
        tracing::info!(user_id = %user_id, photos = count, "Scrubbed geolocation data");
    }
    Ok(count)
}

/// Check if geo scrubbing is enabled for a user.
pub async fn is_scrub_enabled(pool: &SqlitePool, user_id: &str) -> bool {
    let result: Option<(String,)> = sqlx::query_as(
        "SELECT value FROM user_settings WHERE user_id = ?1 AND key = 'geo_scrub_on_upload'"
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);

    matches!(result, Some((ref v,)) if v == "true")
}
