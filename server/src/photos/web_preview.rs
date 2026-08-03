//! Web preview generation for non-browser-native media formats.
//!
//! Some formats (HEIC, MKV, WMA, etc.) can't be displayed natively in a
//! browser. This module converts them into web-friendly formats via FFmpeg.
//! (FFmpeg only — no ImageMagick — to keep the install to a single media tool.
//! HEIC/HEIF decodes natively via FFmpeg's mov demuxer + built-in HEVC decoder,
//! so no libheif build is required.)
//!
//! **This is not an on-the-fly preview path.** Its one consumer is
//! [`crate::photos::server_migrate_encrypt`], which encrypts the chosen payload
//! *at rest* — so whatever this module picks is the byte stream every client is
//! handed for that photo, for as long as the photo exists. Getting the choice
//! wrong is therefore not a cosmetic miss:
//!
//! * **Miss a non-native codec** and the stored payload is unplayable in a
//!   browser. The video ladder (#49) papers over this in the main gallery by
//!   offering a rendition, but a secure gallery item carries no renditions
//!   (`gallery::secure::list_gallery_items` never joins them), so there the
//!   payload is all there is.
//! * **Convert something that was already fine** and the stored copy is a
//!   needless re-encode of the original — generation loss, permanently.
//!
//! Extensions cannot answer either question for video: an `.mp4` container
//! happily carries HEVC or 10-bit H.264, and a `.mov` usually carries perfectly
//! ordinary H.264. So video containers are **probed**, exactly as
//! `ingest::opaque_container_needs_conversion` probes them at scan time.

use std::path::Path;

/// How to produce the browser-viewable payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// Full re-encode — the source is genuinely not something a browser decodes.
    Transcode,
    /// Container rewrite only. The video stream is already browser-native and is
    /// stream-copied, so there is no generation loss and no meaningful CPU cost.
    Remux,
}

/// The preview a file needs: the target extension and how to get there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebPreview {
    pub ext: &'static str,
    pub mode: PreviewMode,
}

/// The extension-only verdict.
///
/// Still the right answer for images and audio, where the extension *is* the
/// format: a `.heic` is HEIC and a `.wma` is WMA. It is only video containers
/// that lie, and those are corrected by [`resolve_web_preview`].
pub fn needs_web_preview(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        // Images that browsers cannot display natively. (RAW formats — cr2,
        // dng, nef, arw, raw — are intentionally omitted: FFmpeg can't decode
        // them and we no longer ship ImageMagick, so they are unsupported.)
        "heic" | "heif" | "tiff" | "tif" | "hdr" | "cur" | "cursor" => Some("jpg"),
        "ico" => Some("png"),
        // Audio that browsers cannot play natively
        "wma" | "aiff" | "aif" => Some("mp3"),
        // Video containers that browsers cannot play natively
        "mkv" | "avi" | "wmv" | "asf" | "h264" | "mpg" | "mpeg" | "3gp" | "mov" | "m4v" => {
            Some("mp4")
        }
        _ => None,
    }
}

/// Whether this filename's extension is too vague to decide on its own.
///
/// Deliberately **not** the same set as
/// [`crate::transcode::probe::is_opaque_video_container`], which also includes
/// `.webm`. That set feeds an H.264-only allowlist ([`probe::is_browser_native`]
/// (crate::transcode::probe::is_browser_native)), so a VP9 WebM — which every
/// target browser plays — would be reported "not native" and re-encoded into
/// the stored payload for nothing. `.webm` has never been previewed here and is
/// left alone; correcting the allowlist to cover VP8/VP9/AV1 is a separate
/// change with its own blast radius at ingest.
fn preview_needs_probe(filename: &str) -> bool {
    let ext = match filename.rsplit('.').next() {
        Some(e) => e.to_ascii_lowercase(),
        None => return false,
    };
    matches!(ext.as_str(), "mp4" | "mov" | "m4v")
}

