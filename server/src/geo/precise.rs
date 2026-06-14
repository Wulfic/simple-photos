//! Opt-in online precise (street-level) reverse geocoding.
//!
//! This is the **only** part of geolocation that contacts a third party, and
//! only ever for a user who has explicitly enabled `geo_precise_enabled`.
//! Two keyless, no-registration OpenStreetMap reverse geocoders are
//! supported: Nominatim (primary) and Photon (fallback).
//!
//! ## Respecting provider usage policy
//!
//! Nominatim's public instance allows **at most 1 request/second** and
//! forbids bulk geocoding.  We stay compliant with three mechanisms:
//!
//!  1. A [`Throttle`] enforcing both a per-second rate and a per-UTC-day cap.
//!  2. A persistent, blind-indexed cache (see [`cache_get`]/[`cache_put`])
//!     so repeated coordinates never hit the network twice.
//!  3. An identifying `User-Agent` (required by the policy).
//!
//! Transient failures (offline, HTTP 429, daily cap reached) are surfaced as
//! errors so the caller leaves the photo *pending* and retries later, rather
//! than poisoning it as permanently unresolved.

use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::config::GeoConfig;

/// Decimal places coordinates are rounded to for the dedup cache key.
/// 4 dp ≈ 11 m — coarse enough that two photos from the same spot collapse
/// to one geocoder lookup, which is what keeps us under provider usage
/// limits.  The whole database is encrypted at rest (SQLCipher), so the
/// rounded coordinate may be stored as the cache key in the clear *within*
/// the encrypted file.
const CACHE_COORD_PRECISION: u32 = 4;

/// Round + render a coordinate pair as the canonical `"lat,lon"` cache key.
fn coord_key(lat: f64, lon: f64) -> String {
    let factor = 10f64.powi(CACHE_COORD_PRECISION as i32);
    let rlat = (lat * factor).round() / factor;
    let rlon = (lon * factor).round() / factor;
    format!(
        "{:.*},{:.*}",
        CACHE_COORD_PRECISION as usize, rlat, CACHE_COORD_PRECISION as usize, rlon
    )
}

/// A resolved precise address.  Stored encrypted in `photos.geo_address_enc`
/// and `geo_address_cache.payload_enc`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PreciseAddress {
    pub house_number: Option<String>,
    pub street: Option<String>,
    /// Pre-formatted human label, e.g. "86 Nelson Blvd, Springfield, Ohio".
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
}

impl PreciseAddress {
    /// True when the provider returned nothing usable — caller marks the photo
    /// "attempted but unresolved" rather than retrying forever.
    pub fn is_empty(&self) -> bool {
        self.house_number.is_none()
            && self.street.is_none()
            && self.address.is_none()
            && self.city.is_none()
    }

    /// Compose a display label, preferring house-number + street, then folding
    /// in city/state.  Falls back to whatever `address` the provider gave.
    pub fn label(&self) -> Option<String> {
        let street_part = match (&self.house_number, &self.street) {
            (Some(h), Some(r)) => Some(format!("{h} {r}")),
            (None, Some(r)) => Some(r.clone()),
            _ => None,
        };
        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = street_part {
            parts.push(s);
        }
        if let Some(c) = &self.city {
            parts.push(c.clone());
        }
        if let Some(st) = &self.state {
            parts.push(st.clone());
        }
        if parts.is_empty() {
            self.address.clone()
        } else {
            Some(parts.join(", "))
        }
    }
}

/// Why a precise lookup could not be completed *right now* (vs. genuinely
/// having no address).  All of these mean "leave it pending, try later".
#[derive(Debug)]
pub enum PreciseError {
    /// Per-UTC-day request cap reached — back off until tomorrow.
    DailyCapReached,
    /// Provider returned HTTP 429 (rate limited).
    RateLimited,
    /// Network/transport failure (likely offline).
    Offline(String),
    /// Provider returned an unexpected/parse-failing response.
    Bad(String),
}

