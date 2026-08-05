//! ffprobe-backed media stream inspection.
//!
//! Extension-based format detection is a guess. A `.mp4` is a *container*, and
//! it routinely carries video a browser cannot decode (HEVC, 10-bit, MPEG-4
//! Part 2) — or an intact container header wrapped around a corrupt bitstream.
//! Both report as "an mp4" and both fail in the player with the same useless
//! "unable to play this video format".
//!
//! This module answers the two questions that actually matter, by probing:
//!
//! 1. [`is_browser_native`] — can a browser decode these streams at all?
//! 2. [`probe_decode_health`] — does the bitstream actually decode, or is the
//!    container lying about its contents?
//!
//! Measured against the live 742-video library (2026-07-20): 704 h264,
//! 28 hevc, 10 mpeg4 — so 38 files (5.1%) are silently unplayable today
//! because `.mp4` never enters the conversion queue. Separately, three files
//! probe as clean `h264/Main/yuv420p` yet emit thousands of
//! `Invalid NAL unit size` errors on decode. A codec allowlist alone would
//! pass every one of those three.
//!
//! **Output format note.** Everything here parses ffprobe's *JSON* output, not
//! CSV, and that is deliberate. `-show_entries` emits fields in ffprobe's
//! internal struct order rather than the order requested, and its CSV writer
//! appends a trailing empty field. Both are silent: CSV parsing by requested
//! position mislabels every row, and a field-count guard drops a
//! non-random subset. JSON is keyed, so neither failure mode exists.

use std::path::Path;

use serde::Deserialize;

use crate::process::{run_with_timeout, FFPROBE_TIMEOUT};

/// Video codecs + profiles + pixel formats every target browser can decode.
///
/// Deliberately narrow: anything not proven native gets transcoded. A
/// needless transcode costs CPU once; a wrongly-trusted "native" verdict
/// leaves the user with a video that never plays.
const NATIVE_H264_PROFILES: &[&str] = &["baseline", "constrained baseline", "main", "high"];

/// 8-bit 4:2:0 chroma — the only pixel formats browser H.264 decoders accept.
/// `yuvj420p` is the same layout with full-range levels.
const NATIVE_PIX_FMTS: &[&str] = &["yuv420p", "yuvj420p"];

/// Cap on how much content the decode health check will read.
///
/// Decoding runs far faster than realtime (~327× measured on CT132), so a
/// bounded check is cheap: the whole 742-video library costs well under a
/// minute, once, at ingest. Unbounded decoding of a 10-minute 4K source to
/// prove it is fine is not a trade worth making.
const DECODE_HEALTH_SECONDS: u32 = 60;

/// A file whose *container* parses but whose streams cannot be inspected is
/// already broken; treat it as such rather than as "no video stream".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// ffprobe could not be run at all (missing binary, timeout, spawn error).
    Unavailable(String),
    /// ffprobe ran but returned no parseable video stream.
    NoVideoStream,
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Unavailable(e) => write!(f, "ffprobe unavailable: {e}"),
            ProbeError::NoVideoStream => write!(f, "no decodable video stream"),
        }
    }
}

/// The properties of a file's primary video stream that decide playability.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VideoStreamInfo {
    pub codec: String,
    pub profile: Option<String>,
    pub pix_fmt: Option<String>,
    pub width: i64,
    pub height: i64,
}

/// Result of attempting to actually decode a file's bitstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeHealth {
    /// Lines ffmpeg emitted at `error` level while decoding.
    pub error_count: usize,
    /// A representative error line, for logging and for the audit trail.
    pub first_error: Option<String>,
}

impl DecodeHealth {
    /// Whether the bitstream decoded without complaint.
    ///
    /// Any decode error means a browser — which is far stricter than ffmpeg —
    /// will almost certainly refuse the file outright. ffmpeg's leniency is
    /// precisely why re-encoding rescues these: it salvages the frames it can
    /// read and emits a clean bitstream. Measured on the reported file:
    /// 3,331 errors in, 0 errors out.
    pub fn is_clean(&self) -> bool {
        self.error_count == 0
    }
}

// ── ffprobe JSON shapes ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    codec_name: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    pix_fmt: Option<String>,
    #[serde(default)]
    width: Option<i64>,
    #[serde(default)]
    height: Option<i64>,
}

// ── Pure decision logic (unit-tested without ffmpeg) ────────────────────────

