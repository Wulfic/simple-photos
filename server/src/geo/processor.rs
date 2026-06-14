//! Background geo-processing pipeline.
//!
//! On startup (when geo is enabled), backfills `geo_city`, `geo_state`,
//! `geo_country`, `geo_country_code`, `photo_year`, and `photo_month`
//! for photos that have GPS coordinates or timestamps but haven't been
//! resolved yet.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sqlx::SqlitePool;

use super::geocoder::ReverseGeocoder;
use super::precise::{self, PreciseGeocoder};
use crate::config::GeoConfig;

/// Spawn the background geo-processor task.
///
/// Always spawns regardless of `config.enabled`. The processor checks
/// per-user `geo_enabled` settings each cycle, using `config.enabled`
/// as the default for users who haven't explicitly toggled. This allows
/// the runtime toggle (`POST /api/settings/geo`) to work correctly.
///
/// **Lazy loading.** The 25 MB GeoNames dataset is *not* parsed until the
/// first poll cycle that actually has work to do (geo enabled by config OR
/// at least one user with `geo_enabled='true'` AND at least one photo
/// pending resolution).  This prevents the ~1-3 s CPU spike + ~30 MB RAM
/// hit at boot on small CPU-only VPS instances where geo is disabled.
///
/// Once loaded the dataset is held for the lifetime of the process — there
/// is no benefit to repeatedly re-parsing it.
///
/// This task:
/// 1. Sleeps until the dataset is needed (lazy)
/// 2. Loads the GeoNames dataset (blocking I/O in spawn_blocking)
/// 3. Backfills geo columns for photos with lat/lon but no geo_city
/// 4. Backfills photo_year/photo_month for photos with taken_at but no year
/// 5. Sleeps and re-checks periodically for newly uploaded photos
pub fn spawn_geo_processor(pool: SqlitePool, config: GeoConfig, active: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let batch_size = config.batch_size as i64;
        let poll_secs = config.poll_interval_secs.max(1);

        // Cached dataset — loaded once on first need.
        let mut geocoder: Option<Arc<ReverseGeocoder>> = None;
        // Online precise geocoder — built once on first need (opt-in users only).
        let mut precise_geocoder: Option<Arc<PreciseGeocoder>> = None;

        // Process loop — `tokio::time::interval` ticks immediately on the
        // first call so the initial backfill runs at startup.  Subsequent
        // ticks fire every `poll_interval_secs` (default 5 minutes; tests
        // shorten this so newly uploaded photos are picked up promptly).
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(poll_secs));
        loop {
            interval.tick().await;

            // Mark geo as active while we run a backfill cycle so the web
            // client can spin the profile avatar indicator.
            active.store(true, Ordering::Relaxed);

            // ── Cheap, always-runs phase: year/month backfill ───────────
            // This is one SQL UPDATE; runs whether or not geo is enabled
            // because timeline albums use it independently of geocoding.
            if let Err(e) = backfill_year_month(&pool, batch_size).await {
                tracing::warn!(error = %e, "Year/month backfill cycle failed");
            }

            // ── Decide whether we need the geocoder this cycle ──────────
            // Skip the expensive dataset load on hosts where nobody wants
            // geocoding.  We re-check every cycle so flipping the toggle
            // at runtime brings the loader online without a restart.
            let needs_geo = geo_resolution_needed(&pool, config.enabled).await;

            if needs_geo {
                // Lazy-load on first need
                if geocoder.is_none() {
                    geocoder = load_geocoder(&config).await;
                }

                if let Some(gc) = &geocoder {
                    if gc.is_loaded() {
                        if let Err(e) = backfill_geo_locations(&pool, gc, batch_size, &config).await
                        {
                            tracing::warn!(error = %e, "Geo backfill cycle failed");
                        }
                    }
                }
            }

            // ── Opt-in precise (street-level) enrichment ────────────────
            // Runs only for users who explicitly enabled it, and only after
            // coarse city resolution.  Network-bound and rate-limited, so it
            // is deliberately the last thing each cycle.
            if precise_resolution_needed(&pool).await {
                if precise_geocoder.is_none() {
                    match PreciseGeocoder::new(config.clone()) {
                        Ok(pg) => precise_geocoder = Some(Arc::new(pg)),
                        Err(e) => tracing::error!(error = %e, "failed to init precise geocoder"),
                    }
                }
                if let Some(pg) = &precise_geocoder {
                    if let Err(e) = backfill_precise_addresses(&pool, pg, &config).await {
                        tracing::warn!(error = %e, "Precise geo backfill cycle failed");
                    }
                }
            }

            active.store(false, Ordering::Relaxed);
        }
    });
}

