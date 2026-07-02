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
