//! Shared HTTP utilities used across multiple handler modules.

use axum::body::Body;
use axum::http::response::Builder;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;

use crate::error::AppError;

/// Begin a `206 Partial Content` response for the inclusive byte range
/// `start..=end` of a `total`-byte resource.
///
/// Sets the four headers every range response needs identically —
/// `Content-Type`, `Content-Length` (= `end - start + 1`), `Content-Range`
/// (`bytes {start}-{end}/{total}`), and `Accept-Ranges: bytes`. The caller
/// chains any response-specific headers (`ETag`, `Cache-Control`, …) and
/// finishes with `.body(...)`.
///
/// Centralises the error-prone `Content-Range` formatting that was previously
/// hand-written at every range-serving site (`photos/serve.rs`,
/// `blobs/download.rs`).
pub fn partial_content_builder(
    content_type: HeaderValue,
    start: u64,
    end: u64,
    total: u64,
) -> Result<Builder, AppError> {
    let length = end - start + 1;
    Ok(Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header("Content-Type", content_type)
        .header("Content-Length", HeaderValue::from(length))
        .header(
            "Content-Range",
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
                .map_err(|e| AppError::Internal(format!("Invalid header: {e}")))?,
        )
        .header("Accept-Ranges", HeaderValue::from_static("bytes")))
}

/// Build a complete `416 Range Not Satisfiable` response with an empty body and
/// the required `Content-Range: bytes */{total}` header. Returned when a
/// client's `Range` header can't be satisfied against a `total`-byte resource.
pub fn range_not_satisfiable(total: u64) -> Result<Response, AppError> {
    Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(
            "Content-Range",
            HeaderValue::from_str(&format!("bytes */{total}"))
                .map_err(|e| AppError::Internal(format!("Invalid header: {e}")))?,
        )
        .body(Body::empty())
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Parse an HTTP `Range: bytes=START-END` header.
///
/// Supports formats:
/// - `bytes=0-499`     → first 500 bytes
/// - `bytes=500-`      → from byte 500 to the end
/// - `bytes=-500`      → last 500 bytes
///
/// Returns `Some((start, end))` inclusive on success, `None` if invalid.
pub fn parse_range_header(header: &str, total_size: u64) -> Option<(u64, u64)> {
    let header = header.trim();
    if !header.starts_with("bytes=") {
        return None;
    }
    let range_spec = &header[6..];

    // We only handle single ranges (no multipart)
    if range_spec.contains(',') {
        return None;
    }

    let parts: Vec<&str> = range_spec.splitn(2, '-').collect();
    if parts.len() != 2 {
        return None;
    }

    let (start_str, end_str) = (parts[0].trim(), parts[1].trim());

    if start_str.is_empty() {
        // Suffix range: bytes=-500 → last 500 bytes
        let suffix_len: u64 = end_str.parse().ok()?;
        if suffix_len == 0 || suffix_len > total_size {
            return None;
        }
        let start = total_size - suffix_len;
        Some((start, total_size - 1))
    } else {
        let start: u64 = start_str.parse().ok()?;
        if start >= total_size {
            return None;
        }
        let end = if end_str.is_empty() {
            total_size - 1
        } else {
            let e: u64 = end_str.parse().ok()?;
            e.min(total_size - 1)
        };
        if start > end {
            return None;
        }
        Some((start, end))
    }
}

// ── Cache-Control policy (B6) ────────────────────────────────────────────────

/// `Cache-Control` for anything that must never reach a client-side store.
///
/// The exact string [`crate::security::security_headers`] stamps on every
/// non-media `/api/` response, repeated here so a media handler that opts *out*
/// of caching produces a byte-identical header to the middleware's. Two spellings
/// of "do not store this" would be indistinguishable in a test and different on
/// the wire.
pub const NO_STORE: &str = "no-store, no-cache, must-revalidate";

/// `Cache-Control` for mutable media (a photo's file / thumb / web preview).
/// One day: a re-crop or a metadata edit changes the bytes behind a stable id,
/// and the ETag catches that on revalidation.
pub const MEDIA_CACHE_1D: &str = "private, max-age=86400";

/// `Cache-Control` for content-addressed blobs, which are immutable by
/// construction (the id *is* the identity), so revalidation is pure waste.
pub const BLOB_CACHE_IMMUTABLE: &str = "private, max-age=31536000, immutable";

/// Whether a media response may be written to a client-side cache.
///
/// Returned by [`crate::gallery::access::require_secure_access`] so a handler
/// that has *already paid* for the secure-gallery lookup can spend the answer
/// twice: once to enforce the unlock token, once to choose this header. The
/// alternative — re-deriving "is this secure" at header-building time — is a
/// second derivation of one fact, and this repo's todo tracks eight of those
/// drifting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confidentiality {
    /// Ordinary library media. Cacheable at the route's own max-age.
    Cacheable,
    /// Secure-gallery media. **Never stored**, at any layer.
    ///
    /// A browser cache is a plaintext copy on disk that outlives the unlock
    /// token and the session. Caching a decrypted secure photo defeats the
    /// album as thoroughly as not encrypting it, which is why this is not
    /// merely a shorter `max-age`.
    Secure,
}

