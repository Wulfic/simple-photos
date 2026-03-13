//! Security hardening middleware.
//!
//! Adds defense-in-depth HTTP response headers recommended by OWASP:
//! - Content-Security-Policy: restrict script/style/img sources
//! - X-Content-Type-Options: prevent MIME-sniffing attacks
//! - X-Frame-Options: prevent clickjacking
//! - Referrer-Policy: prevent URL leakage
//! - Strict-Transport-Security: force HTTPS
//! - Permissions-Policy: disable unnecessary browser APIs
//! - Cache-Control: prevent caching of sensitive API responses
//!
//! Also adds a unique request ID header (X-Request-Id) for tracing.

use axum::body::Body;
use axum::http::{HeaderValue, Request};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

/// Middleware that adds security headers to every response.
///
/// These headers provide defense-in-depth and are recommended by OWASP.
/// They do NOT replace proper server-side security but reduce the attack
/// surface on the client side.
///
/// **Note on HSTS:** `Strict-Transport-Security` is set unconditionally.
/// For LAN-only HTTP deployments this is harmless, but once a browser
/// visits over HTTPS it will refuse plain HTTP for 1 year.
pub async fn security_headers(request: Request<Body>, next: Next) -> Response {
    // Generate a request ID for tracing and correlation
    let request_id = Uuid::new_v4().to_string();

    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    // ── OWASP recommended headers ────────────────────────────────────────────

    // Prevent MIME-type sniffing (IE/Chrome)
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );

    // Prevent clickjacking — no iframe embedding
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));

    // Control what info the Referer header leaks
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // Force HTTPS for 1 year, include subdomains
    headers.insert(
        "Strict-Transport-Security",
        HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );

    // Disable dangerous browser APIs we don't need
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static(
            "camera=(), microphone=(), geolocation=(), payment=(), usb=()"
        ),
    );

    // CSP: allow self + inline styles (Tailwind) + blob: for media URLs + wasm for Argon2id
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' 'wasm-unsafe-eval'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' blob: data:; \
             media-src 'self' blob:; \
             connect-src 'self'; \
             font-src 'self'; \
             object-src 'none'; \
             frame-ancestors 'none'; \
             base-uri 'self'; \
             form-action 'self'",
        ),
    );

    // Prevent caching of API responses — may contain user data or tokens.
    // CAVEAT: This middleware runs after ServeDir, so it also overwrites
    // cache headers on static assets (JS/CSS/images), forcing browsers to
    // re-download them every page load. A future improvement could skip
    // the header for static file paths.
    headers.insert(
        "Cache-Control",
        HeaderValue::from_static("no-store, no-cache, must-revalidate"),
    );

    // Request ID for tracing/debugging
    if let Ok(val) = HeaderValue::from_str(&request_id) {
        headers.insert("X-Request-Id", val);
    }

    response
}