/// Decide whether at least one photo waiting for geo resolution belongs
/// to a user (or default policy) that wants geocoding.  Cheap: a single
/// `EXISTS` query.
async fn geo_resolution_needed(pool: &SqlitePool, config_default_enabled: bool) -> bool {
    let default_flag = if config_default_enabled { 1i32 } else { 0i32 };
    sqlx::query_scalar::<_, i32>(
        "SELECT EXISTS( \
           SELECT 1 FROM photos p \
           WHERE p.latitude IS NOT NULL AND p.longitude IS NOT NULL \
             AND p.geo_city IS NULL \
             AND ( \
               EXISTS (SELECT 1 FROM user_settings us \
                       WHERE us.user_id = p.user_id \
                         AND us.key = 'geo_enabled' AND us.value = 'true') \
               OR ( \
                 ?1 = 1 AND NOT EXISTS ( \
                   SELECT 1 FROM user_settings us \
                   WHERE us.user_id = p.user_id AND us.key = 'geo_enabled') \
               ) \
             ) \
           LIMIT 1)",
    )
    .bind(default_flag)
    .fetch_one(pool)
    .await
    .map(|n| n != 0)
    .unwrap_or(false)
}

/// Load the GeoNames dataset off the runtime thread.  Returns `None` if
/// the file is missing or fails to parse — callers must treat that as
/// "geocoding unavailable" and try again on the next cycle.
///
/// Returning `None` (instead of caching an empty geocoder) is what makes
/// the retry actually happen: the poll loop only loads while its cached
/// slot `is_none()`, so a `Some(empty)` here used to freeze geocoding
/// until restart even after the operator installed the dataset.
async fn load_geocoder(config: &GeoConfig) -> Option<Arc<ReverseGeocoder>> {
    let dataset_path = config.dataset_path.clone();
    match tokio::task::spawn_blocking(move || {
        let path = std::path::Path::new(&dataset_path);
        if path.exists() {
            ReverseGeocoder::load(path).map(Some)
        } else {
            tracing::error!(
                path = %dataset_path,
                cwd = ?std::env::current_dir().ok(),
                "GeoNames cities500.txt not found — reverse geocoding is \
                 unavailable until the file appears (re-checked every poll \
                 cycle).  Note the path resolves relative to the server's \
                 working directory.  The installer downloads it; or fetch \
                 https://download.geonames.org/export/dump/cities500.zip \
                 manually"
            );
            Ok(None)
        }
    })
    .await
    {
        Ok(Ok(Some(gc))) => Some(Arc::new(gc)),
        Ok(Ok(None)) => None,
        Ok(Err(e)) => {
            tracing::error!(error = %e, "Failed to load GeoNames dataset");
            None
        }
        Err(e) => {
            tracing::error!(error = %e, "Geo dataset loading task panicked");
            None
        }
    }
}

