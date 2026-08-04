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
//!
//! ## Calibration — the seeds are a cold start, not a policy (#40)
//!
//! The compiled-in seed rates below only ever govern a category that has no
//! sample *yet*. That is meant to be the first pass on a new machine; without
//! persistence it is **every** pass, because the ledger is reset at both ends of
//! every batch. [`ConversionEta::calibrate`] installs a previously measured rate
//! as this machine's seed, and [`ConversionEta::measured_rate`] exports one to
//! be stored. The DB half lives in [`crate::conversion`] — this module stays
//! pure.

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

/// The band a *persisted* throughput has to land in to be believed.
///
/// A compiled-in seed cannot be wrong in these ways; a stored one can — it
/// survives crashes, hand edits, and whatever a future bug writes — and it feeds
/// a **division**. `NaN` is the nastiest of them: `rate <= 0.0` is `false` for
/// `NaN`, so it would sail straight through [`ConversionEta::eta_seconds`]'s
/// guard and produce a `NaN` ETA. The band also rules out the two silent
/// failures: an absurdly high rate pins the ETA at zero forever, an absurdly low
/// one quotes centuries.
///
/// 1 KiB/s is slower than any machine that can run ffmpeg at all; 10 GiB/s is
/// faster than the disk the input is read from. Anything outside is a corrupt
/// value wearing a plausible type, and the honest response is to fall back to
/// the seed.
const MIN_PLAUSIBLE_RATE: f64 = 1024.0;
const MAX_PLAUSIBLE_RATE: f64 = 10.0 * 1024.0 * MB;

/// Gatekeeper for every rate that crosses the process boundary — applied on the
/// way **out** to storage as well as on the way in, so a value that would be
/// refused on load is never written in the first place.
pub(crate) fn plausible_rate(bytes_per_sec: f64) -> Option<f64> {
    // `is_finite` first and separately: it is the guard that matters (a `NaN`
    // makes every comparison below false anyway, but saying so explicitly is
    // the point) and folding it into a range check would hide it.
    if bytes_per_sec.is_finite()
        && (MIN_PLAUSIBLE_RATE..=MAX_PLAUSIBLE_RATE).contains(&bytes_per_sec)
    {
        Some(bytes_per_sec)
    } else {
        None
    }
}

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

