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
        Err(_) => Err(format!(
            "process timed out after {}s",
            timeout.as_secs()
        )),
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
        Err(_) => Err(format!(
            "process timed out after {}s",
            timeout.as_secs()
        )),
    }
}
