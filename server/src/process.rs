//! Utility helpers for running external processes (ffmpeg, ffprobe) with
//! timeouts and safety defaults.
//!
//! Every child process is spawned with:
//! - `stdin(null)` — prevents hangs from interactive prompts
//! - `kill_on_drop(true)` — kills the child if the future is dropped/cancelled
//! - A caller-specified timeout — returns an error instead of hanging forever

use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

/// Default timeout for ffmpeg render operations (video encoding).
pub const FFMPEG_RENDER_TIMEOUT: Duration = Duration::from_secs(120);

/// Default timeout for quick ffprobe / ffmpeg probe operations.
pub const FFPROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Default timeout for thumbnail extraction (single frame or short GIF).
pub const THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(30);

/// Build a low-priority background command for `program` (ffmpeg, convert, …).
///
/// Heavy media work (transcodes, thumbnails, previews) must not starve the
/// request-serving threads, so we lower its CPU scheduling priority:
///
/// * **Unix** — wrap with `nice -n 19`; `program` becomes `nice`'s argument
///   and the caller appends the program's own arguments afterwards.
/// * **Windows** — there is no `nice` executable, so spawn `program` directly
///   with the `BELOW_NORMAL_PRIORITY_CLASS` creation flag. Wrapping with
///   `nice` on Windows fails to spawn (no such program), which previously
///   broke every FFmpeg / ImageMagick conversion, thumbnail, and web preview.
///
/// Callers append the program's own arguments to the returned `Command`.
pub fn background_command(program: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("nice");
        cmd.arg("-n").arg("19").arg(program);
        cmd
    }
    #[cfg(windows)]
    {
        // BELOW_NORMAL_PRIORITY_CLASS lowers scheduling priority without
        // dropping fully to idle, mirroring the intent of `nice -n 19`.
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        let mut cmd = Command::new(program);
        cmd.creation_flags(BELOW_NORMAL_PRIORITY_CLASS);
        cmd
    }
    #[cfg(not(any(unix, windows)))]
    {
        Command::new(program)
    }
}

/// Run a `Command` to completion, collecting stdout+stderr (like `.output()`),
/// but with `stdin(null)`, `kill_on_drop(true)`, and a timeout.
///
/// Returns `Ok(Output)` on success, or `Err(String)` describing the failure.
pub async fn run_with_timeout(
    cmd: &mut Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    cmd.stdin(Stdio::null()).kill_on_drop(true);

    let child = cmd.output();

    match tokio::time::timeout(timeout, child).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(format!("process spawn/io failed: {e}")),
        Err(_) => Err(format!("process timed out after {}s", timeout.as_secs())),
    }
}

/// Run a `Command` and wait only for its exit status (like `.status()`),
/// with `stdin(null)`, `kill_on_drop(true)`, and a timeout.
///
/// Returns `Ok(ExitStatus)` on success, or `Err(String)` describing the failure.
pub async fn status_with_timeout(
    cmd: &mut Command,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    cmd.stdin(Stdio::null()).kill_on_drop(true);

    let child = cmd.status();

    match tokio::time::timeout(timeout, child).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(e)) => Err(format!("process spawn/io failed: {e}")),
        Err(_) => Err(format!("process timed out after {}s", timeout.as_secs())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On Windows the background command must invoke the target program
    /// **directly** (never `nice`, which does not exist on Windows and made
    /// every FFmpeg / ImageMagick conversion fail to spawn). On Unix it must
    /// wrap with `nice` so heavy work is de-prioritised.
    #[test]
    fn background_command_uses_correct_program_per_platform() {
        let cmd = background_command("ffmpeg");
        let program = cmd.as_std().get_program().to_string_lossy().to_string();

        #[cfg(windows)]
        assert_eq!(
            program, "ffmpeg",
            "Windows must spawn the program directly, not via `nice`"
        );

        #[cfg(unix)]
        {
            assert_eq!(program, "nice", "Unix must wrap with `nice`");
            let args: Vec<String> = cmd
                .as_std()
                .get_args()
                .map(|a| a.to_string_lossy().to_string())
                .collect();
            assert_eq!(args, vec!["-n", "19", "ffmpeg"]);
        }
    }

    /// End-to-end regression guard for the Windows `nice` spawn bug: build a
    /// real FFmpeg invocation through `background_command` and confirm it both
    /// spawns and produces a valid output file. Skipped when FFmpeg is not on
    /// PATH (e.g. minimal CI images) so the suite stays green everywhere.
    #[tokio::test]
    async fn background_command_spawns_ffmpeg_and_transcodes() {
        // Skip gracefully if ffmpeg isn't installed on this host.
        if Command::new("ffmpeg")
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| !s.success())
            .unwrap_or(true)
        {
            eprintln!("ffmpeg not available — skipping spawn/transcode test");
            return;
        }

        let out = std::env::temp_dir().join(format!("sp_bgcmd_{}.mp4", std::process::id()));
        let out_str = out.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&out);

        // Generate a 1-second test clip entirely in FFmpeg, encoded with the
        // CPU x264 encoder so the test is GPU-independent. The whole point is
        // proving the *spawn* path works on this platform.
        let mut cmd = background_command("ffmpeg");
        cmd.args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=1:size=320x240:rate=15",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            &out_str,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());

        let status = status_with_timeout(&mut cmd, Duration::from_secs(60)).await;

        assert!(
            matches!(status, Ok(s) if s.success()),
            "background_command failed to spawn/run ffmpeg: {status:?}"
        );
        let meta = std::fs::metadata(&out).expect("output file should exist");
        assert!(meta.len() > 0, "transcoded output should be non-empty");

        let _ = std::fs::remove_file(&out);
    }
}