/// Backfill geo_city/geo_state/geo_country/geo_country_code for photos
/// that have latitude/longitude but no resolved city yet.
/// Only processes photos for users who have geo enabled (per-user setting
/// or config default).
async fn backfill_geo_locations(
    pool: &SqlitePool,
    geocoder: &Arc<ReverseGeocoder>,
    batch_size: i64,
    config: &GeoConfig,
) -> Result<(), sqlx::Error> {
    let config_default_enabled = if config.enabled { 1i32 } else { 0i32 };

    loop {
        // Fetch a batch of un-resolved photos for users who have geo enabled
        let rows: Vec<(String, f64, f64)> = sqlx::query_as(
            "SELECT p.id, p.latitude, p.longitude FROM photos p \
             WHERE p.latitude IS NOT NULL AND p.longitude IS NOT NULL \
             AND p.geo_city IS NULL \
             AND ( \
                 EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'geo_enabled' AND us.value = 'true') \
                 OR ( \
                     ?2 = 1 AND NOT EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'geo_enabled') \
                 ) \
             ) \
             LIMIT ?1"
        )
        .bind(batch_size)
        .bind(config_default_enabled)
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            break;
        }

        let count = rows.len();

        // Resolve all coordinates
        let coords: Vec<(f64, f64)> = rows.iter().map(|(_, lat, lon)| (*lat, *lon)).collect();
        let gc = Arc::clone(geocoder);
        let locations = tokio::task::spawn_blocking(move || gc.lookup_batch(&coords))
            .await
            .unwrap_or_default();

        // Update each photo with its resolved location, OR mark it as
        // "attempted but unresolved" using an empty-string sentinel.
        //
        // Without the sentinel, photos whose GPS coordinates don't match
        // any city in the offline cities500 dataset (open ocean, remote
        // wilderness, bogus 0,0 EXIF) stay `geo_city IS NULL` forever and
        // the activity-status counter sticks at e.g. "23/31" because each
        // backfill cycle re-fetches and re-fails to resolve the same rows.
        // The empty-string sentinel exits them from the pending pool so
        // progress reaches 31/31. Operators can re-run resolution by
        // clearing the empty values: `UPDATE photos SET geo_city = NULL
        // WHERE geo_city = ''`.
        let mut resolved = 0usize;
        let mut unresolved = 0usize;
        // Wrap per-row UPDATEs in transactions so SQLite doesn't fsync
        // between every row — but commit in small chunks so the
        // activity-status reader pool sees progress mid-batch.
        // Without chunked commits the GeoBanner would jump from 0/N to
        // N/N because all writes were invisible to readers until the
        // single end-of-batch commit.
        const COMMIT_CHUNK: usize = 10;
        let mut tx = pool.begin().await?;
        let mut in_chunk = 0usize;
        for (i, (photo_id, _, _)) in rows.iter().enumerate() {
            match locations.get(i) {
                Some(Some(loc)) => {
                    sqlx::query(
                        "UPDATE photos SET geo_city = ?1, geo_state = ?2, \
                         geo_country = ?3, geo_country_code = ?4 \
                         WHERE id = ?5",
                    )
                    .bind(&loc.city)
                    .bind(&loc.state)
                    .bind(&loc.country)
                    .bind(&loc.country_code)
                    .bind(photo_id)
                    .execute(&mut *tx)
                    .await?;
                    resolved += 1;
                }
                _ => {
                    sqlx::query(
                        "UPDATE photos SET geo_city = '' \
                         WHERE id = ?1 AND geo_city IS NULL",
                    )
                    .bind(photo_id)
                    .execute(&mut *tx)
                    .await?;
                    unresolved += 1;
                }
            }
            in_chunk += 1;
            if in_chunk >= COMMIT_CHUNK {
                tx.commit().await?;
                tx = pool.begin().await?;
                in_chunk = 0;
            }
        }
        if in_chunk > 0 {
            tx.commit().await?;
        } else {
            // Nothing pending — drop the empty transaction.
            drop(tx);
        }

        tracing::info!(
            photos = count,
            resolved,
            unresolved,
            "Backfilled geo locations"
        );

        // If we got a full batch, there may be more
        if (count as i64) < batch_size {
            break;
        }
    }

    Ok(())
}