impl std::fmt::Display for PreciseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreciseError::DailyCapReached => write!(f, "daily geocoder cap reached"),
            PreciseError::RateLimited => write!(f, "provider rate limited (429)"),
            PreciseError::Offline(e) => write!(f, "geocoder unreachable: {e}"),
            PreciseError::Bad(e) => write!(f, "geocoder bad response: {e}"),
        }
    }
}

/// Combined per-second + per-day request throttle.  Shared across the single
/// background enrichment task, so the simple "single writer" reasoning holds.
struct Throttle {
    min_interval: Duration,
    last: Mutex<Option<Instant>>,
    /// UTC day ordinal of the current counting window.
    day: AtomicI64,
    count_today: AtomicU32,
    daily_cap: u32,
}

impl Throttle {
    fn new(rate_per_sec: u32, daily_cap: u32) -> Self {
        let min_interval = if rate_per_sec == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(1.0 / rate_per_sec as f64)
        };
        Self {
            min_interval,
            last: Mutex::new(None),
            day: AtomicI64::new(Self::utc_day()),
            count_today: AtomicU32::new(0),
            daily_cap,
        }
    }

    fn utc_day() -> i64 {
        chrono::Utc::now().timestamp() / 86_400
    }

    /// Reserve one outbound request, sleeping to honour the per-second rate.
    /// Errors with [`PreciseError::DailyCapReached`] when the daily cap is hit.
    async fn acquire(&self) -> Result<(), PreciseError> {
        if self.daily_cap > 0 {
            let today = Self::utc_day();
            if self.day.swap(today, Ordering::Relaxed) != today {
                self.count_today.store(0, Ordering::Relaxed);
            }
            let prev = self.count_today.fetch_add(1, Ordering::Relaxed);
            if prev >= self.daily_cap {
                self.count_today.fetch_sub(1, Ordering::Relaxed);
                return Err(PreciseError::DailyCapReached);
            }
        }
        let mut last = self.last.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.min_interval {
                tokio::time::sleep(self.min_interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
        Ok(())
    }
}

/// HTTP-backed precise geocoder.  Build one per process and reuse it (holds
/// the connection pool and the shared throttle).
pub struct PreciseGeocoder {
    client: reqwest::Client,
    config: GeoConfig,
    throttle: Throttle,
}

impl PreciseGeocoder {
    pub fn new(config: GeoConfig) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(config.geo_user_agent.clone())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| format!("failed to build geo HTTP client: {e}"))?;
        let throttle = Throttle::new(config.precise_rate_per_sec, config.precise_daily_cap);
        Ok(Self {
            client,
            config,
            throttle,
        })
    }

    /// Reverse-geocode a coordinate to a street address.
    ///
    /// `Ok(addr)` — a result (possibly [`PreciseAddress::is_empty`] when the
    /// provider has no address for the spot).  `Err(_)` — a transient failure;
    /// the caller should leave the photo pending and retry later.
    pub async fn reverse(&self, lat: f64, lon: f64) -> Result<PreciseAddress, PreciseError> {
        match self.config.precise_provider.as_str() {
            "photon" => self.photon(lat, lon).await,
            "nominatim" => self.nominatim(lat, lon).await,
            // "auto" (and anything unrecognised): Nominatim first, Photon on a
            // hard failure.  A successful-but-empty Nominatim answer is taken
            // at face value (the spot genuinely has no address) — we do not
            // burn a second request chasing it.
            _ => match self.nominatim(lat, lon).await {
                Ok(addr) => Ok(addr),
                Err(PreciseError::DailyCapReached) => Err(PreciseError::DailyCapReached),
                Err(e) => {
                    tracing::debug!(error = %e, "nominatim failed; trying photon");
                    self.photon(lat, lon).await
                }
            },
        }
    }

    async fn nominatim(&self, lat: f64, lon: f64) -> Result<PreciseAddress, PreciseError> {
        self.throttle.acquire().await?;
        let resp = self
            .client
            .get(&self.config.nominatim_endpoint)
            .query(&[
                ("lat", lat.to_string()),
                ("lon", lon.to_string()),
                ("format", "jsonv2".into()),
                ("addressdetails", "1".into()),
                ("zoom", "18".into()),
            ])
            .send()
            .await
            .map_err(|e| PreciseError::Offline(e.to_string()))?;

        if resp.status().as_u16() == 429 {
            return Err(PreciseError::RateLimited);
        }
        if !resp.status().is_success() {
            return Err(PreciseError::Bad(format!("HTTP {}", resp.status())));
        }
        let body: NominatimResp = resp
            .json()
            .await
            .map_err(|e| PreciseError::Bad(e.to_string()))?;
        if body.error.is_some() {
            return Ok(PreciseAddress::default());
        }
        let a = body.address;
        let city = a
            .city
            .or(a.town)
            .or(a.village)
            .or(a.hamlet)
            .or(a.municipality);
        let mut addr = PreciseAddress {
            house_number: a.house_number,
            street: a.road,
            address: None,
            city,
            state: a.state,
            country: a.country,
            country_code: a.country_code.map(|c| c.to_uppercase()),
        };
        addr.address = addr.label().or(body.display_name);
        Ok(addr)
    }

    async fn photon(&self, lat: f64, lon: f64) -> Result<PreciseAddress, PreciseError> {
        self.throttle.acquire().await?;
        let resp = self
            .client
            .get(&self.config.photon_endpoint)
            .query(&[("lat", lat.to_string()), ("lon", lon.to_string())])
            .send()
            .await
            .map_err(|e| PreciseError::Offline(e.to_string()))?;

        if resp.status().as_u16() == 429 {
            return Err(PreciseError::RateLimited);
        }
        if !resp.status().is_success() {
            return Err(PreciseError::Bad(format!("HTTP {}", resp.status())));
        }
        let body: PhotonResp = resp
            .json()
            .await
            .map_err(|e| PreciseError::Bad(e.to_string()))?;
        let props = match body.features.into_iter().next() {
            Some(f) => f.properties,
            None => return Ok(PreciseAddress::default()),
        };
        let mut addr = PreciseAddress {
            house_number: props.housenumber,
            street: props.street.or(props.name),
            address: None,
            city: props.city,
            state: props.state,
            country: props.country,
            country_code: props.countrycode.map(|c| c.to_uppercase()),
        };
        addr.address = addr.label();
        Ok(addr)
    }
}

