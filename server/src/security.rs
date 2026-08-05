//! Security hardening middleware.
//!
//! Adds defense-in-depth HTTP response headers recommended by OWASP:
//! - Content-Security-Policy: restrict script/style/img sources
//! - X-Content-Type-Options: prevent MIME-sniffing attacks
//! - X-Frame-Options: prevent clickjacking
//! - Referrer-Policy: prevent URL leakage
//! - Strict-Transport-Security: force HTTPS
//! - Permissions-Policy: disable unnecessary browser APIs
//! - Cache-Control: prevent caching of sensitive API responses (API only)
//!
//! Also adds a unique request ID header (X-Request-Id) for tracing.
//!
//! **Performance note:** Cache-Control is only applied to `/api/` and `/health`
//! paths.  Static frontend assets (JS, CSS, images) served by `ServeDir` keep
//! their own cache headers, avoiding unnecessary re-downloads on every page load.

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
    // UUID v7: monotonic, time-sortable — cheaper than v4 (no CSPRNG call)
    // and naturally sorts by creation time in logs/traces.
    let request_id = Uuid::now_v7().to_string();

    // Capture path and method before the request is consumed by `next.run()`.
    // Used below to decide whether to apply no-store Cache-Control (API routes)
    // or leave the handler's own value intact (media routes, static assets).
    //
    // The method is load-bearing, not decoration: `/api/blobs/{id}` is media on
    // GET and a *deletion* on DELETE, and `/api/blobs` is a JSON listing.
    let path = request.uri().path().to_string();
    let method = request.method().clone();

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
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=(), usb=()"),
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

    // ── Cache-Control: no-store for the API, except media routes (B6) ────────
    //
    // `no-store` is the default for API and health endpoints, whose responses
    // may contain user data or tokens. Static frontend assets (JS, CSS, images,
    // fonts) served by ServeDir/ServeFile keep their own long-lived headers —
    // previously we stomped them with no-store, forcing browsers to re-download
    // the entire frontend on every page load.
    //
    // **That same mistake was still live for media, and this is the fix.** The
    // override used to be an unconditional `insert` over the whole `/api/`
    // prefix, so it silently overwrote 17 handler-set `Cache-Control` headers.
    // With them died the ETag machinery behind them: `no-store` forbids storing
    // the response, so there is nothing left to revalidate against. Measured on
    // the wire, `/api/photos/{id}/thumb` sent `private, max-age=86400` and the
    // client received `no-store` — every tile in a scrolled grid re-fetched and
    // re-decrypted on every single visit, and #49's swap-quality-keep-playing
    // picker had no bytes to keep.
    //
    // A media handler now keeps the header it set, but only when **both** hold:
    //
    // 1. the route is on `is_cacheable_media_route`'s allowlist, and
    // 2. the handler actually set a value.
    //
    // Both conditions fail towards `no-store`, which is what makes this safe to
    // extend: a new route, an error response, or a handler that returns early
    // all land back on the default. The allowlisted handlers each derive their
    // value from `require_secure_access`, so secure-gallery media sends
    // `no-store` from the handler itself — the middleware is not what protects
    // it, and must not be mistaken for what does.
    let is_api = path.starts_with("/api/") || path == "/health";
    if is_api {
        let handler_owns_it = crate::http_utils::is_cacheable_media_route(&method, &path)
            && headers.contains_key("cache-control");
        if !handler_owns_it {
            headers.insert(
                "Cache-Control",
                HeaderValue::from_static(crate::http_utils::NO_STORE),
            );
        }
    }
    // Static assets: no Cache-Control override — ServeDir sets appropriate
    // headers (or the browser uses heuristic caching for hashed filenames).

    // Request ID for tracing/debugging
    if let Ok(val) = HeaderValue::from_str(&request_id) {
        headers.insert("X-Request-Id", val);
    }

    response
}

#[cfg(test)]
mod cache_control_tests {
    //! B6 regression tests, driven through the **real middleware**.
    //!
    //! Every one of the 17 overwritten `Cache-Control` headers was correct when
    //! read in its handler. The defect existed only after this middleware ran,
    //! which is precisely why no unit test in `serve.rs` or `download.rs` ever
    //! saw it. These tests therefore assert on the response that leaves the
    //! stack, not on what a handler builds.

    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::response::Response;
    use axum::routing::{delete, get};
    use axum::{middleware, Router};
    use tower::ServiceExt as _;

    /// A stand-in handler that sets `Cache-Control` exactly as the real media
    /// handlers do.
    async fn media_handler() -> Response {
        Response::builder()
            .status(StatusCode::OK)
            .header("Cache-Control", crate::http_utils::MEDIA_CACHE_1D)
            .header("ETag", "\"p1-thumb-1234\"")
            .body(Body::from("bytes"))
            .unwrap()
    }