/// Cheap `EXISTS` probe: is there at least one coarse-resolved photo awaiting
/// precise resolution that belongs to a user who opted into precise geocoding?
async fn precise_resolution_needed(pool: &SqlitePool) -> bool {
    sqlx::query_scalar::<_, i32>(
        "SELECT EXISTS( \
           SELECT 1 FROM photos p \
           WHERE p.latitude IS NOT NULL AND p.longitude IS NOT NULL \
             AND p.geo_city IS NOT NULL AND p.geo_city != '' \
             AND p.geo_precise_status IS NULL \
             AND EXISTS (SELECT 1 FROM user_settings us \
                         WHERE us.user_id = p.user_id \
                           AND us.key = 'geo_precise_enabled' AND us.value = 'true') \
           LIMIT 1)",
    )
    .fetch_one(pool)
    .await
    .map(|n| n != 0)
    .unwrap_or(false)
}

/// Resolve street-level addresses for opt-in users via the online geocoder.
///
/// One bounded batch per cycle (network + rate-limited).  Each coordinate is
/// served from the dedup cache when possible.  A transient failure (offline,
/// rate-limited, daily cap) stops the batch early and leaves the remaining
/// photos pending for the next cycle — they are never poisoned.
async fn backfill_precise_addresses(
    pool: &SqlitePool,
    geocoder: &Arc<PreciseGeocoder>,
    config: &GeoConfig,
) -> Result<(), sqlx::Error> {
    let rows: Vec<(String, f64, f64)> = sqlx::query_as(
        "SELECT p.id, p.latitude, p.longitude FROM photos p \
         WHERE p.latitude IS NOT NULL AND p.longitude IS NOT NULL \
           AND p.geo_city IS NOT NULL AND p.geo_city != '' \
           AND p.geo_precise_status IS NULL \
           AND EXISTS (SELECT 1 FROM user_settings us \
                       WHERE us.user_id = p.user_id \
                         AND us.key = 'geo_precise_enabled' AND us.value = 'true') \
         LIMIT ?1",
    )
    .bind(config.batch_size as i64)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    let mut resolved = 0usize;
    let mut empty = 0usize;
    for (photo_id, lat, lon) in &rows {
        // Dedup cache first — repeated locations never hit the network.
        if let Some(addr) = precise::cache_get(pool, *lat, *lon).await {
            write_precise(pool, photo_id, &addr).await?;
            resolved += 1;
            continue;
        }
        match geocoder.reverse(*lat, *lon).await {
            Ok(addr) if !addr.is_empty() => {
                let _ = precise::cache_put(pool, *lat, *lon, &addr, &config.precise_provider).await;
                write_precise(pool, photo_id, &addr).await?;
                resolved += 1;
            }
            Ok(_) => {
                // Provider has no address for this spot — mark attempted so we
                // don't retry forever (mirrors the geo_city '' sentinel).
                sqlx::query("UPDATE photos SET geo_precise_status = '' WHERE id = ?1")
                    .bind(photo_id)
                    .execute(pool)
                    .await?;
                empty += 1;
            }
            Err(e) => {
                tracing::info!(error = %e, "precise geo lookup deferred; retrying next cycle");
                break;
            }
        }
    }

    tracing::info!(resolved, empty, "Backfilled precise addresses");
    Ok(())
}

/// Persist a resolved precise address onto a photo row.
async fn write_precise(
    pool: &SqlitePool,
    photo_id: &str,
    addr: &precise::PreciseAddress,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE photos SET geo_street = ?1, geo_house_number = ?2, geo_address = ?3, \
         geo_precise_status = 'ok' WHERE id = ?4",
    )
    .bind(&addr.street)
    .bind(&addr.house_number)
    .bind(addr.label())
    .bind(photo_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Backfill photo_year and photo_month from taken_at for photos that
/// have a timestamp but no year cached yet.
async fn backfill_year_month(pool: &SqlitePool, batch_size: i64) -> Result<(), sqlx::Error> {
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
///
/// Note: Currently the geocoder is only available inside the background
/// geo-processor task.  This function is ready for use once the geocoder
/// is exposed via AppState.  Until then, newly uploaded photos are
/// geo-resolved by the periodic backfill cycle (every 5 minutes).
#[allow(dead_code)]
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
             WHERE id = ?5",
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
         WHERE id = ?2",
    )
    .bind(taken_at)
    .bind(photo_id)
    .execute(pool)
    .await?;
    Ok(())
}
