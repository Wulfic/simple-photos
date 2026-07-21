//! Resolution-ladder planning: which renditions a source video should produce.
//!
//! Pure arithmetic. No ffmpeg, no database, no device. That is deliberate — the
//! rule below was wrong twice in the planning document, and both errors are
//! catchable with nothing but integers. Anything that needs a transcode to
//! verify does not belong in this file.
//!
//! **The rule keys on the SHORT EDGE, not on height.** Measured against the
//! live 742-video library (2026-07-20), a `height > 1080` test is wrong twice
//! and the library contains both traps:
//!
//! - **71 videos are portrait, 14 of them exactly `1080x1920`.** A height test
//!   flags every one — but `1080x1920` *is* the 1080p tier. Naively applying
//!   `scale=-2:1080` would **downscale them to 608×1080**, degrading files that
//!   needed no rung at all.
//! - **4 videos are `2288x1088`.** 1088 is macroblock-padded 1080. A strict
//!   `> 1080` test spends a full 4K-class re-encode to save 8 pixels, so the
//!   rule needs a *tolerance band*, not `>`.
//!
//! Short-edge sizing over the live library: 140 sources exceed 1080 strictly,
//! of which the 4 padded ones must be excluded ⇒ **true demand 136**, dominated
//! by 4K. [`tests::live_library_census_yields_the_measured_136`] reproduces that
//! number from the census, so the arithmetic is pinned to the measurement
//! rather than to an assertion about it.

/// Short edge of the 1080p tier.
pub const TIER_1080_SHORT_EDGE: i64 = 1080;

/// How far above a tier a source may sit before it is worth a separate rung.
///
/// A source within this band of a tier *is* that tier for practical purposes.
/// Set from the `2288x1088` case: 1088 exceeds 1080 by 0.74%, and re-encoding
/// a 4K-class file to reclaim 8 pixels of height is pure waste. 10% is well
/// clear of that while still catching `1920x1440` (33% over), the smallest
/// genuine case in the live library.
pub const TIER_TOLERANCE: f64 = 0.10;

/// A single output the ladder asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rendition {
    /// Short edge of this rendition, and its identity in `video_renditions`.
    pub short_edge: i64,
    pub width: i64,
    pub height: i64,
    /// Whether this is the untouched source rather than a downscale.
    ///
    /// The source rung is only playable if the source itself is — see
    /// [`crate::transcode::probe::is_browser_native`]. A ladder whose top rung
    /// is an HEVC or corrupt source hands the user a picker whose "highest"
    /// option does not play (#46).
    pub is_source: bool,
}

/// Short edge of a frame, orientation-independent.
pub fn short_edge(width: i64, height: i64) -> i64 {
    width.min(height)
}

/// Largest short edge that still counts as *being* the tier.
///
/// Exposed rather than inlined into [`needs_rung`] because the candidate query
/// in [`super::rung_queue`] has to apply the same cut in SQL, where none of this
/// module's arithmetic is available. Two hand-written copies of
/// `tier * 1.1` is precisely the drift `gallery::eligibility` documents the cost
/// of — there, a predicate copy-pasted into three queries. Here the two copies
/// would disagree about which files are worth a 4K re-encode.
pub fn rung_threshold(tier_short_edge: i64) -> i64 {
    (tier_short_edge as f64 * (1.0 + TIER_TOLERANCE)).floor() as i64
}

/// Whether a source needs a separate rendition at the given tier.
///
/// `false` when the source is at or below the tier, and also when it is only
/// *just* above it — see [`TIER_TOLERANCE`].
pub fn needs_rung(width: i64, height: i64, tier_short_edge: i64) -> bool {
    // A probe that could not read dimensions reports 0. Planning a ladder from
    // unknown geometry would downscale to garbage, so plan nothing.
    //
    // NOTE this is *not* the same verdict the candidate query reaches. 58 live
    // videos have no recorded geometry, and `rung_queue` deliberately selects
    // them so they get probed. "Unknown" means "ask ffprobe", not "no rung" —
    // it is only here, where the geometry is final, that it means the latter.
    if width <= 0 || height <= 0 {
        return false;
    }
    short_edge(width, height) > rung_threshold(tier_short_edge)
}