/// The `Cache-Control` value a media handler should send.
///
/// `when_cacheable` is the route's own policy ([`MEDIA_CACHE_1D`] or
/// [`BLOB_CACHE_IMMUTABLE`]); secure media ignores it entirely.
pub fn media_cache_control(conf: Confidentiality, when_cacheable: &'static str) -> HeaderValue {
    match conf {
        Confidentiality::Cacheable => HeaderValue::from_static(when_cacheable),
        Confidentiality::Secure => HeaderValue::from_static(NO_STORE),
    }
}

/// Routes whose handler owns its own `Cache-Control`, so the security
/// middleware must not overwrite it.
///
/// # Why this list exists at all
///
/// `security.rs` blanket-`insert`ed `no-store` on every `/api/` path, which
/// silently overwrote **17** handler-set `Cache-Control` headers and, with them,
/// the ETag machinery behind those handlers — `no-store` forbids storing the
/// response, so there is nothing left to revalidate. Measured on the wire, a
/// thumbnail declaring `private, max-age=86400` arrived as `no-store`, and every
/// tile in a scrolled grid was re-fetched *and re-decrypted* on every visit.
///
/// # Why an allowlist rather than "don't stomp what the handler set"
///
/// Because that rule fails **open**. A future handler that forgets to set the
/// header would get no protection at all and nothing would say so. Here, a route
/// is cacheable only if it is named here *and* its handler actually set a value;
/// anything else — a new route, an error response, a handler that returns early —
/// falls through to `no-store`. Two independent conditions, both of which must
/// hold, and the failure direction of each is "do not cache".
///
/// # Method matters
///
/// `GET` only. `/api/blobs/{id}` is media on `GET` and a *deletion* on `DELETE`;
/// `/api/blobs` is a JSON listing. Matching on path alone would hand a JSON
/// response the media policy.
///
/// # Deliberately absent
///
/// `/api/trash/{id}/thumb` and `/api/admin/backup/servers/{id}/photos/{id}/thumb`
/// both set a `private, max-age` header today, and both are left to be stomped
/// into `no-store` exactly as they are now. Neither calls
/// [`crate::gallery::access::require_secure_access`], so neither can tell a
/// secured photo from an ordinary one — and a route that cannot classify its own
/// content must not be granted a cache. Adding them means gating them first.
pub fn is_cacheable_media_route(method: &axum::http::Method, path: &str) -> bool {
    if method != axum::http::Method::GET {
        return false;
    }
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match segments.as_slice() {
        // `/api/photos/{id}/{kind}` — every gated media projection of one photo.
        ["api", "photos", _id, kind] => matches!(
            *kind,
            "file" | "source-file" | "thumb" | "thumbnail" | "web" | "motion-video"
        ),
        // `/api/blobs/{id}` and `/api/blobs/{id}/thumb`.
        ["api", "blobs", _id] => true,
        ["api", "blobs", _id, "thumb"] => true,
        _ => false,
    }
}

