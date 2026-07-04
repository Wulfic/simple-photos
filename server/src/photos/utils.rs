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
    if ts.len() >= 19 && ts.as_bytes().get(4) == Some(&b':') && ts.as_bytes().get(7) == Some(&b':')
    {
        let converted = format!("{}-{}-{}T{}", &ts[0..4], &ts[5..7], &ts[8..10], &ts[11..19]);
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
            let secs = if secs > 10_000_000_000 {
                secs / 1000
            } else {
                secs
            };
            if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
                return dt.to_rfc3339_opts(SecondsFormat::Millis, true);
            }
        }
    }

    // Fallback: return as-is (caller will store it; better than losing data)
    ts.to_string()
}

/// Resolve the canonical `taken_at` for an uploaded photo from every available
/// signal, in offset-aware priority order. Extracted from the upload handler so
/// the ordering is unit-testable and lives in exactly one place (the previous
/// inline version silently let an assume-UTC EXIF guess beat a zone-correct
/// Google Takeout epoch, shifting many photos to the wrong day):
///
///   1. EXIF `DateTimeOriginal` **with** a real capture-zone offset — an
///      unambiguous true instant from the camera; always wins.
///   2. `sidecar_taken` (Google Takeout `photoTakenTime`, a true UTC epoch) —
///      beats an offset-less EXIF value, which was only *assumed* to be UTC.
///   3. EXIF `DateTimeOriginal` **without** an offset (assume-UTC) — still far
///      better than the file mtime for chronological placement.
///   4. `file_modified` (the browser File's `lastModified`).
///   5. `fallback` (typically the upload time).
///
/// Every non-empty candidate is normalised to canonical ISO-8601 UTC via
/// [`normalize_iso_timestamp`] so the result sorts correctly in SQLite.
pub fn resolve_upload_taken_at(
    exif_taken: Option<&str>,
    exif_has_offset: bool,
    sidecar_taken: Option<&str>,
    file_modified: Option<&str>,
    fallback: &str,
) -> String {
    resolve_taken_at(exif_taken, exif_has_offset, sidecar_taken, file_modified)
        .unwrap_or_else(|| fallback.to_string())
}

/// Same offset-aware priority as [`resolve_upload_taken_at`] but returns `None`
/// when no signal carries a usable capture date (rather than substituting a
/// fallback). The filesystem-scan import paths ([`crate::photos::register`])
/// want this: a missing `taken_at` is left NULL so gallery sorting falls back to
/// `created_at`, and — crucially for Google Takeout — the sidecar's
/// `photoTakenTime` beats the file mtime, which for an unzipped Takeout is the
/// *extraction* date, not the capture date. Every non-empty candidate is
/// normalised to canonical ISO-8601 UTC so the result sorts correctly in SQLite.
pub fn resolve_taken_at(
    exif_taken: Option<&str>,
    exif_has_offset: bool,
    sidecar_taken: Option<&str>,
    file_modified: Option<&str>,
) -> Option<String> {
    let exif = exif_taken
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(normalize_iso_timestamp);
    let sidecar = sidecar_taken
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(normalize_iso_timestamp);

    let chosen = if exif_has_offset {
        // (1) zone-correct EXIF always wins.
        exif
    } else {
        // (2) sidecar epoch beats assume-UTC EXIF; (3) fall back to that EXIF.
        sidecar.or(exif)
    };

    chosen.or_else(|| {
        file_modified
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(normalize_iso_timestamp)
    })
}

/// Read the `audio_backup_enabled` server setting.
///
/// Returns `false` when the setting is unset, malformed, or the query fails —
/// failing closed is the safe default (a stale read should never silently turn
/// audio backup *on* against the user's wishes).
///
/// Single source of truth for every import path: upload, scan, ingest,
/// autoscan, and the cross-server sync engine all funnel through this so
/// the toggle is enforced consistently.
pub async fn audio_backup_enabled(pool: &sqlx::SqlitePool) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT value = 'true' FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    const FALLBACK: &str = "2026-07-03T00:00:00.000Z";

    #[test]
    fn zoned_exif_always_wins_over_sidecar() {
        // A modern phone writes OffsetTimeOriginal → the EXIF instant is
        // unambiguous and must beat the Takeout sidecar.
        let got = resolve_upload_taken_at(
            Some("2021-06-01T12:00:00+00:00"), // already-UTC EXIF instant
            true,                              // had a real offset
            Some("1609459200"),                // sidecar epoch (2021-01-01) — ignored
            None,
            FALLBACK,
        );
        assert_eq!(got, "2021-06-01T12:00:00.000Z");
    }

    #[test]
    fn sidecar_epoch_beats_offsetless_exif() {
        // THE regression guard: offset-less EXIF was only *assumed* UTC, so the
        // zone-correct Google Takeout epoch must win instead.
        let got = resolve_upload_taken_at(
            Some("2021-06-01T21:00:00Z"), // assume-UTC EXIF (local wall-clock)
            false,                        // NO offset
            Some("1622574000"),           // 2021-06-01T19:00:00Z (true instant)
            None,
            FALLBACK,
        );
        assert_eq!(got, "2021-06-01T19:00:00.000Z", "sidecar epoch wins");
    }

    #[test]
    fn offsetless_exif_used_when_no_sidecar() {
        // Plain manual upload, EXIF but no offset and no sidecar → keep EXIF
        // (still far better than the file mtime).
        let got = resolve_upload_taken_at(
            Some("2021-06-01T21:00:00Z"),
            false,
            None,
            Some("2020-01-01T00:00:00Z"),
            FALLBACK,
        );
        assert_eq!(got, "2021-06-01T21:00:00.000Z");
    }

    #[test]
    fn file_modified_used_when_no_exif_or_sidecar() {
        let got =
            resolve_upload_taken_at(None, false, None, Some("2020-01-01T00:00:00Z"), FALLBACK);
        assert_eq!(got, "2020-01-01T00:00:00.000Z");
    }

    #[test]
    fn empty_candidates_are_ignored_and_fall_through_to_fallback() {
        // Blank header values must not be treated as a real timestamp.
        let got = resolve_upload_taken_at(Some("   "), false, Some(""), Some(""), FALLBACK);
        assert_eq!(got, FALLBACK);
    }

    #[test]
    fn blank_sidecar_falls_back_to_offsetless_exif() {
        let got = resolve_upload_taken_at(
            Some("2021-06-01T21:00:00Z"),
            false,
            Some("   "), // blank sidecar → skipped, not chosen over EXIF
            None,
            FALLBACK,
        );
        assert_eq!(got, "2021-06-01T21:00:00.000Z");
    }
}