// ── Provider response shapes ─────────────────────────────────────────────

#[derive(Deserialize)]
struct NominatimResp {
    #[serde(default)]
    address: NominatimAddr,
    display_name: Option<String>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
struct NominatimAddr {
    house_number: Option<String>,
    road: Option<String>,
    city: Option<String>,
    town: Option<String>,
    village: Option<String>,
    hamlet: Option<String>,
    municipality: Option<String>,
    state: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
}

#[derive(Deserialize)]
struct PhotonResp {
    #[serde(default)]
    features: Vec<PhotonFeature>,
}

#[derive(Deserialize)]
struct PhotonFeature {
    #[serde(default)]
    properties: PhotonProps,
}

#[derive(Deserialize, Default)]
struct PhotonProps {
    housenumber: Option<String>,
    street: Option<String>,
    name: Option<String>,
    city: Option<String>,
    state: Option<String>,
    country: Option<String>,
    countrycode: Option<String>,
}

// ── Coordinate dedup cache ───────────────────────────────────────────────
// Keyed by the rounded coordinate so repeated locations (home, work) never
// hit the network twice — the single most important lever for staying under
// Nominatim's "no bulk geocoding" usage policy.  Stored plaintext within the
// SQLCipher-encrypted database file.

/// Look up a previously resolved address for these coordinates.  Returns
/// `None` on a cache miss or any decode failure (treated as a miss).
pub async fn cache_get(read_pool: &SqlitePool, lat: f64, lon: f64) -> Option<PreciseAddress> {
    let key = coord_key(lat, lon);
    let row: (String,) =
        sqlx::query_as("SELECT payload FROM geo_address_cache WHERE coord_key = ?1")
            .bind(&key)
            .fetch_optional(read_pool)
            .await
            .ok()??;
    serde_json::from_str(&row.0).ok()
}

/// Persist a resolved address under its rounded coordinate key.
pub async fn cache_put(
    pool: &SqlitePool,
    lat: f64,
    lon: f64,
    addr: &PreciseAddress,
    source: &str,
) -> Result<(), String> {
    let key = coord_key(lat, lon);
    let payload = serde_json::to_string(addr).map_err(|e| format!("geo cache encode failed: {e}"))?;
    sqlx::query(
        "INSERT INTO geo_address_cache (coord_key, payload, source, fetched_at) \
         VALUES (?1, ?2, ?3, datetime('now')) \
         ON CONFLICT(coord_key) DO UPDATE SET \
           payload = excluded.payload, source = excluded.source, fetched_at = excluded.fetched_at",
    )
    .bind(&key)
    .bind(&payload)
    .bind(source)
    .execute(pool)
    .await
    .map_err(|e| format!("geo cache write failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_prefers_house_and_street() {
        let a = PreciseAddress {
            house_number: Some("86".into()),
            street: Some("Nelson Blvd".into()),
            city: Some("Springfield".into()),
            state: Some("Ohio".into()),
            ..Default::default()
        };
        assert_eq!(a.label().as_deref(), Some("86 Nelson Blvd, Springfield, Ohio"));
    }

    #[test]
    fn label_falls_back_to_provider_address() {
        let a = PreciseAddress {
            address: Some("Somewhere remote".into()),
            ..Default::default()
        };
        assert_eq!(a.label().as_deref(), Some("Somewhere remote"));
    }

    #[test]
    fn empty_detection() {
        assert!(PreciseAddress::default().is_empty());
        assert!(!PreciseAddress {
            street: Some("Main St".into()),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn parse_nominatim_payload() {
        let json = r#"{
            "display_name": "86, Nelson Boulevard, Springfield, Ohio, USA",
            "address": {
                "house_number": "86",
                "road": "Nelson Boulevard",
                "town": "Springfield",
                "state": "Ohio",
                "country": "United States",
                "country_code": "us"
            }
        }"#;
        let resp: NominatimResp = serde_json::from_str(json).unwrap();
        let city = resp.address.city.or(resp.address.town);
        assert_eq!(city.as_deref(), Some("Springfield"));
        assert_eq!(resp.address.house_number.as_deref(), Some("86"));
        assert_eq!(resp.address.country_code.as_deref(), Some("us"));
    }

    #[test]
    fn parse_photon_payload() {
        let json = r#"{"features":[{"properties":{
            "housenumber":"86","street":"Nelson Boulevard","city":"Springfield",
            "state":"Ohio","country":"United States","countrycode":"US"}}]}"#;
        let resp: PhotonResp = serde_json::from_str(json).unwrap();
        let p = &resp.features[0].properties;
        assert_eq!(p.housenumber.as_deref(), Some("86"));
        assert_eq!(p.street.as_deref(), Some("Nelson Boulevard"));
    }

    #[test]
    fn parse_photon_empty() {
        let resp: PhotonResp = serde_json::from_str(r#"{"features":[]}"#).unwrap();
        assert!(resp.features.is_empty());
    }

    #[tokio::test]
    async fn throttle_enforces_daily_cap() {
        let t = Throttle::new(1000, 2); // high rate so we only test the cap
        assert!(t.acquire().await.is_ok());
        assert!(t.acquire().await.is_ok());
        assert!(matches!(
            t.acquire().await,
            Err(PreciseError::DailyCapReached)
        ));
    }

    #[tokio::test]
    async fn throttle_zero_cap_is_unlimited() {
        let t = Throttle::new(1000, 0);
        for _ in 0..50 {
            assert!(t.acquire().await.is_ok());
        }
    }
}