/// Which responses the global [`CompressionLayer`] is allowed to touch.
///
/// [`CompressionLayer`]: tower_http::compression::CompressionLayer
///
/// `DefaultPredicate` already declines images — but its exclusion is the literal
/// prefix `image/`, so **`video/*` and `audio/*` were never covered**, and every
/// video served from `/api/photos/{id}/file` was being run through gzip. Two
/// costs, and the second is the one that matters:
///
/// 1. H.264/AAC is already entropy-coded. Compressing it is pure CPU for ~0%
///    saving, paid over the *whole* file — and this route streams multi-GB 4K
///    originals frame by frame.
/// 2. **A compressed body is a transformed body, so the layer drops
///    `Content-Length` and `Accept-Ranges: bytes` and switches to
///    `Transfer-Encoding: chunked`.** `serve_photo` sets both headers
///    deliberately; the middleware threw them away. `Accept-Ranges` is how a
///    client discovers that seeking is possible at all, which makes this a
///    functional defect on the #49 ladder's own serving path rather than a
///    performance wart: the picker exists to let a user swap quality mid-video,
///    and the response advertised no seek support.
///
/// Fixed centrally rather than by adding `Content-Encoding: identity` to each
/// response builder in `photos/serve.rs`. `main.rs` documents that per-response
/// opt-out as the contract and `blobs/download.rs` follows it in four places —
/// but `photos/serve.rs` follows it in **zero**, which is precisely the
/// "two derivations of one rule will drift" failure `todo.md` tracks. One
/// predicate cannot be forgotten by the next media route that gets added.
///
/// `blobs/download.rs`'s explicit `identity` headers are left in place: they are
/// still correct, they cover `application/octet-stream` (which is *not* excluded
/// here, because octet-stream is a genuine catch-all and some of it compresses),
/// and removing them would be churn with a downside and no upside.
pub fn media_compression_predicate() -> impl tower_http::compression::Predicate {
    use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};

    DefaultPredicate::new()
        .and(NotForContentType::const_new("video/"))
        .and(NotForContentType::const_new("audio/"))
}

#[cfg(test)]
mod cache_policy_tests {
    use super::*;
    use axum::http::Method;

    /// Every media route the allowlist is meant to cover, spelled as a real
    /// path. A regression here is silent — the route keeps working, it just
    /// stops being cacheable — so it has to be asserted rather than eyeballed.
    #[test]
    fn every_gated_media_route_is_cacheable() {
        let id = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
        for path in [
            format!("/api/photos/{id}/file"),
            format!("/api/photos/{id}/source-file"),
            format!("/api/photos/{id}/thumb"),
            format!("/api/photos/{id}/thumbnail"),
            format!("/api/photos/{id}/web"),
            format!("/api/photos/{id}/motion-video"),
            format!("/api/blobs/{id}"),
            format!("/api/blobs/{id}/thumb"),
        ] {
            assert!(
                is_cacheable_media_route(&Method::GET, &path),
                "{path} serves media and its handler owns its Cache-Control"
            );
        }
    }

    /// The JSON API is the thing `no-store` exists for. Any of these leaking
    /// onto the allowlist would let a token-bearing response be written to disk.
    #[test]
    fn json_and_control_routes_are_never_cacheable() {
        for path in [
            "/api/photos",
            "/api/blobs",
            "/api/auth/login",
            "/api/auth/refresh",
            "/api/galleries/secure/g1/items",
            "/api/sync/delta",
            "/api/status/encryption",
            "/health",
            // Same prefix, different resource — `burst` sits where an id does,
            // so the id-shaped wildcard must not swallow it.
            "/api/photos/burst/b1",
            "/api/photos/crop-sync",
            "/api/photos/detect-bursts",
        ] {
            assert!(
                !is_cacheable_media_route(&Method::GET, path),
                "{path} is not media — it must fall through to no-store"
            );
        }
    }

    /// `/api/blobs/{id}` is media on GET and a **deletion** on DELETE. Matching
    /// on path alone would hand a mutation the media cache policy.
    #[test]
    fn only_get_is_cacheable() {
        let path = "/api/blobs/0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
        assert!(is_cacheable_media_route(&Method::GET, path));
        for m in [Method::DELETE, Method::POST, Method::PUT, Method::PATCH] {
            assert!(
                !is_cacheable_media_route(&m, path),
                "{m} {path} is not a cacheable read"
            );
        }
    }

    /// Two routes that set a `private, max-age` header today but cannot tell a
    /// secured photo from an ordinary one — neither calls
    /// `require_secure_access`. They are deliberately excluded, and this test is
    /// the record of that decision: adding them means gating them first.
    #[test]
    fn routes_that_cannot_classify_their_content_are_excluded() {
        for path in [
            "/api/trash/t1/thumb",
            "/api/admin/backup/servers/s1/photos/p1/thumb",
        ] {
            assert!(
                !is_cacheable_media_route(&Method::GET, path),
                "{path} has no secure-album gate, so it must not be granted a cache"
            );
        }
    }

