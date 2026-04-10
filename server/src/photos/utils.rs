//! Shared utility functions for photo timestamp normalization and hashing.

use chrono::{SecondsFormat, Utc};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncReadExt;

/// Produce a UTC ISO-8601 timestamp with millisecond precision and Z suffix.
/// Format: `2024-02-28T22:44:29.043Z`
///
/// This is critical for consistent text-based sorting in SQLite — all
/// timestamps (taken_at, created_at) must use the same format so that
/// `ORDER BY COALESCE(taken_at, created_at) DESC` works correctly.
pub fn utc_now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Normalize a timestamp string to consistent ISO-8601 Z-suffix format.
///
/// All timestamps in the system must use the canonical format
/// `YYYY-MM-DDTHH:MM:SS.sssZ` (UTC, millisecond precision, Z suffix) so
/// that text-based `ORDER BY` in SQLite produces correct chronological
/// ordering.  This function accepts many common input formats and converts
/// them to the canonical form:
///
/// - RFC 3339 / ISO 8601 with offset: `2024-01-15T14:30:00+05:30` → converted to UTC
/// - ISO 8601 with Z:      `2024-01-15T14:30:00Z` → passed through (with millis added)
/// - Naive ISO 8601:        `2024-01-15T14:30:00` → treated as UTC
/// - EXIF DateTimeOriginal: `2024:01:15 14:30:00` → converted to ISO
/// - Date only:             `2024-01-15` → midnight UTC
/// - Unix timestamp (secs): `1705312200` → converted
/// - Chrono default:        `2024-01-15 14:30:00 UTC` → parsed
pub fn normalize_iso_timestamp(ts: &str) -> String {
    let ts = ts.trim();
    if ts.is_empty() {
        return ts.to_string();
    }

    // Try parsing as a full DateTime<Utc> or DateTime<FixedOffset> (RFC 3339)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        return dt
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true);
    }

    // Try parsing as naive datetime with fractional seconds
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.f") {
        return naive.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true);
    }

    // Naive datetime without fractional seconds
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S") {
        return naive.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true);
    }

    // EXIF DateTimeOriginal format: "2024:01:15 14:30:00"
    if ts.len() >= 19 && ts.as_bytes().get(4) == Some(&b':') && ts.as_bytes().get(7) == Some(&b':') {
        let converted = format!(
            "{}-{}-{}T{}",
            &ts[0..4],
            &ts[5..7],
            &ts[8..10],
            &ts[11..19]
        );
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&converted, "%Y-%m-%dT%H:%M:%S") {
            return naive.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true);
        }
    }

    // Space-separated datetime (e.g. "2024-01-15 14:30:00" from some DBs/tools)
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S%.f") {
        return naive.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true);
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
        return naive.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true);
    }

    // Chrono default format: "2024-01-15 14:30:00 UTC"
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(
        ts.trim_end_matches(" UTC").trim_end_matches(" utc"),
        "%Y-%m-%d %H:%M:%S",
    ) {
        return naive.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true);
    }

    // Date only: "2024-01-15" → midnight UTC
    if let Ok(date) = chrono::NaiveDate::parse_from_str(ts, "%Y-%m-%d") {
        if let Some(dt) = date.and_hms_opt(0, 0, 0) {
            return dt.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true);
        }
    }

    // Unix timestamp (seconds since epoch) — pure digits, 9-13 chars
    if ts.len() >= 9 && ts.len() <= 13 && ts.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(secs) = ts.parse::<i64>() {
            // If > 10 billion, treat as milliseconds
            let secs = if secs > 10_000_000_000 { secs / 1000 } else { secs };
            if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
                return dt.to_rfc3339_opts(SecondsFormat::Millis, true);
            }
        }
    }

    // Fallback: return as-is (caller will store it; better than losing data)
    ts.to_string()
}

/// Compute a short content-based hash: first 12 hex chars of SHA-256.
/// This deterministic fingerprint is the same regardless of which platform
/// uploads the photo, guaranteeing cross-platform alignment.
pub fn compute_photo_hash(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex::encode(&digest[..6]) // 6 bytes → 12 hex chars (48-bit)
}

/// Streaming variant of [`compute_photo_hash`] — reads in 64 KB chunks so
/// large files (videos, RAW photos) never need to be buffered entirely in
/// memory.  Returns `None` only when the file cannot be opened or read.
pub async fn compute_photo_hash_streaming(path: &Path) -> Option<String> {
    let mut file = tokio::fs::File::open(path).await.ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 65_536]; // 64 KB chunks
    loop {
        let n = file.read(&mut buf).await.ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hex::encode(&hasher.finalize()[..6]))
}