/// Fold what the probe found inside the file into the extension's verdict.
///
/// Pure, so the whole decision matrix is unit-testable without FFmpeg — the
/// probe spawn lives in [`plan_web_preview`].
///
/// `stream_is_native` is `None` when the file could not be probed at all (no
/// video stream, ffprobe missing, timeout). An environmental failure is not
/// evidence either way, so the extension's answer stands and the behaviour is
/// byte-for-byte what it was before this function existed.
pub fn resolve_web_preview(filename: &str, stream_is_native: Option<bool>) -> Option<WebPreview> {
    let ext_verdict = needs_web_preview(filename);

    if !preview_needs_probe(filename) {
        return ext_verdict.map(|ext| WebPreview {
            ext,
            mode: PreviewMode::Transcode,
        });
    }

    match stream_is_native {
        // A non-native codec wearing a native-looking extension. The extension
        // said "no preview needed" and was wrong; without this arm the stored
        // payload is an HEVC / MPEG-4 Part 2 / 10-bit H.264 stream that no
        // browser decodes.
        Some(false) => Some(WebPreview {
            ext: "mp4",
            mode: PreviewMode::Transcode,
        }),
        // Browser-native stream. `.mp4` needs nothing at all (`ext_verdict` is
        // already `None`); `.mov`/`.m4v` need their wrapper rewritten, not their
        // pixels re-encoded.
        Some(true) => ext_verdict.map(|ext| WebPreview {
            ext,
            mode: PreviewMode::Remux,
        }),
        None => ext_verdict.map(|ext| WebPreview {
            ext,
            mode: PreviewMode::Transcode,
        }),
    }
}

/// Decide what preview a file needs, probing video containers rather than
/// trusting their extension.
///
/// Costs one `ffprobe` for `.mp4`/`.mov`/`.m4v` only; images, audio and the
/// unambiguous video containers answer from the filename and spawn nothing.
pub async fn plan_web_preview(path: &Path, filename: &str) -> Option<WebPreview> {
    let stream_is_native = if preview_needs_probe(filename) {
        match crate::transcode::probe::probe_video_stream(path).await {
            Ok(info) => Some(crate::transcode::probe::is_browser_native(&info)),
            Err(e) => {
                // Not an error path worth failing the encryption over: falling
                // back to the extension verdict is exactly the old behaviour.
                tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    "Web preview: could not probe video container — \
                     falling back to the extension's verdict"
                );
                None
            }
        }
    } else {
        None
    };

    resolve_web_preview(filename, stream_is_native)
}

/// Public wrapper for background web preview generation.
pub async fn generate_web_preview_bg(
    input_path: &Path,
    output_path: &Path,
    plan: WebPreview,
) -> bool {
    generate_web_preview(input_path, output_path, plan).await
}

