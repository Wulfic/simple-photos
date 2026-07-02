//! Video frame selection for face detection (item #8).
//!
//! Face detection on videos used to run on the poster thumbnail (~10% in),
//! which frequently misses people entirely — establishing shots, title cards,
//! black intros. This module instead extracts the frame **~5 s in** (the
//! `fps × 5` frame), where a subject is far more likely to be present, and lets
//! the caller slide **±2 s** when that frame has no detectable face.
//!
//! Everything here is bounded and best-effort:
//!   * the encrypted video is **streamed** to a temp file (never buffered whole
//!     in RAM — the OOM hazard that made the old code refuse videos), and
//!   * one JPEG frame is pulled per probe via a seek-first ffmpeg call.
//!
//! ANY failure returns `None`/`None`, so the caller falls back to the existing
//! thumbnail path and the AI pipeline never destabilises (cf. item #16).

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;

/// Per-frame ffmpeg ceiling — a single seek+decode is quick even for 4K.
const FRAME_TIMEOUT: Duration = Duration::from_secs(60);
/// Primary face-detection seek — ~5 s in (the "fps × 5" frame).
const PRIMARY_SECS: f64 = 5.0;
/// ±window tried when the primary frame has no face (→ 3 s and 7 s).
const WINDOW_SECS: f64 = 2.0;

/// A decrypted, on-disk video ready for frame extraction. Deletes its temp file
/// (when it created one) on drop.
pub struct VideoFrameSource {
    path: PathBuf,
    duration: f64,
    temp: bool,
}

impl Drop for VideoFrameSource {
    fn drop(&mut self) {
        if self.temp {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl VideoFrameSource {
    /// Timestamps to try, in order: 5 s, then 3 s and 7 s (clamped to the clip,
    /// de-duplicated). Short clips collapse toward the start.
    pub fn candidate_secs(&self) -> Vec<f64> {
        if self.duration <= 0.0 {
            // Duration unknown — a single attempt at the nominal 5 s. ffmpeg
            // clamps to the last frame if the clip is shorter.
            return vec![PRIMARY_SECS];
        }
        // Small margin before EOF so ffmpeg always lands on a real frame.
        let max_t = (self.duration - 0.1).max(0.0);
        let mut out: Vec<f64> = Vec::with_capacity(3);
        for t in [
            PRIMARY_SECS,
            PRIMARY_SECS - WINDOW_SECS,
            PRIMARY_SECS + WINDOW_SECS,
        ] {
            let c = t.clamp(0.0, max_t);
            if !out.iter().any(|&x| (x - c).abs() < 0.01) {
                out.push(c);
            }
        }
        out
    }

    /// Extract a single JPEG frame at `secs`. `None` on any ffmpeg failure.
    ///
    /// Scales the longest edge to ≤ 1280 px — big enough for reliable face
    /// detection, small enough to keep decode/detect cheap.
    pub async fn frame_at(&self, secs: f64) -> Option<Vec<u8>> {
        let dst = temp_path("spframe", "jpg");
        // -ss before -i = fast keyframe seek; setsar normalises non-square PARs.
        let mut cmd = tokio::process::Command::new("ffmpeg");
        cmd.args(["-y", "-ss", &format!("{secs:.2}"), "-i"])
            .arg(&self.path)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "setsar=1,scale=1280:1280:force_original_aspect_ratio=decrease",
                "-q:v",
                "3",
            ])
            .arg(&dst)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let ok = matches!(
            crate::process::status_with_timeout(&mut cmd, FRAME_TIMEOUT).await,
            Ok(s) if s.success()
        );
        let bytes = if ok {
            tokio::fs::read(&dst).await.ok()
        } else {
            None
        };
        let _ = tokio::fs::remove_file(&dst).await;
        bytes.filter(|b| b.len() >= 100)
    }
}

/// Open a video for frame extraction. Prefers the plaintext original on disk;
/// otherwise streams the encrypted blob to a temp file. Returns `None` (→ caller
/// falls back to the thumbnail) on any failure.
pub async fn open(
    pool: &SqlitePool,
    storage_root: &Path,
    jwt_secret: &str,
    file_path: &str,
    encrypted_blob_id: Option<&str>,
    user_id: &str,
) -> Option<VideoFrameSource> {
    // 1. Plaintext original still on disk (unencrypted mode / pre-encryption).
    if !file_path.is_empty() {
        let abs = storage_root.join(file_path);
        if tokio::fs::try_exists(&abs).await.unwrap_or(false) {
            let duration = crate::photos::thumbnail::probe_duration(&abs)
                .await
                .unwrap_or(0.0);
            return Some(VideoFrameSource {
                path: abs,
                duration,
                temp: false,
            });
        }
    }

    // 2. Encrypted blob → stream-decrypt to a temp file (bounded RAM).
    let blob_id = encrypted_blob_id?;
    if blob_id.is_empty() {
        return None;
    }
    let key = crate::crypto::load_wrapped_key(pool, jwt_secret)
        .await
        .ok()
        .flatten()?;
    let (storage_path,): (String,) =
        sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
            .bind(blob_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()?;

    let src = storage_root.join(&storage_path);
    let dst = temp_path("spvideo", "bin");
    let src_c = src.clone();
    let dst_c = dst.clone();
    let decrypted = tokio::task::spawn_blocking(move || {
        crate::blobs::chunked::decrypt_blob_file_to_file(&key, &src_c, &dst_c)
    })
    .await;

    match decrypted {
        Ok(Ok(())) => {
            let duration = crate::photos::thumbnail::probe_duration(&dst)
                .await
                .unwrap_or(0.0);
            Some(VideoFrameSource {
                path: dst,
                duration,
                temp: true,
            })
        }
        _ => {
            // Clean up a partially-written temp file on decrypt failure.
            let _ = tokio::fs::remove_file(&dst).await;
            None
        }
    }
}

/// A unique temp path under the system temp dir. Cleaned up by callers /
/// [`VideoFrameSource`]'s `Drop`.
fn temp_path(prefix: &str, ext: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.{ext}", uuid::Uuid::new_v4()))
}
