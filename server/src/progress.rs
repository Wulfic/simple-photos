//! Pure progress / ETA estimation, shared by the encryption banner and the
//! conversion banner. No DB, no wall-clock, no globals — every entry point takes
//! its timestamps as arguments so the whole module is unit-testable.
//!
//! Two estimators live here, and the difference between them is the point:
//!
//! * [`progress_math`] — count-based cumulative mean. Correct when every queue
//!   item costs roughly the same (the encryption banner: one photo each) and
//!   when the caller has nothing but a count (the client-declared upload batch,
//!   whose wire shape carries no sizes).
//! * [`ConversionEta`] — work-weighted, per-category. Required by the conversion
//!   queue, which deliberately mixes categories whose per-item costs differ by
//!   orders of magnitude.
//!
//! ## Why the conversion queue needs its own estimator (#40)
//!
//! `progress_math` treats every queue item as equal cost. The conversion pass
//! sorts images and audio ahead of video ([`crate::conversion::conversion_priority`])
//! and drains the fast lane to exhaustion before the video lane starts, so a
//! mixed import spends its entire first phase learning a per-item cost that is
//! orders of magnitude too small — and then hits the video tail, where the ETA
//! explodes. It is also *cumulative*, so those early images bias the estimate
//! for the rest of the batch.
//!
//! `ConversionEta` fixes all three: it measures **weight**, keeps a separate
//! throughput estimate **per category**, and uses a sliding (EWMA) rate rather
//! than the batch-lifetime mean.

use crate::conversion::MediaCategory;

const MB: f64 = 1024.0 * 1024.0;

/// Pure throughput math for a progress ETA. Given the batch denominator, the
/// current pending count, and how long the batch has been running, compute
/// `(done, eta_seconds)`. `eta` is `None` until at least one item has finished
/// (no throughput sample yet).
///
/// This is the **count-based** estimator. It is right only when queue items are
/// interchangeable; see [`ConversionEta`] for the weighted one and the module
/// docs for which caller gets which.
pub(crate) fn progress_math(
    batch_total: i64,
    total_pending: i64,
    elapsed_secs: f64,
) -> (i64, Option<f64>) {
    let done = (batch_total - total_pending).max(0);
    let eta = if done > 0 && elapsed_secs > 0.0 {
        let per_item = elapsed_secs / done as f64;
        Some((total_pending as f64) * per_item)
    } else {
        None
    };
    (done, eta)
}

// ── Work-weighted conversion ETA (#40) ───────────────────────────────────────

/// Seed throughput for a category with no completed sample yet, in bytes of
/// *input* consumed per wall-clock second.
///
/// A seed is not a nicety — it is structurally required. The video category has
/// zero samples for the whole image phase, which is exactly the window in which
/// the old estimator was most wrong, so an estimator that refuses to guess would
/// print nothing precisely when the user most wants a number. The first real
/// sample **replaces** a seed outright rather than blending with it: the seed
/// carries no evidence about this machine, so there is nothing to average.
///
/// These are order-of-magnitude figures, and their only job is to stop the
/// image→video cliff. They are deliberately conservative (slow), because an ETA
/// that overestimates and shrinks reads far better than one that grows.
const IMAGE_SEED_BYTES_PER_SEC: f64 = 40.0 * MB;
const AUDIO_SEED_BYTES_PER_SEC: f64 = 20.0 * MB;
/// The one that matters. A software x264 transcode of a 4K source consumes
/// roughly 0.5–1 MB/s of input; a hardware encoder is several times that. Seeded
/// near the low end on purpose (see above).
const VIDEO_SEED_BYTES_PER_SEC: f64 = 2.0 * MB;

/// EWMA smoothing factor for the throughput samples. High enough that a handful
/// of samples dominates — the batches here are often only a few videos long, so
/// a slow filter would still be reporting the seed when the queue drained.
const EWMA_ALPHA: f64 = 0.35;

