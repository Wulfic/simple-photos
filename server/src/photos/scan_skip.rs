//! Verdicts for the `scan_skipped_paths` cache — "have we already examined this
//! file, and may we skip it this pass?"
//!
//! Migration 031 introduced the cache with exactly one rule: a row exists and
//! the file is unchanged ⇒ skip forever. That is right for the two verdicts it
//! shipped (`hash_duplicate`, `gallery_hidden`), which are *deterministic dead
//! ends* — re-examining them can only ever reach the same answer.
//!
//! #40 adds a third reason, `conversion_failed`, which is **not** deterministic:
//! a transcode can fail for a reason that goes away (a GPU that was busy, a
//! network mount that blipped, an ffmpeg upgrade). Retiring it on the first
//! failure would be a silent one-strike cap; retrying it forever is the bug #40
//! is about. So the row carries an `attempt_count` and the verdict depends on
//! the reason.
//!
//! **This is the whole reason this module exists as a pure function.** The two
//! walks that consult the cache (`backup::autoscan` for native files,
//! `ingest::run_conversion_pass_inner` for convertible ones) previously each
//! inlined the `size == && mtime ==` comparison, and a third caller with
//! *different* semantics per reason is exactly how those drift apart. Nothing
//! here touches a DB or the filesystem, so every branch below is unit-tested.

/// How many times a single file may be handed to the transcoder before it is
/// retired. Named rather than a literal because it appears in the walk, in the
/// failure path, and in the audit event that announces the retirement, and
/// three copies of `3` is how a cap becomes inconsistent.
pub const CONVERSION_MAX_ATTEMPTS: i64 = 3;

/// Content collided with an already-registered photo's hash (Takeout stores the
/// same bytes in the date folder and in every album folder).
pub const REASON_HASH_DUPLICATE: &str = "hash_duplicate";

/// Content belongs to a secure-gallery original and must stay out of the main
/// gallery.
pub const REASON_GALLERY_HIDDEN: &str = "gallery_hidden";

/// Conversion was attempted and left no `photos` row behind (#40). The only
/// reason whose verdict depends on `attempt_count`.
pub const REASON_CONVERSION_FAILED: &str = "conversion_failed";

/// A video container with no decodable video stream at all (#46:
/// `VIDEO0063.mp4`). A re-encode cannot invent a stream, so re-examining it can
/// only ever reach the same answer — terminal, like `hash_duplicate`. Recorded
/// by the conversion walk so an unregistered, unplayable file stops costing an
/// ffprobe on every pass. Un-retired if the file changes on disk (031's rule).
pub const REASON_UNPLAYABLE: &str = "unplayable";

/// A `scan_skipped_paths` row, as the walks load it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipRow {
    pub size_bytes: i64,
    pub mtime: Option<String>,
    pub reason: String,
    pub attempt_count: i64,
}

/// What a walk should do with a candidate that has a skip row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipVerdict {
    /// Leave the file alone this pass. No hash, no probe, no transcode.
    Skip,
    /// The row still describes this file, but it has attempts left — evaluate
    /// it again. The row is **kept**: it carries the attempt history.
    Retry,
    /// The file on disk changed since the row was written, so the row describes
    /// a file that no longer exists. Drop it and evaluate from scratch.
    Stale,
}