/// Scale a frame so its short edge hits `target_short_edge`, preserving
/// orientation and aspect ratio, rounded to even dimensions.
///
/// Even dimensions are not cosmetic: NVENC rejects odd width/height outright,
/// and `yuv420p` chroma subsampling requires them.
///
/// Orientation is preserved by scaling the *short* edge. Scaling height
/// unconditionally is what turns `1080x1920` into `608x1080`.
pub fn rung_dimensions(width: i64, height: i64, target_short_edge: i64) -> (i64, i64) {
    let source_short = short_edge(width, height);
    if source_short <= 0 {
        return (width, height);
    }
    let scale = target_short_edge as f64 / source_short as f64;

    let scaled = |edge: i64| -> i64 {
        let v = (edge as f64 * scale).round() as i64;
        // Round to even, never to zero.
        let even = v - (v % 2);
        even.max(2)
    };

    if width <= height {
        // Portrait or square: width is the short edge and lands exactly.
        (target_short_edge, scaled(height))
    } else {
        (scaled(width), target_short_edge)
    }
}

/// Plan the full ladder for a source of the given dimensions.
///
/// The source is always rung 0 — a picker must be able to offer the original
/// quality. A 1080p rung is appended only when [`needs_rung`] agrees.
///
/// Returned highest-quality-first, which is the order a picker displays and the
/// order a "default to highest" client reads.
pub fn plan_renditions(width: i64, height: i64) -> Vec<Rendition> {
    let mut out = vec![Rendition {
        short_edge: short_edge(width, height),
        width,
        height,
        is_source: true,
    }];

    if needs_rung(width, height, TIER_1080_SHORT_EDGE) {
        let (w, h) = rung_dimensions(width, height, TIER_1080_SHORT_EDGE);
        out.push(Rendition {
            short_edge: TIER_1080_SHORT_EDGE,
            width: w,
            height: h,
            is_source: false,
        });
    }

    out
}

/// Whether the file that would back the source rung is actually playable.
///
/// #46 established that a `.mp4` may be HEVC, 10-bit, or a corrupt bitstream
/// behind an intact container. A ladder whose top rung is one of those hands
/// the user a picker whose "highest" option does not play — strictly worse than
/// having no picker, because it looks like a feature rather than a broken file.
///
/// This is deliberately evaluated against the file the server would *serve*
/// (`photos.file_path`), which after #46's ingest probe is already the
/// converted H.264 for anything non-native. So a `false` here means the
/// conversion failed or the row predates #46 — not that a re-encode is owed.
/// Producing one is #46's job, not the ladder's; the ladder simply declines to
/// offer a rung it cannot stand behind.
///
/// `health` is optional because decode-health probing costs a bounded decode
/// and is not run on every path. `None` means "not checked", which falls back
/// to the codec verdict alone — the pre-#46 level of confidence, and honest
/// about it. It is never treated as evidence of health.
pub fn source_rung_is_offerable(
    info: &super::probe::VideoStreamInfo,
    health: Option<&super::probe::DecodeHealth>,
) -> bool {
    if !super::probe::is_browser_native(info) {
        return false;
    }
    match health {
        Some(h) => h.is_clean(),
        None => true,
    }
}

