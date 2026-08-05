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
/// request-serving threads **or the import pipeline**, so we lower BOTH its CPU
/// scheduling priority and its disk-I/O priority:
///
/// * **Unix** — wrap with `nice -n 19` (CPU) and, when the `ionice` binary is
///   available, `ionice -c 3` (idle I/O class). `nice` alone only yields the
///   CPU; on a spinning HDD the real bottleneck during a big import is disk
///   seeks, and an un-throttled ffmpeg transcode reading/writing the disk will
///   stall every other request and the import copy itself (the "conversion
///   makes everything lag / hits the HDD hard" report). Idle-class I/O means
///   the transcode only gets disk time when nothing else wants it, so imports
///   and page loads stay responsive while conversion takes a genuine back seat.
///   The final command chains `nice -n 19 ionice -c 3 <program> <args…>`; each
///   wrapper `exec`s the next and the flags are inherited by the ffmpeg child.
/// * **Windows** — there is no `nice`/`ionice`, so spawn `program` directly with
///   the `BELOW_NORMAL_PRIORITY_CLASS` creation flag (CPU). (Windows has no
///   per-child idle-I/O class — `PROCESS_MODE_BACKGROUND_BEGIN` only applies to
///   the calling process — so I/O de-prioritisation is Unix-only.)
///
/// `ionice` availability is probed once and cached: if it is missing (e.g. a
/// busybox image without util-linux) we silently fall back to `nice` only, so a
/// missing binary can never break spawning — mirroring the care taken after the
/// historic `nice`-on-Windows bug that broke every conversion.
///
/// Callers append the program's own arguments to the returned `Command`.
pub fn background_command(program: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("nice");
        cmd.arg("-n").arg("19");
        // Idle I/O class so disk-heavy transcodes yield to imports + requests.
        if ionice_available() {
            cmd.arg("ionice").arg("-c").arg("3");
        }
        cmd.arg(program);
        cmd
    }
    #[cfg(windows)]
    {
        // BELOW_NORMAL_PRIORITY_CLASS lowers scheduling priority without
        // dropping fully to idle, mirroring the CPU intent of `nice -n 19`.
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

/// Whether the `ionice` binary is present on `PATH`, probed once and cached.
///
/// A plain filesystem check (no process spawn) so it's cheap and side-effect
/// free. Used to decide whether [`background_command`] can lower disk-I/O
/// priority on Unix; when absent we fall back to `nice` only.
#[cfg(unix)]
fn ionice_available() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::env::var_os("PATH")
            .map(|paths| {
                std::env::split_paths(&paths).any(|dir| {
                    let candidate = dir.join("ionice");
                    std::fs::metadata(&candidate)
                        .map(|m| m.is_file())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
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
            // Always de-prioritises CPU via `nice -n 19` and always ends by
            // invoking the real program. `ionice -c 3` is inserted only when the
            // binary is present on this host, so accept both shapes.
            assert_eq!(&args[0..2], &["-n", "19"], "CPU nice level first");
            assert_eq!(args.last().unwrap(), "ffmpeg", "program invoked last");
            if super::ionice_available() {
                assert_eq!(
                    args,
                    vec!["-n", "19", "ionice", "-c", "3", "ffmpeg"],
                    "when ionice exists, idle I/O class must wrap the program"
                );
            } else {
                assert_eq!(
                    args,
                    vec!["-n", "19", "ffmpeg"],
                    "without ionice, fall back to nice-only"
                );
            }
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