fn seed_rate(cat: MediaCategory) -> f64 {
    match cat {
        MediaCategory::Image => IMAGE_SEED_BYTES_PER_SEC,
        MediaCategory::Audio => AUDIO_SEED_BYTES_PER_SEC,
        MediaCategory::Video => VIDEO_SEED_BYTES_PER_SEC,
    }
}

fn index(cat: MediaCategory) -> usize {
    match cat {
        MediaCategory::Image => 0,
        MediaCategory::Audio => 1,
        MediaCategory::Video => 2,
    }
}

const CATEGORIES: [MediaCategory; 3] = [
    MediaCategory::Image,
    MediaCategory::Audio,
    MediaCategory::Video,
];

/// One category's queued work and its measured throughput.
#[derive(Debug, Clone, Default)]
struct CategoryProgress {
    /// Bytes of input enqueued for this category in the current batch.
    total_weight: f64,
    /// Bytes charged as completed — on **success and failure alike**. A failed
    /// transcode still consumed the wall-clock it is being measured against, so
    /// omitting it would make the rate climb silently as failures accumulate.
    done_weight: f64,
    /// Numerator and denominator of the throughput estimate, each smoothed
    /// separately. See [`CategoryProgress::rate`] for why it is a ratio of two
    /// EWMAs rather than an EWMA of ratios.
    ewma_weight: f64,
    ewma_secs: f64,
    samples: u32,
    /// When the first file of this category began converting. Seeds the delta
    /// for the first sample, which otherwise has no predecessor to measure from.
    phase_started_at: Option<f64>,
    /// Timestamp of the most recent event in this category.
    last_event_at: Option<f64>,
}

impl CategoryProgress {
    /// Throughput in bytes/sec: measured once there is a sample, seeded before.
    ///
    /// The measured form is `EWMA(weight) / EWMA(secs)`, **not** `EWMA(weight /
    /// secs)`. The latter is a time-unweighted mean of instantaneous rates and
    /// is biased high by short deltas — and short deltas are exactly what a
    /// wide lane produces, because N concurrent encodes finish in a burst. The
    /// ratio-of-EWMAs form converges to `Σweight / Σtime`, i.e. real throughput.
    fn rate(&self, cat: MediaCategory) -> f64 {
        if self.samples > 0 && self.ewma_secs > 0.0 && self.ewma_weight > 0.0 {
            self.ewma_weight / self.ewma_secs
        } else {
            seed_rate(cat)
        }
    }

    fn remaining_weight(&self) -> f64 {
        (self.total_weight - self.done_weight).max(0.0)
    }
}

/// Work-weighted, per-category ETA for the conversion queue (#40).
///
/// Fed by the conversion pass, which knows each candidate's category and size
/// up front. Deliberately **parallel to**, not a replacement for, the
/// `CONV_TOTAL`/`CONV_DONE` counters: those are counts, they drive the "3 / 4"
/// banner text, and they carry #11's pinned-denominator fix. Rewriting them in
/// terms of bytes would reopen that.
///
/// ### Weight is bytes, for every category
///
/// The obvious refinement — weight video by *duration* rather than bytes —
/// looks better and is a trap. Duration is only known for the containers that
/// went through [`crate::transcode::probe`] (`.mp4`/`.mov`/`.m4v`/`.webm`);
/// `.mkv`/`.avi`/`.wmv` are matched by extension and never probed. That would
/// make the unit inconsistent *within* the video category, and a rate estimator
/// whose denominator silently changes units between samples is worse than one
/// that is merely coarse. Bytes are known for every candidate, and because the
/// rate is tracked per category the unit only ever has to be comparable with
/// itself — the units cancel before the per-category times are summed.
#[derive(Debug, Clone, Default)]
pub(crate) struct ConversionEta {
    cats: [CategoryProgress; 3],
}