/// Plan the ladder, dropping the source rung when the source cannot be played.
///
/// An empty result means "no picker" — the client falls back to the single
/// blob it already has, which is exactly today's behaviour. That is the correct
/// degenerate case for the 602 sub-1080p videos in the live library.
pub fn plan_ladder(width: i64, height: i64, source_offerable: bool) -> Vec<Rendition> {
    let mut plan = plan_renditions(width, height);
    if !source_offerable {
        plan.retain(|r| !r.is_source);
    }
    // A lone source rung is not a ladder — there is nothing to pick between,
    // and offering a one-entry picker is noise.
    if plan.len() == 1 && plan[0].is_source {
        return Vec::new();
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcode::probe::{DecodeHealth, VideoStreamInfo};

    fn native_1080() -> VideoStreamInfo {
        VideoStreamInfo {
            codec: "h264".into(),
            profile: Some("High".into()),
            pix_fmt: Some("yuv420p".into()),
            width: 3840,
            height: 2160,
        }
    }

    fn corrupt() -> DecodeHealth {
        DecodeHealth {
            error_count: 3331,
            first_error: Some("Invalid NAL unit size (-536345661 > 542).".into()),
        }
    }

    fn clean() -> DecodeHealth {
        DecodeHealth {
            error_count: 0,
            first_error: None,
        }
    }

    /// The #46 interaction. A 4K HEVC source still needs its 1080p rung — the
    /// user gains a playable option they did not have — but the source itself
    /// must not be offered, because it does not play.
    #[test]
    fn an_unplayable_source_is_not_offered_but_still_earns_a_rung() {
        let plan = plan_ladder(3840, 2160, false);
        assert_eq!(plan.len(), 1, "the 1080p rung must survive");
        assert!(
            !plan[0].is_source,
            "a picker must never offer a 'highest' that does not play"
        );
        assert_eq!((plan[0].width, plan[0].height), (1920, 1080));
    }

    /// A playable 4K source gives the two-entry picker the issue actually asks
    /// for.
    #[test]
    fn a_playable_oversized_source_yields_a_two_rung_picker() {
        let plan = plan_ladder(3840, 2160, true);
        assert_eq!(plan.len(), 2);
        assert!(plan[0].is_source);
        assert!(!plan[1].is_source);
    }

    /// 602 of 742 live videos land here. A single source rung is not a choice,
    /// so it must not render a picker at all.
    #[test]
    fn an_ordinary_source_produces_no_picker() {
        assert!(plan_ladder(1920, 1080, true).is_empty());
        assert!(plan_ladder(1280, 720, true).is_empty());
        // ...and the portrait/padded traps must not sneak a picker in either.
        assert!(plan_ladder(1080, 1920, true).is_empty());
        assert!(plan_ladder(2288, 1088, true).is_empty());
    }

    /// An unplayable sub-1080p source has nothing to offer — #46's conversion
    /// is what fixes it, not the ladder.
    #[test]
    fn an_unplayable_small_source_yields_nothing_for_the_ladder_to_do() {
        assert!(plan_ladder(1280, 720, false).is_empty());
    }

    /// The reported #46 file: impeccably native by codec, thousands of decode
    /// errors in fact. Codec identity alone must not clear it.
    #[test]
    fn a_corrupt_bitstream_is_not_offerable_despite_a_native_codec() {
        let info = native_1080();
        assert!(
            super::super::probe::is_browser_native(&info),
            "precondition: it IS codec-native, which is why codec alone is not enough"
        );
        assert!(!source_rung_is_offerable(&info, Some(&corrupt())));
        assert!(source_rung_is_offerable(&info, Some(&clean())));
    }

    /// Unchecked health is not evidence of health — but it must not veto an
    /// otherwise-native source either, or every un-probed file loses its rung.
    #[test]
    fn unchecked_health_falls_back_to_the_codec_verdict() {
        assert!(source_rung_is_offerable(&native_1080(), None));

        let hevc = VideoStreamInfo {
            codec: "hevc".into(),
            ..native_1080()
        };
        assert!(
            !source_rung_is_offerable(&hevc, None),
            "a non-native codec is disqualifying regardless of health"
        );
        // Clean decode does not rescue a codec browsers cannot decode.
        assert!(!source_rung_is_offerable(&hevc, Some(&clean())));
    }

    /// The exact shapes measured on the live library, with their counts.
    /// Keeping the census here is what lets the tests below assert a *number*
    /// rather than a belief about the rule.
    const LIVE_CENSUS: &[(i64, i64, usize)] = &[
        (3840, 2160, 126), // 4K landscape — dominates the demand
        (1920, 1440, 6),   // 4:3 — the smallest genuine case
        (7680, 4320, 4),   // 8K
        (2288, 1088, 4),   // macroblock-padded 1080 — must NOT get a rung
        (1080, 1920, 14),  // portrait 1080p — must NOT get a rung
    ];

    /// The headline number. If this test changes, the rule changed, and the
    /// live library's re-encode bill changed with it.
    #[test]
    fn live_library_census_yields_the_measured_136() {
        let strict: usize = LIVE_CENSUS
            .iter()
            .filter(|(w, h, _)| short_edge(*w, *h) > TIER_1080_SHORT_EDGE)
            .map(|(_, _, n)| n)
            .sum();
        assert_eq!(
            strict, 140,
            "a strict `short edge > 1080` test should flag 140 sources"
        );

        let with_tolerance: usize = LIVE_CENSUS
            .iter()
            .filter(|(w, h, _)| needs_rung(*w, *h, TIER_1080_SHORT_EDGE))
            .map(|(_, _, n)| n)
            .sum();
        assert_eq!(
            with_tolerance, 136,
            "the tolerance band must exclude exactly the 4 `2288x1088` files"
        );
    }

    /// Trap 1: portrait 1080p. Keying on height flags these, and the naive
    /// `scale=-2:1080` fix would downscale them to 608x1080.
    #[test]
    fn portrait_1080p_needs_no_rung() {
        assert!(!needs_rung(1080, 1920, TIER_1080_SHORT_EDGE));
        assert_eq!(
            plan_renditions(1080, 1920).len(),
            1,
            "1080x1920 IS the 1080p tier — it must get the source rung only"
        );
        // Guard the specific degradation, so a regression names itself.
        assert_ne!(
            rung_dimensions(1080, 1920, TIER_1080_SHORT_EDGE),
            (608, 1080),
            "scaling the height of a portrait source is the documented bug"
        );
        assert_eq!(
            rung_dimensions(1080, 1920, TIER_1080_SHORT_EDGE),
            (1080, 1920)
        );
    }

    /// Trap 2: macroblock padding. 1088 is 1080 rounded up to a multiple of 16.
    #[test]
    fn macroblock_padded_1080_is_inside_the_tolerance_band() {
        assert!(
            short_edge(2288, 1088) > TIER_1080_SHORT_EDGE,
            "precondition: it IS strictly taller, which is why `>` is not enough"
        );
        assert!(
            !needs_rung(2288, 1088, TIER_1080_SHORT_EDGE),
            "re-encoding a 4K-class file to reclaim 8 pixels is not a trade worth making"
        );
    }

    /// The genuine cases, each with the dimensions the transcoder will be asked
    /// for. Orientation must survive in every one.
    #[test]
    fn oversized_sources_get_a_correctly_shaped_1080_rung() {
        for (w, h, want) in [
            ((3840, 2160), (1920, 1080)),
            ((7680, 4320), (1920, 1080)),
            ((1920, 1440), (1440, 1080)),
        ]
        .map(|((w, h), want)| (w, h, want))
        {
            let plan = plan_renditions(w, h);
            assert_eq!(plan.len(), 2, "{w}x{h} must produce source + 1080");
            assert!(plan[0].is_source, "the source must be offered first");
            assert_eq!(plan[0].width, w);
            assert!(!plan[1].is_source);
            assert_eq!(
                (plan[1].width, plan[1].height),
                want,
                "{w}x{h} should downscale to {want:?}"
            );
            assert_eq!(plan[1].short_edge, TIER_1080_SHORT_EDGE);
        }
    }

    /// Portrait 4K is the mirror of the landscape case: the short edge is the
    /// width, so the width is what must land on 1080.
    #[test]
    fn portrait_sources_scale_their_width() {
        assert!(needs_rung(2160, 3840, TIER_1080_SHORT_EDGE));
        assert_eq!(
            rung_dimensions(2160, 3840, TIER_1080_SHORT_EDGE),
            (1080, 1920)
        );
        // ...and the aspect ratio is unchanged, which is the actual invariant.
        let (w, h) = rung_dimensions(2160, 3840, TIER_1080_SHORT_EDGE);
        assert!(
            ((w as f64 / h as f64) - (2160.0 / 3840.0)).abs() < 0.01,
            "aspect ratio must survive the downscale"
        );
    }

    /// NVENC rejects odd dimensions outright, so this is a hard requirement
    /// rather than tidiness. Awkward aspect ratios are where it bites.
    #[test]
    fn rung_dimensions_are_always_even() {
        for (w, h) in [
            (3840, 2160),
            (1920, 1443),
            (2049, 1300),
            (3841, 2161),
            (1234, 5678),
        ] {
            let (rw, rh) = rung_dimensions(w, h, TIER_1080_SHORT_EDGE);
            assert_eq!(rw % 2, 0, "{w}x{h} produced odd width {rw}");
            assert_eq!(rh % 2, 0, "{w}x{h} produced odd height {rh}");
            assert!(rw > 0 && rh > 0);
        }
    }

    /// A probe that cannot read geometry reports 0. Planning from that would
    /// hand ffmpeg a zero or negative scale factor.
    #[test]
    fn unknown_geometry_plans_no_rung_and_never_divides_by_zero() {
        for (w, h) in [(0, 0), (0, 1080), (3840, 0), (-1, -1)] {
            assert!(!needs_rung(w, h, TIER_1080_SHORT_EDGE), "{w}x{h}");
            assert_eq!(
                plan_renditions(w, h).len(),
                1,
                "{w}x{h} must degenerate to source-only, not panic"
            );
            // Must not panic, and must not invent dimensions.
            let _ = rung_dimensions(w, h, TIER_1080_SHORT_EDGE);
        }
    }

    /// Sub-1080 sources are the overwhelming majority (602 of 742). They must
    /// never be upscaled — that costs an encode and destroys quality.
    #[test]
    fn undersized_sources_are_never_upscaled() {
        for (w, h) in [(640, 480), (320, 240), (1280, 720), (1920, 1080)] {
            let plan = plan_renditions(w, h);
            assert_eq!(plan.len(), 1, "{w}x{h} must not gain a rung");
            assert!(plan[0].is_source);
            assert_eq!((plan[0].width, plan[0].height), (w, h));
        }
    }

    /// The tier boundary itself, from both sides.
    #[test]
    fn the_tolerance_boundary_is_where_it_is_documented_to_be() {
        // 1080 * 1.10 = 1188, floored. Strictly greater than that gets a rung.
        assert!(!needs_rung(1920, 1188, TIER_1080_SHORT_EDGE));
        assert!(needs_rung(1920, 1189, TIER_1080_SHORT_EDGE));
    }
}