/// Decide what to do with `row` given the candidate's current size and mtime.
///
/// `Stale` outranks everything: a replaced or edited file has earned a fresh
/// evaluation regardless of what the previous verdict was or how many attempts
/// it burned. That is migration 031's invalidation rule, and it is also exactly
/// the retry escape hatch #40 asks for ("until its mtime changes, after which it
/// is retried") — so the cap needs no separate expiry policy.
pub fn skip_verdict(row: &SkipRow, size_bytes: i64, mtime: Option<&str>) -> SkipVerdict {
    if row.size_bytes != size_bytes || row.mtime.as_deref() != mtime {
        return SkipVerdict::Stale;
    }

    if row.reason == REASON_CONVERSION_FAILED {
        // `>=` rather than `==` deliberately, matching the #41 window cap: if a
        // count ever overshoots (a double-charge, a manual edit), the file stays
        // retired instead of being waved through on every pass forever. The
        // asymmetry is the point — over-retiring costs one file its conversion,
        // under-retiring costs a full transcode every pass, indefinitely.
        if row.attempt_count >= CONVERSION_MAX_ATTEMPTS {
            return SkipVerdict::Skip;
        }
        return SkipVerdict::Retry;
    }

    // `hash_duplicate`, `gallery_hidden`, and any reason a future migration adds
    // are terminal — this is migration 031's original behaviour, preserved as
    // the default so a new reason cannot accidentally opt into the retry path.
    SkipVerdict::Skip
}