pub(crate) const CATEGORIES: [MediaCategory; 3] = [
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
    /// Unsmoothed running totals over **exactly** the samples folded into the
    /// EWMAs above. Their ratio is this batch's true average throughput, which
    /// is what gets persisted as the next boot's seed — see
    /// [`CategoryProgress::cumulative_rate`] for why the EWMA is the wrong thing
    /// to store.
    ///
    /// Deliberately not derived from `done_weight`: that is charged on paths
    /// that produce no measurement (a completion with no start, a non-positive
    /// delta), so deriving it would give the calibration and the EWMA two
    /// different definitions of "a sample" — the one-list-two-derivations trap
    /// this repo has six recorded instances of.
    sampled_weight: f64,
    sampled_secs: f64,
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
    ///
    /// `calibrated` is the rate this machine measured on an earlier pass, if one
    /// was ever stored. It stands in for the compiled-in seed and nothing else:
    /// once this batch has a sample of its own, that sample wins outright, so
    /// in-batch behaviour is unchanged by calibration being present.
    fn rate(&self, cat: MediaCategory, calibrated: Option<f64>) -> f64 {
        if self.samples > 0 && self.ewma_secs > 0.0 && self.ewma_weight > 0.0 {
            self.ewma_weight / self.ewma_secs
        } else {
            calibrated.unwrap_or_else(|| seed_rate(cat))
        }
    }

    /// This batch's **average** throughput, `Σweight / Σsecs`, or `None` when
    /// nothing was measured. This — not [`CategoryProgress::rate`] — is what
    /// gets persisted, for two reasons:
    ///
    /// 1. The EWMA is deliberately recency-biased, which is right for tracking a
    ///    machine that is throttling *now* and wrong for describing what that
    ///    machine generally does.
    /// 2. It is immune to the burst artefact a wide lane produces, and that
    ///    artefact is not small. A video lane of width N starts N encodes
    ///    together, so sample 1 charges one file's weight against the whole
    ///    phase and samples 2..N arrive with near-zero deltas. Each of those
    ///    decays `ewma_secs` by a further 0.65 while `ewma_weight` does not
    ///    move, so what the EWMA reports at the end of a burst is a function of
    ///    the lane width and **not** of the machine: it under-reads a narrow
    ///    lane and over-reads a wide one (8 wide ⇒ ~2.5× *fast*, measured in
    ///    `the_persisted_rate_is_the_batch_average_not_the_ewma`). Over-reading
    ///    is the dangerous direction to store, because the resulting ETA starts
    ///    short and grows.
    ///
    ///    None of that is a defect in the EWMA — mid-burst the *recent* rate
    ///    genuinely is high, which is what in-batch tracking wants. It is a
    ///    reason not to persist it. `Σweight / Σsecs` reads
    ///    `N·weight / phase` — the true throughput — from the moment the burst
    ///    lands, whatever N is.
    fn cumulative_rate(&self) -> Option<f64> {
        if self.samples == 0 || self.sampled_secs <= 0.0 || self.sampled_weight <= 0.0 {
            return None;
        }
        plausible_rate(self.sampled_weight / self.sampled_secs)
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
    /// Per-category throughput measured by an **earlier** batch, installed by
    /// [`ConversionEta::calibrate`]. Machine calibration, not batch state — see
    /// [`ConversionEta::reset`].
    calibration: [Option<f64>; 3],
}

impl ConversionEta {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Forget the current batch. Called when a batch starts or ends so a stale
    /// ledger cannot leak its weights into the next pass's estimate.
    ///
    /// **Keeps the calibration on purpose — do not "simplify" this back to
    /// `*self = Self::new()`.** This runs at both ends of every batch, so
    /// wiping the calibration here would put every pass after the first back on
    /// the compiled-in seeds within a single boot, which is the entire defect
    /// this half of #40 exists to remove. What is per-batch is the queue;
    /// what the machine can do is not.
    pub(crate) fn reset(&mut self) {
        self.cats = Default::default();
    }

    /// Install a previously measured throughput as this machine's seed for
    /// `cat`. Returns `false` — and changes nothing — when the value is not a
    /// plausible rate, so a corrupt stored value degrades to the compiled-in
    /// seed rather than poisoning the estimate.
    pub(crate) fn calibrate(&mut self, cat: MediaCategory, bytes_per_sec: f64) -> bool {
        match plausible_rate(bytes_per_sec) {
            Some(rate) => {
                self.calibration[index(cat)] = Some(rate);
                true
            }
            None => false,
        }
    }

    /// This batch's measured average throughput for `cat`, ready to persist.
    /// `None` when the category was never sampled — which is the common case
    /// (an image-only pass measures no video) and is why the caller must write
    /// per category rather than writing all three: overwriting a good video rate
    /// with "nothing" would undo the calibration on every images-only scan.
    pub(crate) fn measured_rate(&self, cat: MediaCategory) -> Option<f64> {
        self.cats[index(cat)].cumulative_rate()
    }

    /// True when no work has been enqueued — the signal callers use to fall back
    /// to the count-based estimator.
    ///
    /// Keyed on enqueued weight alone, and it must stay that way: a calibrated
    /// ledger with an empty queue is still empty. The client-declared upload
    /// batch enqueues nothing here, and if calibration made this read non-empty
    /// that path would lose its count-based ETA entirely.
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
        // Same event, unsmoothed — the calibration figure that outlives the
        // batch. Accumulated here rather than anywhere else so it can only ever
        // count what the EWMA counted.
        c.sampled_weight += weight;
        c.sampled_secs += delta;
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
            let rate = c.rate(cat, self.calibration[index(cat)]);
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
        let after = eta
            .eta_seconds()
            .expect("estimate with the video tail left");

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

    // ── Cross-pass calibration (#40 remainder) ──────────────────────────────

    /// The headline of this half: a machine that has converted video once must
    /// not go back to the compiled-in seed on the next pass.
    ///
    /// Verified RED by making `rate` ignore its `calibrated` argument: the
    /// estimate returns to the 2 MB/s seed and reads ~500s.
    #[test]
    fn a_calibrated_rate_replaces_the_seed() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        assert!(eta.calibrate(MediaCategory::Video, 20.0 * MB));
        eta.enqueue(MediaCategory::Video, 1000 * mb);

        // 1000 MB at the calibrated 20 MB/s ⇒ ~50s. The seed would say ~500s.
        let remaining = eta.eta_seconds().expect("1000 MB outstanding");
        assert!(
            (remaining - 50.0).abs() < 2.0,
            "expected the calibrated rate to govern, got {remaining:.1}s \
             (the 2 MB/s seed lands near 500s)"
        );
    }

    /// Calibration is a better *seed*, not a competing estimator. The instant
    /// this batch has evidence of its own, that evidence wins — the same
    /// "measurement replaces the seed, never blends with it" rule the seeds
    /// already follow.
    #[test]
    fn this_batch_s_own_measurement_outranks_the_calibration() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        // A stale calibration claiming this box is very fast.
        assert!(eta.calibrate(MediaCategory::Video, 100.0 * MB));
        eta.enqueue(MediaCategory::Video, 1010 * mb);
        eta.start(MediaCategory::Video, 0.0);
        // Reality today: 10 MB in 10s ⇒ 1 MB/s.
        eta.complete(MediaCategory::Video, 10 * mb, 10.0);

        let remaining = eta.eta_seconds().expect("1000 MB outstanding");
        assert!(
            remaining > 500.0,
            "a live sample must displace the stored calibration, got \
             {remaining:.1}s (the stale 100 MB/s reads ~10s)"
        );
    }

    /// **The sharp edge.** `reset` runs at both ends of every batch. If it wiped
    /// the calibration, this feature would do nothing even within one boot.
    ///
    /// Verified RED by restoring `*self = Self::new()`: the second estimate
    /// falls back to the seed and reads ~500s.
    #[test]
    fn a_reset_drops_the_queue_and_keeps_the_calibration() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        assert!(eta.calibrate(MediaCategory::Video, 20.0 * MB));
        eta.enqueue(MediaCategory::Video, 4000 * mb);

        eta.reset();
        assert!(eta.is_empty(), "the queue must not survive a reset");
        assert!(
            eta.eta_seconds().is_none(),
            "the previous batch's 4 GB tail leaked past the reset"
        );

        eta.enqueue(MediaCategory::Video, 1000 * mb);
        let remaining = eta.eta_seconds().expect("1000 MB outstanding");
        assert!(
            (remaining - 50.0).abs() < 2.0,
            "the calibration must survive the reset, got {remaining:.1}s \
             (the seed lands near 500s)"
        );
    }

    /// A calibrated ledger with nothing queued is still *empty*. The
    /// client-declared upload batch enqueues no weight and depends on this to
    /// fall through to the count-based estimator.
    #[test]
    fn calibration_alone_does_not_make_the_ledger_look_busy() {
        let mut eta = ConversionEta::new();
        assert!(eta.calibrate(MediaCategory::Video, 20.0 * MB));
        assert!(eta.is_empty());
        assert!(eta.eta_seconds().is_none());
    }

    /// **What gets persisted is the batch average, not the EWMA**, and this is
    /// the trace that separates them.
    ///
    /// A video lane eight wide: all eight encodes start together and finish in
    /// a burst at ~100s, so true throughput is 800 MB / 100s = 8 MB/s. Each of
    /// the seven near-zero deltas decays `ewma_secs` by 0.65 while
    /// `ewma_weight` stays at one file's weight, so the EWMA ends the burst at
    /// ~20 MB/s — a figure derived from the lane width rather than from the
    /// machine. The running totals read 8 MB/s.
    ///
    /// Verified RED by exporting `rate()` instead: the stored figure comes back
    /// at **20.4 MB/s, 2.5× optimistic**, which is the bad direction — every
    /// later boot would open with an ETA that is too short and then grows.
    #[test]
    fn the_persisted_rate_is_the_batch_average_not_the_ewma() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Video, 800 * mb);
        eta.start(MediaCategory::Video, 0.0);

        // One long delta, then the rest of the lane landing together.
        eta.complete(MediaCategory::Video, 100 * mb, 100.0);
        for i in 1..8 {
            eta.complete(MediaCategory::Video, 100 * mb, 100.0 + f64::from(i) * 0.01);
        }

        let stored = eta
            .measured_rate(MediaCategory::Video)
            .expect("eight samples were taken");
        let true_rate = 800.0 * MB / 100.07;
        assert!(
            (stored / true_rate - 1.0).abs() < 0.05,
            "expected ~{:.2} MB/s (800 MB in 100s), got {:.2} MB/s",
            true_rate / MB,
            stored / MB
        );
    }

    /// An unsampled category exports nothing, so an images-only pass cannot
    /// clobber the video rate a mixed pass measured last week.
    #[test]
    fn an_unsampled_category_has_nothing_to_persist() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Image, 100 * mb);
        eta.enqueue(MediaCategory::Video, 100 * mb);
        eta.start(MediaCategory::Image, 0.0);
        eta.complete(MediaCategory::Image, 50 * mb, 1.0);

        assert!(eta.measured_rate(MediaCategory::Image).is_some());
        assert!(
            eta.measured_rate(MediaCategory::Video).is_none(),
            "no video finished, so there is no video rate to write"
        );
        assert!(
            eta.measured_rate(MediaCategory::Audio).is_none(),
            "no audio was even queued"
        );
    }

    /// A stored rate crosses a process boundary, so it can be corrupt in ways a
    /// constant cannot. `NaN` is the one that matters: `rate <= 0.0` is `false`
    /// for `NaN`, so an unguarded one reaches the division and yields a `NaN`
    /// ETA.
    #[test]
    fn an_implausible_calibration_is_refused_and_the_seed_stands() {
        let mb = 1024 * 1024;
        for bad in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -1.0,
            0.5,  // half a byte per second ⇒ centuries
            1e15, // faster than any disk ⇒ ETA pinned at 0
        ] {
            let mut eta = ConversionEta::new();
            assert!(
                !eta.calibrate(MediaCategory::Video, bad),
                "{bad} was accepted as a throughput"
            );
            eta.enqueue(MediaCategory::Video, 1000 * mb);
            let remaining = eta.eta_seconds().expect("1000 MB outstanding");
            assert!(
                remaining.is_finite() && (remaining - 500.0).abs() < 5.0,
                "a refused calibration must leave the 2 MB/s seed in place; \
                 {bad} produced {remaining}"
            );
        }
    }

    /// The same gate applies on the way out — a measurement that could not be
    /// read back is not worth writing. The reachable case is a lone sample
    /// landing inside the clock's resolution: `complete` only discards a
    /// *non-positive* delta, so a 1 ms one survives and reads as ~100 GB/s.
    #[test]
    fn an_implausible_measurement_is_not_exported() {
        let mb = 1024 * 1024;
        let mut eta = ConversionEta::new();
        eta.enqueue(MediaCategory::Image, 200 * mb);
        eta.start(MediaCategory::Image, 0.0);
        eta.complete(MediaCategory::Image, 100 * mb, 0.001);
        assert!(
            eta.measured_rate(MediaCategory::Image).is_none(),
            "a 100 GB/s clock artefact must not be persisted as this box's rate"
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