    /// The whole point of `Confidentiality`. A secure item ignores the route's
    /// own policy entirely rather than getting a shorter `max-age`.
    #[test]
    fn secure_media_never_gets_a_cacheable_header() {
        for policy in [MEDIA_CACHE_1D, BLOB_CACHE_IMMUTABLE] {
            let v = media_cache_control(Confidentiality::Secure, policy);
            assert_eq!(v, NO_STORE);
            let s = v.to_str().unwrap();
            assert!(
                !s.contains("max-age") && !s.contains("immutable"),
                "secure media must not carry any storage permission, got {s:?}"
            );
        }
    }

    /// The other half — without this, `media_cache_control` returning `NO_STORE`
    /// unconditionally would pass every assertion above while re-introducing the
    /// exact bug B6 exists to fix.
    #[test]
    fn ordinary_media_keeps_its_routes_own_policy() {
        assert_eq!(
            media_cache_control(Confidentiality::Cacheable, MEDIA_CACHE_1D),
            MEDIA_CACHE_1D
        );
        assert_eq!(
            media_cache_control(Confidentiality::Cacheable, BLOB_CACHE_IMMUTABLE),
            BLOB_CACHE_IMMUTABLE
        );
    }

    /// The middleware's default and a handler's secure value must be **byte
    /// identical**. Two spellings of "do not store this" are indistinguishable
    /// in a test that only checks for the substring `no-store`, and different on
    /// the wire — a proxy that understands one and not the other is exactly the
    /// kind of gap that never shows up locally.
    #[test]
    fn the_secure_value_is_the_same_string_the_middleware_stamps() {
        assert_eq!(NO_STORE, "no-store, no-cache, must-revalidate");
    }
}

#[cfg(test)]
mod compression_predicate_tests {
    use super::*;
    use tower_http::compression::Predicate as _;

    /// A body with a real size hint.
    ///
    /// **Not `Body::empty()`**, and that is the whole reason this helper exists:
    /// `DefaultPredicate` starts with `SizeAbove(32)`, so an empty body reports
    /// "do not compress" for *every* content type and every assertion below
    /// would pass against a predicate that does nothing at all. Same shape as
    /// the vacuous-pass traps `todo.md` records in B4 and A2.
    fn response(content_type: &str) -> axum::http::Response<Body> {
        axum::http::Response::builder()
            .header("Content-Type", content_type)
            .body(Body::from(vec![0u8; 1024]))
            .unwrap()
    }

    #[test]
    fn video_and_audio_are_never_compressed() {
        let p = media_compression_predicate();
        for ct in [
            "video/mp4",
            "video/quicktime",
            "video/webm",
            "audio/mpeg",
            "audio/aac",
        ] {
            assert!(
                !p.should_compress(&response(ct)),
                "{ct} must bypass compression — gzipping it strips Accept-Ranges"
            );
        }
    }

    /// The exact regression. `video/mp4` is what the ladder stamps on every
    /// rung, and it was compressed because the default exclusion is the string
    /// `image/`.
    #[test]
    fn the_stock_default_would_have_compressed_video_mp4() {
        use tower_http::compression::predicate::DefaultPredicate;
        assert!(
            DefaultPredicate::new().should_compress(&response("video/mp4")),
            "precondition: if the stock default already declined video/mp4 there \
             was never a bug and this predicate is dead weight"
        );
    }

    /// The vacuity guard with teeth: a predicate that refuses everything would
    /// satisfy every assertion above while silently un-compressing the JSON API,
    /// which is what the layer is actually there for.
    #[test]
    fn json_and_text_are_still_compressed() {
        let p = media_compression_predicate();
        for ct in ["application/json", "text/html", "text/css"] {
            assert!(
                p.should_compress(&response(ct)),
                "{ct} must still be compressed — that is the layer's whole purpose"
            );
        }
    }

    /// The two halves of `DefaultPredicate` we are building on must survive
    /// being wrapped: the size floor, and the SVG exception to the image rule.
    #[test]
    fn the_inherited_default_rules_are_not_lost() {
        let p = media_compression_predicate();

        assert!(!p.should_compress(&response("image/jpeg")));
        assert!(
            p.should_compress(&response("image/svg+xml")),
            "SVG is text and DefaultPredicate deliberately excepts it"
        );

        let tiny = axum::http::Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(vec![0u8; 8]))
            .unwrap();
        assert!(
            !p.should_compress(&tiny),
            "SizeAbove(32) must survive — compressing 8 bytes costs more than it saves"
        );
    }
}
