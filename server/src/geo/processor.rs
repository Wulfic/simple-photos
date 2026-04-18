//! Background geo-processing pipeline.
//!
//! On startup (when geo is enabled), backfills `geo_city`, `geo_state`,
//! `geo_country`, `geo_country_code`, `photo_year`, and `photo_month`
//! for photos that have GPS coordinates or timestamps but haven't been
//! resolved yet.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::config::GeoConfig;
use super::geocoder::ReverseGeocoder;

/// Spawn the background geo-processor task.
///
/// This task:
/// 1. Loads the GeoNames dataset (blocking I/O in spawn_blocking)
/// 2. Backfills geo columns for photos with lat/lon but no geo_city
/// 3. Backfills photo_year/photo_month for photos with taken_at but no year
/// 4. Sleeps and re-checks periodically for newly uploaded photos
pub fn spawn_geo_processor(pool: SqlitePool, config: GeoConfig) {
    if !config.enabled {
        tracing::info!("Geo processing disabled in config");
        return;
    }

    tokio::spawn(async move {
        // Load the dataset in a blocking task (file I/O + parsing)
        let dataset_path = config.dataset_path.clone();
        let geocoder = match tokio::task::spawn_blocking(move || {
            let path = std::path::Path::new(&dataset_path);
            if path.exists() {
                ReverseGeocoder::load(path)
            } else {
                tracing::warn!(path = %dataset_path, "GeoNames dataset not found — geo-resolution disabled");
                Ok(ReverseGeocoder::empty())
            }
        })
        .await
        {
            Ok(Ok(gc)) => Arc::new(gc),
            Ok(Err(e)) => {
                tracing::error!(error = %e, "Failed to load GeoNames dataset");
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "Geo dataset loading task panicked");
                return;
            }
        };

        tracing::info!(
            cities = geocoder.city_count(),
            "Geo processor started"
        );

        let batch_size = config.batch_size as i64;

        // Process loop — runs once then every 5 minutes to catch new uploads
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;

            // ── Backfill geo location from lat/lon ──────────────────────
            if geocoder.is_loaded() {
                if let Err(e) = backfill_geo_locations(&pool, &geocoder, batch_size).await {
                    tracing::warn!(error = %e, "Geo backfill cycle failed");
                }
            }

            // ── Backfill photo_year / photo_month from taken_at ─────────
            if let Err(e) = backfill_year_month(&pool, batch_size).await {
                tracing::warn!(error = %e, "Year/month backfill cycle failed");
            }
        }
    });
}

/// Backfill geo_city/geo_state/geo_country/geo_country_code for photos
/// that have latitude/longitude but no resolved city yet.
async fn backfill_geo_locations(
    pool: &SqlitePool,
    geocoder: &Arc<ReverseGeocoder>,
    batch_size: i64,
) -> Result<(), sqlx::Error> {
    loop {
        // Fetch a batch of un-resolved photos
        let rows: Vec<(String, f64, f64)> = sqlx::query_as(
            "SELECT id, latitude, longitude FROM photos \
             WHERE latitude IS NOT NULL AND longitude IS NOT NULL \
             AND geo_city IS NULL \
             LIMIT ?1"
        )
        .bind(batch_size)
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            break;
        }

        let count = rows.len();

        // Resolve all coordinates
        let coords: Vec<(f64, f64)> = rows.iter().map(|(_, lat, lon)| (*lat, *lon)).collect();
        let gc = Arc::clone(geocoder);
        let locations = tokio::task::spawn_blocking(move || {
            gc.lookup_batch(&coords)
        })
        .await
        .unwrap_or_default();

        // Update each photo with its resolved location
        for (i, (photo_id, _, _)) in rows.iter().enumerate() {
            if let Some(Some(loc)) = locations.get(i) {
                sqlx::query(
                    "UPDATE photos SET geo_city = ?1, geo_state = ?2, \
                     geo_country = ?3, geo_country_code = ?4 \
                     WHERE id = ?5"
                )
                .bind(&loc.city)
                .bind(&loc.state)
                .bind(&loc.country)
                .bind(&loc.country_code)
                .bind(photo_id)
                .execute(pool)
                .await?;
            }
        }

        tracing::info!(photos = count, "Backfilled geo locations");

        // If we got a full batch, there may be more
        if (count as i64) < batch_size {
            break;
        }
    }

    Ok(())
}

/// Backfill photo_year and photo_month from taken_at for photos that
/// have a timestamp but no year cached yet.
async fn backfill_year_month(
    pool: &SqlitePool,
    batch_size: i64,
) -> Result<(), sqlx::Error> {
    // Use SQLite date functions to extract year/month from the ISO timestamp
    let result = sqlx::query(
        "UPDATE photos SET \
         photo_year = CAST(strftime('%Y', taken_at) AS INTEGER), \
         photo_month = CAST(strftime('%m', taken_at) AS INTEGER) \
         WHERE taken_at IS NOT NULL AND photo_year IS NULL \
         AND id IN (SELECT id FROM photos WHERE taken_at IS NOT NULL AND photo_year IS NULL LIMIT ?1)"
    )
    .bind(batch_size)
    .execute(pool)
    .await?;

    let updated = result.rows_affected();
    if updated > 0 {
        tracing::info!(photos = updated, "Backfilled year/month from timestamps");
    }

    Ok(())
}

/// Resolve geo location for a single photo inline (during upload).
/// Called when a photo is inserted with GPS coordinates and geo is enabled.
pub async fn resolve_photo_geo(
    pool: &SqlitePool,
    geocoder: &ReverseGeocoder,
    photo_id: &str,
    lat: f64,
    lon: f64,
) -> Result<(), sqlx::Error> {
    if let Some(loc) = geocoder.lookup(lat, lon) {
        sqlx::query(
            "UPDATE photos SET geo_city = ?1, geo_state = ?2, \
             geo_country = ?3, geo_country_code = ?4 \
             WHERE id = ?5"
        )
        .bind(&loc.city)
        .bind(&loc.state)
        .bind(&loc.country)
        .bind(&loc.country_code)
        .bind(photo_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Set photo_year and photo_month for a single photo inline (during upload).
pub async fn set_photo_year_month(
    pool: &SqlitePool,
    photo_id: &str,
    taken_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE photos SET \
         photo_year = CAST(strftime('%Y', ?1) AS INTEGER), \
         photo_month = CAST(strftime('%m', ?1) AS INTEGER) \
         WHERE id = ?2"
    )
    .bind(taken_at)
    .bind(photo_id)
    .execute(pool)
    .await?;
    Ok(())
}
