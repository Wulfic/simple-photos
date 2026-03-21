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
/// Handles:
/// - Naive "2024-01-15T14:30:00" → "2024-01-15T14:30:00.000Z" (treated as UTC)
/// - Offset "+00:00" → "Z"
/// - Already "Z" → passed through
pub fn normalize_iso_timestamp(ts: &str) -> String {
    // Try parsing as a full DateTime<Utc> or DateTime<FixedOffset>
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        return dt
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true);
    }
    // Try parsing as naive datetime (no timezone) — treat as UTC
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.f") {
        let dt = naive.and_utc();
        return dt.to_rfc3339_opts(SecondsFormat::Millis, true);
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S") {
        let dt = naive.and_utc();
        return dt.to_rfc3339_opts(SecondsFormat::Millis, true);
    }
    // Fallback: return as-is
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