/// Whether a filename's extension is a container that tells us nothing about
/// its contents, and therefore must be probed rather than trusted.
///
/// The rule is **two** conditions, not one, and collapsing them to the first is
/// what this set previously got wrong:
///
/// 1. [`crate::conversion::conversion_target`] must skip it — otherwise the
///    extension already decided and the probe never runs.
/// 2. [`is_browser_native`] must be able to *adjudicate* it — otherwise the
///    probe can only ever return the wrong answer.
///
/// `conversion_target` skips exactly `.mp4` and `.webm`, so condition 1 alone
/// would admit both. Condition 2 removes `.webm`: `is_browser_native` is an
/// **H.264-only allowlist**, so a VP9 / AV1 WebM — which `crate::media`'s own
/// `MEDIA_EXTENSIONS` already lists as universally playable in `<video>` — comes
/// back "not native" and is re-encoded for nothing. That is the same false
/// positive `photos::web_preview::preview_needs_probe` excludes `.webm` for;
/// this is the ingest side of it. Widening the allowlist to VP8/VP9/AV1 is the
/// other way to satisfy condition 2, and remains a separate change.
///
/// `.mov` / `.m4v` are absent because they fail condition 1: `conversion_target`
/// returns `Some(mp4)` for both, so the caller's probe branch was **unreachable**
/// for them. Moving them here by dropping them from `conversion_target` was
/// considered and rejected — the ingest path has no remux verdict
/// ([`crate::ingest::OpaqueVerdict`] is convert / leave / unplayable), so a
/// native `.mov` would be *left* as `video/quicktime`, which Chrome and Firefox
/// refuse to play. That is worse than today's wasteful-but-correct re-encode.
pub fn is_opaque_video_container(filename: &str) -> bool {
    let ext = match filename.rsplit('.').next() {
        Some(e) => e.to_ascii_lowercase(),
        None => return false,
    };
    matches!(ext.as_str(), "mp4")
}