    /// A media handler serving a **secure** item: it picks `no-store` itself via
    /// `media_cache_control(Confidentiality::Secure, ..)`.
    async fn secure_media_handler() -> Response {
        Response::builder()
            .status(StatusCode::OK)
            .header(
                "Cache-Control",
                crate::http_utils::media_cache_control(
                    crate::http_utils::Confidentiality::Secure,
                    crate::http_utils::MEDIA_CACHE_1D,
                ),
            )
            .body(Body::from("secret bytes"))
            .unwrap()
    }

    /// A JSON endpoint — sets nothing, and must not be cacheable.
    async fn json_handler() -> Response {
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"token":"secret"}"#))
            .unwrap()
    }

    fn app() -> Router {
        Router::new()
            .route("/api/photos/{id}/thumb", get(media_handler))
            .route("/api/photos/{id}/file", get(media_handler))
            .route("/api/blobs/{id}", get(media_handler))
            .route("/api/blobs/{id}", delete(json_handler))
            .route("/api/photos/{id}/web", get(secure_media_handler))
            .route("/api/photos", get(json_handler))
            .route("/api/auth/refresh", get(json_handler))
            .route("/api/trash/{id}/thumb", get(media_handler))
            .layer(middleware::from_fn(super::security_headers))
    }

    async fn cache_control_of(method: Method, uri: &str) -> String {
        let res = app()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        res.headers()
            .get("cache-control")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_else(|| "<absent>".to_string())
    }

    /// **The B6 defect, verbatim.** On the pre-fix tree this returns
    /// `no-store, no-cache, must-revalidate` for all three.
    #[tokio::test]
    async fn media_routes_keep_the_handlers_cache_control() {
        for uri in [
            "/api/photos/p1/thumb",
            "/api/photos/p1/file",
            "/api/blobs/b1",
        ] {
            assert_eq!(
                cache_control_of(Method::GET, uri).await,
                crate::http_utils::MEDIA_CACHE_1D,
                "{uri}: the middleware overwrote the handler's header — this is B6"
            );
        }
    }

    /// The vacuity guard, and the one that would matter most if it broke.
    /// Without it, a middleware changed to "never touch Cache-Control" would
    /// satisfy every other test here while letting a refresh-token response be
    /// written to a browser's disk cache.
    #[tokio::test]
    async fn json_routes_are_still_no_store() {
        for uri in ["/api/photos", "/api/auth/refresh"] {
            assert_eq!(
                cache_control_of(Method::GET, uri).await,
                crate::http_utils::NO_STORE,
                "{uri} may carry user data or tokens and must never be stored"
            );
        }
    }

    /// Secure media sets `no-store` at the handler. The middleware must leave it
    /// alone — and, critically, the result must be identical to what a
    /// non-media route gets, so nothing downstream can tell the two apart.
    #[tokio::test]
    async fn secure_media_is_no_store_and_indistinguishable_from_the_default() {
        assert_eq!(
            cache_control_of(Method::GET, "/api/photos/p1/web").await,
            crate::http_utils::NO_STORE
        );
    }

    /// Same path, different method. The DELETE must not inherit the media
    /// policy just because `/api/blobs/{id}` is cacheable on GET.
    #[tokio::test]
    async fn the_same_path_is_not_cacheable_on_a_mutating_method() {
        assert_eq!(
            cache_control_of(Method::GET, "/api/blobs/b1").await,
            crate::http_utils::MEDIA_CACHE_1D
        );
        assert_eq!(
            cache_control_of(Method::DELETE, "/api/blobs/b1").await,
            crate::http_utils::NO_STORE
        );
    }

    /// A route that sets a `private, max-age` header but is **not** on the
    /// allowlist still gets stomped. This pins the fail-closed direction: the
    /// allowlist is what grants caching, not the presence of a handler header.
    #[tokio::test]
    async fn a_non_allowlisted_route_is_stomped_even_though_it_set_a_header() {
        assert_eq!(
            cache_control_of(Method::GET, "/api/trash/t1/thumb").await,
            crate::http_utils::NO_STORE,
            "trash thumbs have no secure-album gate; they must stay uncacheable"
        );
    }

    /// The other fail-closed arm: on the allowlist, but the handler set nothing
    /// (an early return, an error response). Absence must not read as
    /// permission.
    #[tokio::test]
    async fn an_allowlisted_route_that_sets_no_header_falls_back_to_no_store() {
        let app = Router::new()
            .route("/api/photos/{id}/thumb", get(json_handler))
            .layer(middleware::from_fn(super::security_headers));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/photos/p1/thumb")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.headers().get("cache-control").unwrap(),
            crate::http_utils::NO_STORE
        );
    }
}