impl ConversionEta {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Forget the current batch. Called when a batch starts or ends so a stale
    /// ledger cannot leak its weights into the next pass's estimate.
    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }

    /// True when no work has been enqueued — the signal callers use to fall back
    /// to the count-based estimator.
    pub(crate) fn is_empty(&self) -> bool {
        self.cats.iter().all(|c| c.total_weight <= 0.0)
    }

    /// Enqueue one candidate's work. Called once per candidate when the pass
    /// enumerates its batch, before any conversion starts.
    pub(crate) fn enqueue(&mut self, cat: MediaCategory, weight_bytes: i64) {
        // A zero/negative size (an unstatable file) must not subtract work.
        self.cats[index(cat)].total_weight += (weight_bytes.max(0)) as f64;
    }

    /// Mark that a file of this category has begun converting. Only the first
    /// call per category does anything: it starts the wall-clock the first
    /// throughput sample is measured against.
    pub(crate) fn start(&mut self, cat: MediaCategory, now: f64) {
        let c = &mut self.cats[index(cat)];
        if c.phase_started_at.is_none() {
            c.phase_started_at = Some(now);
            c.last_event_at = Some(now);
        }
    }

    /// Charge one completed file — **success or failure**, see
    /// [`CategoryProgress::done_weight`] — and fold a throughput sample.
    pub(crate) fn complete(&mut self, cat: MediaCategory, weight_bytes: i64, now: f64) {
        let c = &mut self.cats[index(cat)];
        let weight = (weight_bytes.max(0)) as f64;
        c.done_weight += weight;

        // A completion with no recorded start is not a measurement — treat it as
        // this category's phase start so the *next* one is.
        let previous = match c.last_event_at.or(c.phase_started_at) {
            Some(t) => t,
            None => {
                c.phase_started_at = Some(now);
                c.last_event_at = Some(now);
                return;
            }
        };
        c.last_event_at = Some(now);

        let delta = now - previous;
        // A non-positive delta carries no rate information (two completions
        // inside one clock tick, or a clock that went backwards). Charging the
        // weight but skipping the sample is the honest handling; folding it
        // would divide by ~zero and report an infinite throughput.
        if delta <= 0.0 || weight <= 0.0 {
            return;
        }

        if c.samples == 0 {
            // First evidence about this machine replaces the seed outright.
            c.ewma_weight = weight;
            c.ewma_secs = delta;
        } else {
            c.ewma_weight = EWMA_ALPHA * weight + (1.0 - EWMA_ALPHA) * c.ewma_weight;
            c.ewma_secs = EWMA_ALPHA * delta + (1.0 - EWMA_ALPHA) * c.ewma_secs;
        }
        c.samples += 1;
    }

    /// Estimated seconds remaining for the whole batch: the sum of each
    /// category's remaining weight divided by that category's throughput.
    ///
    /// Summing (rather than taking a maximum) is right because the pass runs the
    /// fast lane to exhaustion *before* the video lane — the phases are serial.
    /// Image and audio do overlap inside the fast lane, so their two remainders
    /// are summed when they in truth partly run concurrently; that overestimates
    /// by a fraction of the fast phase, which is the short one and the safe
    /// direction.
    ///
    /// `None` when nothing is queued or nothing is left, so the caller can fall
    /// back to the count-based estimator rather than render a bare `0`.
    pub(crate) fn eta_seconds(&self) -> Option<f64> {
        if self.is_empty() {
            return None;
        }
        let mut eta = 0.0;
        let mut any_remaining = false;
        for cat in CATEGORIES {
            let c = &self.cats[index(cat)];
            let remaining = c.remaining_weight();
            if remaining <= 0.0 {
                continue;
            }
            any_remaining = true;
            let rate = c.rate(cat);
            if rate <= 0.0 {
                // Cannot happen with the seeds above, but a zero rate would be
                // an infinite ETA — refuse rather than emit one.
                return None;
            }
            eta += remaining / rate;
        }
        if any_remaining {
            Some(eta)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Count-based estimator (unchanged behaviour, moved from status.rs) ────

    #[test]
    fn no_done_yet_has_no_eta() {
        // Nothing finished → no throughput sample → ETA unknown.
        let (done, eta) = progress_math(100, 100, 5.0);
        assert_eq!(done, 0);
        assert!(eta.is_none());
    }

    #[test]
    fn eta_scales_with_remaining() {
        // 10 of 100 done in 10s ⇒ 1s/item ⇒ 90 remaining ⇒ ~90s.
        let (done, eta) = progress_math(100, 90, 10.0);
        assert_eq!(done, 10);
        assert!((eta.unwrap() - 90.0).abs() < 1e-6);
    }

    #[test]
    fn done_is_clamped_non_negative() {
        // Pending briefly exceeds the denominator (race) — done floors at 0.
        let (done, eta) = progress_math(50, 60, 5.0);
        assert_eq!(done, 0);
        assert!(eta.is_none());
    }

    #[test]
    fn zero_elapsed_yields_no_eta() {
        // Guard against divide-by-zero on the very first tick.
        let (done, eta) = progress_math(100, 50, 0.0);
        assert_eq!(done, 50);
        assert!(eta.is_none());
    }

    // ── Weighted conversion estimator (#40) ─────────────────────────────────

    /// The headline defect. A mixed queue converts 100 images then 4 videos; the
    /// ETA must not collapse the moment the images run out.
    ///
    /// This is the shape `conversion_priority` guarantees (images first, video
    /// last) and it is what made the old estimator explode: at the boundary it
    /// had learned a per-item cost drawn *entirely* from images and applied it
    /// to a tail of videos costing ~1000× more each.
    #[test]
    fn mixed_queue_eta_does_not_swing_across_the_category_boundary() {
        let img = 5 * 1024 * 1024; // 5 MB stills
        let vid = 500 * 1024 * 1024; // 500 MB videos

        let mut eta = ConversionEta::new();
        for _ in 0..100 {
            eta.enqueue(MediaCategory::Image, img);
        }
        for _ in 0..4 {
            eta.enqueue(MediaCategory::Video, vid);
        }

        // Fast lane: 100 images at 0.1s apart ⇒ ~50 MB/s measured.
        let mut t = 0.0;
        eta.start(MediaCategory::Image, t);
        for _ in 0..99 {
            t += 0.1;
            eta.complete(MediaCategory::Image, img, t);
        }
        let before = eta.eta_seconds().expect("estimate with work outstanding");

        // The last image lands, and the queue crosses into the video tail.
        t += 0.1;
        eta.complete(MediaCategory::Image, img, t);
        let after = eta.eta_seconds().expect("estimate with the video tail left");

        let ratio = if before > after {
            before / after
        } else {
            after / before
        };
        assert!(
            ratio < 2.0,
            "ETA swung {ratio:.1}× across the image→video boundary \
             ({before:.0}s → {after:.0}s); #40 requires < 2×"
        );

        // And it must be in the right ballpark, not merely stable: 2 GB of video
        // at the seeded ~2 MB/s is ~1000s. A "stable" estimator that reported
        // half a second at both samples would also pass the ratio check.
        assert!(
            after > 300.0,
            "video tail of 2 GB must dominate the estimate, got {after:.1}s"
        );
    }

    /// Pins *why* the above works, by running the same trace through the old
    /// count-based estimator. If this ever stops failing to swing, the mixed
    /// queue is no longer the shape the fix was built for.
    #[test]
    fn the_count_based_estimator_is_the_thing_being_replaced() {
        // 104 items, 100 done, 10s elapsed — the state at the boundary above.
        let (_done, eta) = progress_math(104, 4, 10.0);
        let eta = eta.expect("count-based estimator has samples here");
        // ~0.4s, for a queue with 2 GB of 4K video still to transcode.
        assert!(
            eta < 1.0,
            "the count-based estimator should be wildly optimistic here \
             (that is the bug); got {eta:.1}s"
        );
    }

    #[test]
    fn weights_within_a_category_are_not_treated_as_equal() {
        // One 10 MB clip and one 1000 MB clip. After the small one completes,
        // the remaining estimate must reflect the *large* file's weight, not
        // "one item, same cost as the last".
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Video, 10 * 1024 * 1024);
        eta.enqueue(MediaCategory::Video, 1000 * 1024 * 1024);

        eta.start(MediaCategory::Video, 0.0);
        eta.complete(MediaCategory::Video, 10 * 1024 * 1024, 10.0);

        // Measured 1 MB/s ⇒ 1000 MB left ⇒ ~1000s. A count-based estimator would
        // say "1 of 2 done in 10s ⇒ 10s left".
        let remaining = eta.eta_seconds().expect("one file outstanding");
        assert!(
            remaining > 500.0,
            "the large file's weight must dominate, got {remaining:.1}s"
        );
    }

    #[test]
    fn measured_throughput_replaces_the_seed() {
        let mb = 1024 * 1024;
        let mut seeded = ConversionEta::new();
        seeded.enqueue(MediaCategory::Video, 100 * mb);
        let from_seed = seeded.eta_seconds().unwrap();

        // Same queue, but this machine is measured at ~10 MB/s — five times the
        // seed — so the estimate must move a long way toward the measurement.
        let mut measured = ConversionEta::new();
        measured.enqueue(MediaCategory::Video, 110 * mb);
        measured.start(MediaCategory::Video, 0.0);
        measured.complete(MediaCategory::Video, 10 * mb, 1.0);
        let from_measurement = measured.eta_seconds().unwrap();

        assert!(
            from_measurement < from_seed / 2.0,
            "a measured rate must displace the seed, not average with it \
             (seed {from_seed:.0}s vs measured {from_measurement:.0}s)"
        );
    }

    /// The estimator must not be a cumulative mean: a machine that slows down
    /// (thermal throttling, the 8K files sorted to the end) has to be tracked.
    #[test]
    fn the_rate_is_sliding_not_cumulative() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        // 24 files of 10 MB are converted below, against a 400 MB queue — the
        // queue must still have work outstanding at BOTH measurements or the
        // second one is `None` rather than a slower estimate.
        eta.enqueue(MediaCategory::Video, 400 * mb);
        eta.start(MediaCategory::Video, 0.0);

        let mut t = 0.0;
        for _ in 0..20 {
            t += 1.0; // 10 MB/s
            eta.complete(MediaCategory::Video, 10 * mb, t);
        }
        let while_fast = eta.eta_seconds().unwrap();

        // Now it slows to 1 MB/s for several files.
        for _ in 0..4 {
            t += 10.0;
            eta.complete(MediaCategory::Video, 10 * mb, t);
        }
        let while_slow = eta.eta_seconds().unwrap();

        // "It went up" is NOT enough — a cumulative mean also goes up here
        // (10 MB/s → 4 MB/s ⇒ 20s → 40s), so a `while_slow > while_fast`
        // assertion passes against the very implementation this test exists to
        // reject. The estimate must track the RECENT rate: 160 MB remaining at
        // the new ~1 MB/s is ~160s, versus ~40s for the cumulative blend.
        assert!(
            while_slow > 90.0,
            "the estimate must follow the recent rate, not the batch-lifetime \
             mean; got {while_fast:.1}s → {while_slow:.1}s (a cumulative mean \
             lands near 40s here)"
        );
    }

    /// Pins the *form* of the rate estimator, which is the part most likely to
    /// be "simplified" into a bug later.
    ///
    /// A wide lane finishes a large file and then a tiny one almost immediately
    /// after. True throughput over that window is ~10 MB/s. An `EWMA(weight /
    /// secs)` — the obvious implementation — averages `10 MB/s` with the tiny
    /// file's instantaneous `100 MB/s` and reports ~40 MB/s, a 4× overestimate
    /// of throughput and so a 4× *under*estimate of the ETA. `EWMA(weight) /
    /// EWMA(secs)` stays put.
    #[test]
    fn the_rate_is_a_ratio_of_averages_not_an_average_of_ratios() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Video, 1101 * mb);
        eta.start(MediaCategory::Video, 0.0);

        eta.complete(MediaCategory::Video, 100 * mb, 10.0); // 10 MB/s
        eta.complete(MediaCategory::Video, mb, 10.01); // 100 MB/s instantaneous

        // 1000 MB left at the true ~10 MB/s ⇒ ~100s. An average-of-ratios
        // estimator reports ~24s here.
        let remaining = eta.eta_seconds().expect("1000 MB outstanding");
        assert!(
            (remaining - 100.0).abs() < 15.0,
            "expected ~100s from true throughput; got {remaining:.1}s \
             (a burst of short deltas must not inflate the rate)"
        );
    }

    #[test]
    fn a_failed_file_still_charges_its_weight() {
        // `process_candidate` ticks on both arms. If failures were not charged,
        // remaining weight would never drain and the ETA would never reach zero.
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Video, 100 * mb);
        eta.start(MediaCategory::Video, 0.0);
        eta.complete(MediaCategory::Video, 100 * mb, 50.0);
        assert!(
            eta.eta_seconds().is_none(),
            "a fully-charged batch has nothing outstanding"
        );
    }

    #[test]
    fn an_empty_ledger_defers_to_the_caller() {
        // The client-declared upload batch enqueues nothing here, so the caller
        // must be able to tell and fall back to `progress_math`.
        let eta = ConversionEta::new();
        assert!(eta.is_empty());
        assert!(eta.eta_seconds().is_none());
    }

    #[test]
    fn zero_and_negative_deltas_do_not_produce_an_infinite_rate() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Image, 100 * mb);
        eta.start(MediaCategory::Image, 10.0);
        // Same instant as the start, then a clock that went backwards.
        eta.complete(MediaCategory::Image, 10 * mb, 10.0);
        eta.complete(MediaCategory::Image, 10 * mb, 9.0);
        let remaining = eta.eta_seconds().expect("80 MB still outstanding");
        assert!(
            remaining.is_finite() && remaining > 0.0,
            "degenerate deltas must not yield {remaining}"
        );
    }

    #[test]
    fn an_unstatable_file_cannot_subtract_work() {
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Image, 10 * 1024 * 1024);
        eta.enqueue(MediaCategory::Image, -1);
        eta.start(MediaCategory::Image, 0.0);
        eta.complete(MediaCategory::Image, -1, 1.0);
        assert!(
            eta.eta_seconds().is_some(),
            "a negative size must not drain the queue"
        );
    }

    #[test]
    fn categories_are_summed_because_the_lanes_are_serial() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Image, 100 * mb);
        eta.enqueue(MediaCategory::Video, 100 * mb);

        // Seeded: 100 MB of images at 40 MB/s (2.5s) + 100 MB of video at
        // 2 MB/s (50s) ⇒ ~52.5s, dominated by the video tail.
        let total = eta.eta_seconds().unwrap();
        assert!(
            (total - 52.5).abs() < 1.0,
            "expected the per-category remainders to sum, got {total:.1}s"
        );
    }

    #[test]
    fn a_completion_with_no_start_is_not_a_measurement() {
        // Defensive: `complete` before `start` must not fabricate a sample from
        // a zero baseline.
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Video, 100 * mb);
        eta.complete(MediaCategory::Video, 10 * mb, 123.0);
        let remaining = eta.eta_seconds().unwrap();
        // Still on the seed: 90 MB at 2 MB/s ⇒ 45s.
        assert!(
            (remaining - 45.0).abs() < 1.0,
            "expected the seed rate to still apply, got {remaining:.1}s"
        );
    }
}
