//! Per-IP sliding-window rate limiter.
//!
//! Prevents brute-force attacks on authentication endpoints by tracking request
//! counts per IP address with automatic expiration. Uses an in-memory DashMap
//! for lock-free concurrent access.
//!
//! # Security rationale
//! - Login: 10 attempts per 60 seconds (prevents password guessing)
//! - Register: 3 per 60 seconds (prevents mass account creation)
//! - TOTP: 5 per 60 seconds (prevents code brute-force — only 1M possibilities)

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;
use serde_json::json;

/// Tracks per-IP request counts within a sliding window.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<RateLimiterInner>,
}

struct RateLimiterInner {
    /// Maps IP → list of request timestamps within the window.
    entries: DashMap<IpAddr, Vec<Instant>>,
    /// Maximum number of requests allowed within `window`.
    max_requests: u32,
    /// Duration of the sliding window.
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            inner: Arc::new(RateLimiterInner {
                entries: DashMap::new(),
                max_requests,
                window: Duration::from_secs(window_secs),
            }),
        }
    }

    /// Check if a request from `ip` is allowed. Returns `Ok(())` if allowed,
    /// or `Err(response)` with 429 Too Many Requests.
    pub fn check(&self, ip: IpAddr) -> Result<(), Response> {
        let now = Instant::now();
        let window_start = now - self.inner.window;

        let mut entry = self.inner.entries.entry(ip).or_insert_with(Vec::new);

        // Remove timestamps outside the window
        entry.retain(|t| *t > window_start);

        if entry.len() >= self.inner.max_requests as usize {
            // Calculate retry-after: time until oldest entry expires
            let retry_after = entry
                .first()
                .map(|oldest| {
                    let expires = *oldest + self.inner.window;
                    expires.duration_since(now).as_secs() + 1
                })
                .unwrap_or(self.inner.window.as_secs());

            tracing::warn!(
                ip = %ip,
                limit = self.inner.max_requests,
                window_secs = self.inner.window.as_secs(),
                "Rate limit exceeded"
            );

            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                [
                    ("Retry-After", retry_after.to_string()),
                    ("Content-Type", "application/json".to_string()),
                ],
                axum::Json(json!({
                    "error": "Too many requests. Please try again later.",
                    "retry_after_secs": retry_after
                })),
            )
                .into_response());
        }

        entry.push(now);
        Ok(())
    }

    /// Periodically evict stale entries to prevent memory growth.
    /// Called every 5 minutes by the background cleanup task spawned
    /// from [`RateLimiters::spawn_cleanup_task`].
    pub fn cleanup(&self) {
        let now = Instant::now();
        let window = self.inner.window;
        self.inner.entries.retain(|_ip, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < window);
            !timestamps.is_empty()
        });
    }
}

/// Extract client IP from request headers.
///
/// When `trust_proxy` is `true`, reads `X-Forwarded-For` (leftmost) or
/// `X-Real-IP`. When `false`, returns the loopback address so all
/// clients share one rate-limit bucket (safe for direct-access setups).
///
/// # Security note
/// Only set `trust_proxy = true` when behind a reverse proxy that
/// overwrites these headers. Trusting them on a public-facing server
/// lets attackers spoof arbitrary IPs to bypass rate limiting.
pub fn extract_client_ip(headers: &HeaderMap, trust_proxy: bool) -> IpAddr {
    if trust_proxy {
        // Try X-Forwarded-For first (leftmost = original client)
        if let Some(xff) = headers.get("x-forwarded-for") {
            if let Ok(val) = xff.to_str() {
                if let Some(first) = val.split(',').next() {
                    if let Ok(ip) = first.trim().parse::<IpAddr>() {
                        return ip;
                    }
                }
            }
        }

        // Try X-Real-IP
        if let Some(xri) = headers.get("x-real-ip") {
            if let Ok(val) = xri.to_str() {
                if let Ok(ip) = val.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
    }

    // Fallback to loopback (ConnectInfo not used here — we extract from headers).
    // When trust_proxy is false (default), ALL clients share one rate-limit bucket.
    // This is safe for simple deployments; for multi-user servers behind a proxy,
    // enable trust_proxy in config.toml.
    "127.0.0.1".parse().unwrap()
}

/// Pre-configured rate limiters for different endpoint categories.
#[derive(Clone)]
pub struct RateLimiters {
    /// Login: 10 attempts per 60 seconds
    pub login: RateLimiter,
    /// Registration: 3 per 60 seconds
    pub register: RateLimiter,
    /// TOTP verification: 5 per 60 seconds (1M possible codes — brute-force must be blocked)
    pub totp: RateLimiter,
    /// General API: 100 per 60 seconds.
    /// Used for refresh and logout endpoints.
    pub general: RateLimiter,
}

impl RateLimiters {
    pub fn new() -> Self {
        Self {
            login: RateLimiter::new(10, 60),
            register: RateLimiter::new(3, 60),
            totp: RateLimiter::new(5, 60),
            general: RateLimiter::new(100, 60),
        }
    }

    /// Spawn a background cleanup task that runs every 5 minutes.
    pub fn spawn_cleanup_task(&self) {
        let limiters = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                limiters.login.cleanup();
                limiters.register.cleanup();
                limiters.totp.cleanup();
                limiters.general.cleanup();
                tracing::debug!("Rate limiter cleanup complete");
            }
        });
    }
}
