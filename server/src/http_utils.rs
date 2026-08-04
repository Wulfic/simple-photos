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