/// Whether a browser can decode this video stream natively.
///
/// Pure — this is the whole point of separating it from the probe. The rule is
/// an allowlist, not a denylist: unknown codecs are *not* native.
pub fn is_browser_native(info: &VideoStreamInfo) -> bool {
    if !info.codec.eq_ignore_ascii_case("h264") {
        return false;
    }

    // A missing pix_fmt is not evidence of 8-bit 4:2:0, so it is not native.
    let pix_ok = info
        .pix_fmt
        .as_deref()
        .map(|p| NATIVE_PIX_FMTS.contains(&p.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    if !pix_ok {
        return false;
    }

    // "High 10" / "High 4:2:2" / "High 4:4:4 Predictive" all start with "High"
    // but are emphatically not native, so match the full profile string.
    info.profile
        .as_deref()
        .map(|p| NATIVE_H264_PROFILES.contains(&p.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Parse ffprobe JSON into the primary video stream's properties.
///
/// Split out from the process spawn so the parsing — including the awkward
/// real-world shapes — is testable against fixtures.
fn parse_video_stream(json: &str) -> Result<VideoStreamInfo, ProbeError> {
    let parsed: FfprobeOutput =
        serde_json::from_str(json).map_err(|e| ProbeError::Unavailable(e.to_string()))?;

    parsed
        .streams
        .into_iter()
        .find(|s| s.codec_type == "video")
        .map(|s| VideoStreamInfo {
            codec: s.codec_name,
            profile: s.profile,
            pix_fmt: s.pix_fmt,
            width: s.width.unwrap_or(0),
            height: s.height.unwrap_or(0),
        })
        .ok_or(ProbeError::NoVideoStream)
}

/// Count decode-level errors in ffmpeg's stderr.
///
/// Separated from the spawn for the same reason as [`parse_video_stream`].
fn parse_decode_errors(stderr: &str) -> DecodeHealth {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    DecodeHealth {
        error_count: lines.len(),
        first_error: lines.first().map(|s| s.to_string()),
    }
}

// ── Probing (spawns ffprobe / ffmpeg) ───────────────────────────────────────

/// Inspect a file's primary video stream.
pub async fn probe_video_stream(path: &Path) -> Result<VideoStreamInfo, ProbeError> {
    let mut cmd = tokio::process::Command::new("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=codec_name,codec_type,profile,pix_fmt,width,height",
        "-of",
        "json",
    ])
    .arg(path)
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null());

    let output = run_with_timeout(&mut cmd, FFPROBE_TIMEOUT)
        .await
        .map_err(ProbeError::Unavailable)?;

    parse_video_stream(&String::from_utf8_lossy(&output.stdout))
}

/// Attempt to decode a bounded prefix of the file and report how many errors
/// the decoder emitted.
///
/// Decodes to the null muxer, so nothing is written; the only output that
/// matters is stderr.
pub async fn probe_decode_health(path: &Path) -> Result<DecodeHealth, ProbeError> {
    let mut cmd = crate::process::background_command("ffmpeg");
    cmd.args(["-v", "error", "-t", &DECODE_HEALTH_SECONDS.to_string()])
        .arg("-i")
        .arg(path)
        .args(["-f", "null", "-"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let output = run_with_timeout(&mut cmd, crate::process::FFMPEG_RENDER_TIMEOUT)
        .await
        .map_err(ProbeError::Unavailable)?;

    Ok(parse_decode_errors(&String::from_utf8_lossy(
        &output.stderr,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(codec: &str, profile: &str, pix_fmt: &str) -> VideoStreamInfo {
        VideoStreamInfo {
            codec: codec.into(),
            profile: Some(profile.into()),
            pix_fmt: Some(pix_fmt.into()),
            width: 1920,
            height: 1080,
        }
    }

    /// The 704 h264 files measured on the live library must not be dragged
    /// into the conversion queue — a false positive here re-encodes almost the
    /// whole library for nothing.
    #[test]
    fn native_h264_profiles_are_left_alone() {
        for profile in ["Baseline", "Constrained Baseline", "Main", "High"] {
            assert!(
                is_browser_native(&info("h264", profile, "yuv420p")),
                "h264/{profile}/yuv420p must be treated as browser-native"
            );
        }
        // Full-range 4:2:0 is the same layout; 313 files in the live library
        // use it and they play fine today.
        assert!(is_browser_native(&info("h264", "High", "yuvj420p")));
    }

    /// The 38 genuinely-unplayable files measured on the live library. Each of
    /// these is a `.mp4`/`.mov` today and is therefore skipped entirely.
    #[test]
    fn non_native_codecs_are_flagged() {
        // 28 hevc files.
        assert!(!is_browser_native(&info("hevc", "Main", "yuv420p")));
        assert!(!is_browser_native(&info("hevc", "Main", "yuvj420p")));
        // 10 mpeg4 Part 2 (DivX/Xvid era).
        assert!(!is_browser_native(&info(
            "mpeg4",
            "Simple Profile",
            "yuv420p"
        )));
    }

    /// `High 10` shares a prefix with the native `High`. Matching on prefix
    /// rather than the full string would pass 10-bit video as native — there
    /// is exactly one such file in the live library, which is precisely the
    /// kind of single case that never gets noticed.
    #[test]
    fn ten_bit_h264_is_not_native_despite_the_high_prefix() {
        assert!(
            !is_browser_native(&info("h264", "High 10", "yuv420p10le")),
            "High 10 / 10-bit must not be mistaken for the native High profile"
        );
        // Even if the pixel format were somehow reported as 8-bit, the profile
        // alone must disqualify it.
        assert!(!is_browser_native(&info("h264", "High 10", "yuv420p")));
        assert!(!is_browser_native(&info("h264", "High 4:2:2", "yuv420p")));
    }

    /// Absent metadata is not evidence of playability.
    #[test]
    fn missing_metadata_is_not_native() {
        assert!(!is_browser_native(&VideoStreamInfo {
            codec: "h264".into(),
            profile: None,
            pix_fmt: Some("yuv420p".into()),
            width: 1920,
            height: 1080,
        }));
        assert!(!is_browser_native(&VideoStreamInfo {
            codec: "h264".into(),
            profile: Some("High".into()),
            pix_fmt: None,
            width: 1920,
            height: 1080,
        }));
        assert!(!is_browser_native(&VideoStreamInfo::default()));
    }

    /// Condition 1: an extension `conversion_target` already claims can never
    /// reach the probe, so listing it here is a lie about why the file converts.
    ///
    /// Asserted against `conversion_target` itself rather than against a copied
    /// list, because a copied list is exactly how this drifted: the old version
    /// of this test hard-coded `.mov`/`.m4v` as "must be probed" while
    /// `conversion_target` returned `Some(mp4)` for both, making the caller's
    /// probe branch unreachable and this assertion decorative.
    #[test]
    fn nothing_conversion_target_already_claims_is_routed_through_the_probe() {
        for name in ["a.mov", "a.m4v", "a.mkv", "a.avi", "a.wmv", "a.3gp"] {
            assert!(
                crate::conversion::conversion_target(name).is_some(),
                "precondition: {name} converts on its extension alone"
            );
            assert!(
                !is_opaque_video_container(name),
                "{name} is claimed by conversion_target — its probe branch is unreachable, \
                 so listing it here documents a path that does not exist"
            );
        }
    }

    /// Condition 2, and the live defect this set had: `.webm` *is* skipped by
    /// `conversion_target`, so it reached the probe — where an H.264-only
    /// allowlist can only ever call VP9/AV1 "not native" and queue a pointless
    /// full re-encode of a file every target browser plays.
    ///
    /// `media::MEDIA_EXTENSIONS` already calls `.webm` universally playable, and
    /// an *already-registered* `.webm` is served untouched, so the pre-fix tree
    /// treated the same file two different ways depending only on when it
    /// arrived. This assertion fails on that tree.
    #[test]
    fn webm_is_not_probed_because_the_allowlist_cannot_judge_it() {
        assert!(
            crate::conversion::conversion_target("a.webm").is_none(),
            "precondition: .webm is skipped by conversion_target, so it does reach the probe"
        );
        assert!(
            crate::media::MEDIA_EXTENSIONS.contains(&"webm"),
            "precondition: the rest of the server already treats .webm as browser-native"
        );
        for name in ["a.webm", "a.WEBM"] {
            assert!(
                !is_opaque_video_container(name),
                "{name} must not be probed — is_browser_native is H.264-only and would \
                 re-encode a VP9/AV1 stream that plays fine"
            );
        }
    }

    /// The one container that satisfies both conditions.
    #[test]
    fn mp4_is_the_only_opaque_container() {
        for name in ["a.mp4", "a.MP4"] {
            assert!(is_opaque_video_container(name), "{name} must be probed");
        }
        for name in ["a.jpg", "a.mp3", "noextension", ""] {
            assert!(
                !is_opaque_video_container(name),
                "{name} must not be routed through the probe"
            );
        }
    }

    /// Real ffprobe JSON from the reported file, `20210520212438-5a45c3d4.mp4`.
    /// Note it parses as impeccably native — which is the entire reason the
    /// codec allowlist cannot be the whole fix.
    #[test]
    fn parses_the_reported_file_as_native() {
        let json = r#"{
            "streams": [
                { "index": 1, "codec_name": "h264", "codec_type": "video",
                  "profile": "Main", "pix_fmt": "yuv420p",
                  "width": 320, "height": 240 }
            ]
        }"#;
        let parsed = parse_video_stream(json).expect("should parse");
        assert_eq!(parsed.codec, "h264");
        assert_eq!(parsed.width, 320);
        assert_eq!(parsed.height, 240);
        assert!(
            is_browser_native(&parsed),
            "the reported file IS codec-native — corruption, not codec, is its defect"
        );
    }

    /// The audio stream is listed first in the reported file, and the file also
    /// carries two `mp4s` data tracks. Picking the first stream, or assuming
    /// video is at index 0, gets the wrong answer.
    #[test]
    fn selects_the_video_stream_not_the_first_stream() {
        let json = r#"{
            "streams": [
                { "index": 0, "codec_name": "aac", "codec_type": "audio", "profile": "LC" },
                { "index": 1, "codec_name": "hevc", "codec_type": "video",
                  "profile": "Main", "pix_fmt": "yuv420p", "width": 3840, "height": 2160 },
                { "index": 2, "codec_name": "unknown", "codec_type": "data" },
                { "index": 3, "codec_name": "unknown", "codec_type": "data" }
            ]
        }"#;
        let parsed = parse_video_stream(json).expect("should parse");
        assert_eq!(parsed.codec, "hevc", "must skip audio and data tracks");
        assert_eq!(parsed.height, 2160);
        assert!(!is_browser_native(&parsed));
    }

    /// `VIDEO0063.mp4` in the live library returns no usable stream at all.
    /// It must surface as an error, never as a default-constructed "fine".
    #[test]
    fn a_file_with_no_video_stream_is_an_error() {
        let audio_only = r#"{"streams":[{"codec_name":"aac","codec_type":"audio"}]}"#;
        assert_eq!(
            parse_video_stream(audio_only),
            Err(ProbeError::NoVideoStream)
        );
        assert_eq!(
            parse_video_stream(r#"{"streams":[]}"#),
            Err(ProbeError::NoVideoStream)
        );
    }

    /// Garbage in must not panic the scan task.
    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(matches!(
            parse_video_stream("not json at all"),
            Err(ProbeError::Unavailable(_))
        ));
        assert!(matches!(
            parse_video_stream(""),
            Err(ProbeError::Unavailable(_))
        ));
    }

    /// A clean decode is the signal that no conversion is needed.
    #[test]
    fn silent_stderr_means_a_healthy_bitstream() {
        let health = parse_decode_errors("");
        assert!(health.is_clean());
        assert_eq!(health.error_count, 0);
        assert_eq!(health.first_error, None);
        // Whitespace-only output is still clean.
        assert!(parse_decode_errors("\n  \n").is_clean());
    }

    /// Real ffmpeg stderr from the reported file. The first line is retained
    /// so the audit trail can say *why* a file was re-encoded.
    #[test]
    fn decode_errors_are_counted_and_the_first_is_retained() {
        let stderr = "[h264 @ 0x60d0] Invalid NAL unit size (-536345661 > 542).\n\
                      [h264 @ 0x60d0] missing picture in access unit with size 546\n\
                      [h264 @ 0x60d0] Error splitting the input into NAL units.\n";
        let health = parse_decode_errors(stderr);
        assert!(
            !health.is_clean(),
            "a corrupt bitstream must not be reported as healthy"
        );
        assert_eq!(health.error_count, 3);
        assert!(health
            .first_error
            .as_deref()
            .unwrap()
            .contains("Invalid NAL unit size"));
    }
}