/// Ceiling for a single preview conversion.  Without it a hung FFmpeg
/// (corrupt input, stuck GPU session) wedged the preview task forever.
const PREVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Rewrite a browser-native video stream into an MP4 container without touching
/// the pixels.
///
/// Audio is re-encoded to AAC rather than copied: a QuickTime container can
/// legally carry PCM or ALAC, an MP4 wrapper will happily accept either, and no
/// browser will play the result — a silent failure that a stream-copy would
/// produce and nothing downstream would catch. Re-encoding an audio track costs
/// a rounding error next to the video encode this whole path exists to skip.
async fn remux_to_mp4(input: &str, output: &str) -> bool {
    let mut cmd = crate::process::background_command("ffmpeg");
    cmd.args([
        "-y",
        "-i",
        input,
        "-c:v",
        "copy",
        "-c:a",
        "aac",
        "-movflags",
        "+faststart",
        output,
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    let status = crate::process::status_with_timeout(&mut cmd, PREVIEW_TIMEOUT).await;
    matches!(status, Ok(s) if s.success())
}

/// Generate a browser-compatible web preview file.
/// Images → high-quality JPEG, ICO → PNG, Audio → MP3, Video → MP4 (H.264/AAC).
async fn generate_web_preview(input_path: &Path, output_path: &Path, plan: WebPreview) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let input_str = input_path.to_str().unwrap_or("");
    let output_str = output_path.to_str().unwrap_or("");

    let ffmpeg_ok = match plan.ext {
        "jpg" => {
            let mut cmd = crate::process::background_command("ffmpeg");
            cmd.args(["-y", "-i", input_str, "-q:v", "2", output_str])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let status = crate::process::status_with_timeout(&mut cmd, PREVIEW_TIMEOUT).await;
            matches!(status, Ok(s) if s.success())
        }
        "png" => {
            let mut cmd = crate::process::background_command("ffmpeg");
            cmd.args(["-y", "-i", input_str, output_str])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let status = crate::process::status_with_timeout(&mut cmd, PREVIEW_TIMEOUT).await;
            matches!(status, Ok(s) if s.success())
        }
        "mp3" => {
            let mut cmd = crate::process::background_command("ffmpeg");
            cmd.args([
                "-y",
                "-i",
                input_str,
                "-codec:a",
                "libmp3lame",
                "-b:a",
                "192k",
                output_str,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
            let status = crate::process::status_with_timeout(&mut cmd, PREVIEW_TIMEOUT).await;
            matches!(status, Ok(s) if s.success())
        }
        "mp4" if plan.mode == PreviewMode::Remux => {
            // A stream copy can still be refused (exotic stream, unsupported
            // bitstream filter), and "the container is wrong" is not a reason to
            // leave the payload unplayable — fall through to the full encode.
            if remux_to_mp4(input_str, output_str).await {
                true
            } else {
                tracing::warn!(
                    input = %input_path.display(),
                    "Web preview: remux refused the stream — falling back to a full transcode"
                );
                let hwaccel = crate::conversion::active_hwaccel();
                let fallback = crate::conversion::cpu_fallback_enabled();
                crate::conversion::convert_video(
                    input_str, output_str, hwaccel, fallback, None, None,
                )
                .await
            }
        }
        "mp4" => {
            // Route through the shared GPU-aware video transcoder so
            // on-the-fly previews honour the NVENC / QSV / VAAPI path
            // configured at startup. Falls back to libx264 internally
            // when no hwaccel is registered or the GPU encode fails.
            let hwaccel = crate::conversion::active_hwaccel();
            let fallback = crate::conversion::cpu_fallback_enabled();
            // A single on-the-fly preview is a lone transcode — let ffmpeg
            // auto-detect threads (all cores). The per-encode thread cap is only
            // for the bulk ingest pass that runs many encodes in parallel.
            // Source resolution: a web preview replaces an unplayable original,
            // it is not a ladder rung.
            crate::conversion::convert_video(input_str, output_str, hwaccel, fallback, None, None)
                .await
        }
        _ => false,
    };

    if ffmpeg_ok {
        return true;
    }

    tracing::warn!(
        input = %input_path.display(),
        target = plan.ext,
        mode = ?plan.mode,
        "Web preview: FFmpeg conversion failed"
    );
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCODE_MP4: WebPreview = WebPreview {
        ext: "mp4",
        mode: PreviewMode::Transcode,
    };
    const REMUX_MP4: WebPreview = WebPreview {
        ext: "mp4",
        mode: PreviewMode::Remux,
    };

    // ── Pure decision matrix ────────────────────────────────────────────────

    /// Images and audio are decided by extension and must never pay for a probe
    /// — a `.heic` is HEIC whatever ffprobe would say about it.
    #[test]
    fn stills_and_audio_are_extension_only() {
        for (name, ext) in [
            ("a.heic", "jpg"),
            ("a.HEIF", "jpg"),
            ("a.tiff", "jpg"),
            ("a.ico", "png"),
            ("a.wma", "mp3"),
            ("a.aiff", "mp3"),
        ] {
            assert!(!preview_needs_probe(name), "{name} must not be probed");
            assert_eq!(
                resolve_web_preview(name, None),
                Some(WebPreview {
                    ext,
                    mode: PreviewMode::Transcode
                }),
                "{name}"
            );
        }
        // Already browser-native stills need nothing.
        for name in ["a.jpg", "a.png", "a.webp", "a.gif", "noextension"] {
            assert_eq!(resolve_web_preview(name, None), None, "{name}");
        }
    }

    /// The false negative this change exists to close: a native-looking
    /// container holding a codec no browser decodes. The extension-only answer
    /// (`stream_is_native = None`, which is literally the old code path) is
    /// `None` — that is the bug, asserted here so it cannot come back quietly.
    #[test]
    fn a_non_native_codec_in_an_mp4_is_previewed() {
        assert_eq!(
            resolve_web_preview("hevc.mp4", None),
            None,
            "precondition: the extension-only verdict for .mp4 is 'no preview needed'"
        );
        assert_eq!(resolve_web_preview("hevc.mp4", Some(false)), Some(TRANSCODE_MP4));
        assert_eq!(resolve_web_preview("hevc.MP4", Some(false)), Some(TRANSCODE_MP4));
    }

    /// A genuinely native `.mp4` must stay untouched. Getting this wrong
    /// re-encodes almost the entire library into its own encrypted payloads.
    #[test]
    fn a_native_mp4_is_left_alone() {
        assert_eq!(resolve_web_preview("native.mp4", Some(true)), None);
    }

    /// The false positive: a `.mov` carrying ordinary H.264. It still needs an
    /// MP4 wrapper — browsers do not accept `video/quicktime` — but it must be
    /// a stream copy, not a re-encode of pixels that were already fine.
    #[test]
    fn a_native_mov_is_remuxed_not_re_encoded() {
        assert_eq!(resolve_web_preview("clip.mov", Some(true)), Some(REMUX_MP4));
        assert_eq!(resolve_web_preview("clip.m4v", Some(true)), Some(REMUX_MP4));
        // The old, extension-only answer paid for a full transcode instead.
        assert_eq!(resolve_web_preview("clip.mov", None), Some(TRANSCODE_MP4));
    }

    /// A `.mov` that really is non-native still gets the full encode.
    #[test]
    fn a_non_native_mov_is_still_transcoded() {
        assert_eq!(resolve_web_preview("prores.mov", Some(false)), Some(TRANSCODE_MP4));
    }

    /// Containers that are unambiguous by extension never reach the probe, and
    /// a probe result must not change their answer if one somehow arrives.
    #[test]
    fn unambiguous_video_containers_skip_the_probe() {
        for name in ["a.mkv", "a.avi", "a.wmv", "a.asf", "a.mpg", "a.3gp"] {
            assert!(!preview_needs_probe(name), "{name} must not be probed");
            assert_eq!(resolve_web_preview(name, None), Some(TRANSCODE_MP4), "{name}");
        }
    }

    /// `.webm` is excluded on purpose: `is_browser_native` is an H.264-only
    /// allowlist, so probing a VP9 WebM would report "not native" and re-encode
    /// a file every target browser already plays.
    #[test]
    fn webm_is_never_probed_or_previewed() {
        assert!(!preview_needs_probe("a.webm"));
        assert_eq!(resolve_web_preview("a.webm", None), None);
        assert_eq!(
            resolve_web_preview("a.webm", Some(false)),
            None,
            "even a 'not native' verdict must not drag WebM into a re-encode"
        );
    }

    /// An unprobeable file falls back to the extension verdict, unchanged. This
    /// is the safety property: a missing ffprobe degrades to the old behaviour
    /// rather than to an unplayable payload or a library-wide re-encode.
    #[test]
    fn an_unprobeable_file_keeps_the_old_extension_behaviour() {
        assert_eq!(resolve_web_preview("clip.mp4", None), needs_web_preview("clip.mp4").map(|ext| WebPreview { ext, mode: PreviewMode::Transcode }));
        assert_eq!(resolve_web_preview("clip.mov", None), Some(TRANSCODE_MP4));
        assert_eq!(resolve_web_preview("clip.mkv", None), Some(TRANSCODE_MP4));
    }

    // ── Real FFmpeg fixtures through the real probe ─────────────────────────
    //
    // The defect being guarded is precisely that the old path never looked
    // inside the file, so a pure test cannot demonstrate it end to end.
    // Skipped when FFmpeg (or a given encoder) is unavailable.

    fn make_fixture(name: &str, vcodec: &str, extra: &[&str]) -> Option<std::path::PathBuf> {
        let path = std::env::temp_dir().join(format!("sp_webprev_{}_{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut args: Vec<String> = vec![
            "-y".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=duration=1:size=320x240:rate=10".into(),
            "-c:v".into(),
            vcodec.into(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        args.push(path.to_string_lossy().to_string());

        let ok = std::process::Command::new("ffmpeg")
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok && std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
            Some(path)
        } else {
            let _ = std::fs::remove_file(&path);
            None
        }
    }

    /// End-to-end for the false negative. On the pre-fix tree this file was
    /// encrypted as-is and no browser could play the resulting payload.
    #[tokio::test]
    async fn hevc_in_mp4_is_planned_for_a_preview() {
        let Some(path) = make_fixture(
            "hevc.mp4",
            "libx265",
            &[
                "-x265-params",
                "log-level=none",
                "-pix_fmt",
                "yuv420p",
                "-tag:v",
                "hvc1",
            ],
        ) else {
            eprintln!("ffmpeg/libx265 unavailable — skipping");
            return;
        };

        assert!(
            needs_web_preview("hevc.mp4").is_none(),
            "precondition: the extension-only path stores this unplayable file verbatim"
        );

        let plan = plan_web_preview(&path, "hevc.mp4").await;
        let _ = std::fs::remove_file(&path);

        assert_eq!(plan, Some(TRANSCODE_MP4));
    }

    /// End-to-end for the "leave it alone" half.
    #[tokio::test]
    async fn native_h264_mp4_needs_no_preview() {
        let Some(path) = make_fixture(
            "native.mp4",
            "libx264",
            &["-profile:v", "high", "-pix_fmt", "yuv420p"],
        ) else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let plan = plan_web_preview(&path, "native.mp4").await;
        let _ = std::fs::remove_file(&path);

        assert_eq!(plan, None);
    }

    /// End-to-end for the false positive, plus proof that the remux it now
    /// chooses actually produces a playable MP4 with the *same* video codec.
    #[tokio::test]
    async fn native_h264_mov_is_remuxed_and_the_output_is_a_real_mp4() {
        let Some(path) = make_fixture(
            "native.mov",
            "libx264",
            &["-profile:v", "high", "-pix_fmt", "yuv420p"],
        ) else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let plan = plan_web_preview(&path, "native.mov").await;
        assert_eq!(
            plan,
            Some(REMUX_MP4),
            "an H.264 .mov must be rewrapped, not re-encoded"
        );

        let out = std::env::temp_dir()
            .join(format!("sp_webprev_{}_remuxed.mp4", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let ok = generate_web_preview_bg(&path, &out, plan.unwrap()).await;

        let probed = if ok {
            crate::transcode::probe::probe_video_stream(&out).await.ok()
        } else {
            None
        };
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&out);

        assert!(ok, "the remux must succeed for plain H.264 in a .mov");
        let info = probed.expect("the remuxed file must probe as a video");
        assert_eq!(
            info.codec.to_ascii_lowercase(),
            "h264",
            "a remux copies the stream — the codec must be unchanged"
        );
        assert!(crate::transcode::probe::is_browser_native(&info));
    }

    /// An audio-only `.mp4` has no video stream. The probe errors, the plan
    /// falls back to the extension verdict (`None`), and the file is stored as
    /// it is — which is correct: browsers play its audio track fine.
    #[tokio::test]
    async fn an_audio_only_mp4_is_stored_as_is() {
        let path =
            std::env::temp_dir().join(format!("sp_webprev_{}_audio.mp4", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=1",
                "-c:a",
                "aac",
            ])
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok || std::fs::metadata(&path).map(|m| m.len() == 0).unwrap_or(true) {
            eprintln!("ffmpeg/aac unavailable — skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }

        let plan = plan_web_preview(&path, "audio.mp4").await;
        let _ = std::fs::remove_file(&path);

        assert_eq!(plan, None);
    }
}