/// Whether a file that has just been handed to the transcoder for the
/// `attempt`-th time has now exhausted its budget. Used to decide whether the
/// failure path emits the terminal "retired" audit event.
///
/// Takes the count **after** the increment, because attempts are charged before
/// the encode runs (see `ingest::process_candidate`): a file that hard-kills
/// ffmpeg never reaches an error handler, and that is precisely the file that
/// needs retiring.
pub fn is_retired(attempt_count: i64) -> bool {
    attempt_count >= CONVERSION_MAX_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(reason: &str, attempt_count: i64) -> SkipRow {
        SkipRow {
            size_bytes: 1_000,
            mtime: Some("2026-07-21T00:00:00.000Z".to_string()),
            reason: reason.to_string(),
            attempt_count,
        }
    }

    const MTIME: Option<&str> = Some("2026-07-21T00:00:00.000Z");

    // ── The 031 verdicts stay terminal ───────────────────────────────────

    #[test]
    fn hash_duplicate_skips_at_zero_attempts() {
        assert_eq!(
            skip_verdict(&row(REASON_HASH_DUPLICATE, 0), 1_000, MTIME),
            SkipVerdict::Skip
        );
    }

    #[test]
    fn gallery_hidden_skips_at_zero_attempts() {
        assert_eq!(
            skip_verdict(&row(REASON_GALLERY_HIDDEN, 0), 1_000, MTIME),
            SkipVerdict::Skip
        );
    }

    #[test]
    fn unplayable_skips_at_zero_attempts() {
        // A container with no decodable video stream (#46) is terminal on sight,
        // like a hash duplicate: no attempt budget, no retry, examined once.
        assert_eq!(
            skip_verdict(&row(REASON_UNPLAYABLE, 0), 1_000, MTIME),
            SkipVerdict::Skip
        );
    }

    #[test]
    fn an_unplayable_file_that_changed_on_disk_is_re_evaluated() {
        // The escape hatch still applies: replacing the file (perhaps with a
        // repaired copy that HAS a video stream) must un-retire it.
        assert_eq!(
            skip_verdict(&row(REASON_UNPLAYABLE, 0), 2_000, MTIME),
            SkipVerdict::Stale
        );
    }

    #[test]
    fn an_unknown_future_reason_is_terminal_not_retryable() {
        // The default must be the conservative one. A reason a later migration
        // adds should behave like 031's rows until someone deliberately opts it
        // into the retry path here.
        assert_eq!(
            skip_verdict(&row("some_future_reason", 0), 1_000, MTIME),
            SkipVerdict::Skip
        );
    }

    // ── The cap itself — this is the one that goes wrong ─────────────────

    #[test]
    fn conversion_failed_retries_below_the_cap() {
        // The trap: treating "row exists ⇒ skip" uniformly retires the file on
        // its FIRST failure, a silent one-strike cap wearing a three-strike
        // name. Each of these must be Retry, not Skip.
        for attempts in 0..CONVERSION_MAX_ATTEMPTS {
            assert_eq!(
                skip_verdict(&row(REASON_CONVERSION_FAILED, attempts), 1_000, MTIME),
                SkipVerdict::Retry,
                "attempt_count={attempts} must still be retried"
            );
        }
    }

    #[test]
    fn conversion_failed_skips_at_the_cap() {
        assert_eq!(
            skip_verdict(
                &row(REASON_CONVERSION_FAILED, CONVERSION_MAX_ATTEMPTS),
                1_000,
                MTIME
            ),
            SkipVerdict::Skip
        );
    }

    #[test]
    fn an_overshot_count_stays_retired() {
        // `==` instead of `>=` would wave this file through on every pass.
        assert_eq!(
            skip_verdict(
                &row(REASON_CONVERSION_FAILED, CONVERSION_MAX_ATTEMPTS + 7),
                1_000,
                MTIME
            ),
            SkipVerdict::Skip
        );
    }

    // ── Staleness outranks every verdict ────────────────────────────────

    #[test]
    fn a_resized_file_is_stale_even_when_retired() {
        // The escape hatch: replacing the file must un-retire it, or a user who
        // fixes a corrupt video has no way to get it converted short of a DB
        // edit.
        assert_eq!(
            skip_verdict(
                &row(REASON_CONVERSION_FAILED, CONVERSION_MAX_ATTEMPTS),
                2_000,
                MTIME
            ),
            SkipVerdict::Stale
        );
    }

    #[test]
    fn a_touched_file_is_stale_even_when_retired() {
        assert_eq!(
            skip_verdict(
                &row(REASON_CONVERSION_FAILED, CONVERSION_MAX_ATTEMPTS),
                1_000,
                Some("2026-07-22T00:00:00.000Z")
            ),
            SkipVerdict::Stale
        );
    }

    #[test]
    fn a_hash_duplicate_whose_file_changed_is_stale() {
        assert_eq!(
            skip_verdict(&row(REASON_HASH_DUPLICATE, 0), 999, MTIME),
            SkipVerdict::Stale
        );
    }

    #[test]
    fn a_row_that_lost_its_mtime_is_stale() {
        // `mtime` is nullable on both sides. `None == None` is unchanged;
        // `Some(_) != None` is a change. Comparing via `as_deref` keeps that
        // honest instead of unwrapping to a sentinel that collides with a real
        // value.
        assert_eq!(
            skip_verdict(&row(REASON_CONVERSION_FAILED, 0), 1_000, None),
            SkipVerdict::Stale
        );
    }

    #[test]
    fn a_row_with_no_mtime_matches_a_file_with_no_mtime() {
        let mut r = row(REASON_HASH_DUPLICATE, 0);
        r.mtime = None;
        assert_eq!(skip_verdict(&r, 1_000, None), SkipVerdict::Skip);
    }

    // ── Retirement predicate ────────────────────────────────────────────

    #[test]
    fn is_retired_only_at_or_above_the_cap() {
        assert!(!is_retired(0));
        assert!(!is_retired(CONVERSION_MAX_ATTEMPTS - 1));
        assert!(is_retired(CONVERSION_MAX_ATTEMPTS));
        assert!(is_retired(CONVERSION_MAX_ATTEMPTS + 1));
    }

    #[test]
    fn the_cap_and_the_retry_window_agree() {
        // The walk retries while `skip_verdict == Retry`; the failure path
        // announces retirement when `is_retired`. If those two ever disagree a
        // file is either retired while still being retried (a warning that lies)
        // or retried after being announced dead. Pin them to each other.
        for attempts in 0..=CONVERSION_MAX_ATTEMPTS + 2 {
            let retried = matches!(
                skip_verdict(&row(REASON_CONVERSION_FAILED, attempts), 1_000, MTIME),
                SkipVerdict::Retry
            );
            assert_eq!(
                retried,
                !is_retired(attempts),
                "attempt_count={attempts}: walk and audit disagree"
            );
        }
    }
}
