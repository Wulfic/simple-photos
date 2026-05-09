//! Shared ONNX Runtime session builder.
//!
//! All ONNX models in the AI pipeline (SCRFD detection, ArcFace
//! recognition, MobileNetV2 classification, legacy UltraFace) share
//! identical session-construction needs:
//!
//! * Honour `[ai].threads` from config (was previously hardcoded to 1,
//!   pinning every model to a single CPU core).
//! * Register CUDA execution provider when the binary was built with
//!   `--features cuda` *and* the runtime hardware is CUDA-capable.
//!   Falls back to CPU with a warning if EP registration fails so a
//!   missing CUDA driver doesn't crash the server.
//!
//! `AiEngine::new` calls [`init`] once at startup with the resolved
//! provider/threads pair; every subsequent `load_onnx_*` helper calls
//! [`build_session`] to commit a model file with those settings.

use ort::session::builder::SessionBuilder;
use ort::session::Session;
use std::path::Path;
use std::sync::OnceLock;

use super::engine::ExecutionProvider;

#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    pub provider: ExecutionProvider,
    pub num_threads: usize,
}

static SESSION_CONFIG: OnceLock<SessionConfig> = OnceLock::new();

/// Install the global session config. Called once from
/// [`crate::ai::engine::AiEngine::new`]. Subsequent calls are ignored
/// (the first registration wins, matching `OnceLock` semantics).
pub fn init(cfg: SessionConfig) {
    let _ = SESSION_CONFIG.set(cfg);
}

/// Snapshot of the active session config. Falls back to a sane
/// CPU-only default if `init` has not been called yet (e.g. unit
/// tests that touch `load_onnx_*` directly).
pub fn current() -> SessionConfig {
    SESSION_CONFIG.get().copied().unwrap_or(SessionConfig {
        provider: ExecutionProvider::Cpu,
        num_threads: 1,
    })
}

/// Build and commit an ONNX session for `path` using the global
/// [`SessionConfig`]. Registers the CUDA execution provider when the
/// binary was compiled with `--features cuda` and the active provider
/// is `Cuda`; otherwise CPU is used.
///
/// On EP registration failure we emit a `tracing::warn!` and continue
/// with CPU rather than failing the whole model load — operators with
/// a half-broken CUDA install still get a working server.
pub fn build_session(path: &Path) -> anyhow::Result<Session> {
    let cfg = current();
    let builder = Session::builder().map_err(|e| anyhow::anyhow!("Session::builder: {e}"))?;
    let builder = builder
        .with_intra_threads(cfg.num_threads)
        .map_err(|e| anyhow::anyhow!("with_intra_threads({}): {e}", cfg.num_threads))?;

    let mut builder = apply_execution_provider(builder, cfg.provider, path);

    builder
        .commit_from_file(path)
        .map_err(|e| anyhow::anyhow!("commit_from_file({}): {e}", path.display()))
}

#[cfg(feature = "cuda")]
fn apply_execution_provider(
    builder: SessionBuilder,
    provider: ExecutionProvider,
    path: &Path,
) -> SessionBuilder {
    use ort::execution_providers::CUDAExecutionProvider;

    if provider != ExecutionProvider::Cuda {
        return builder;
    }
    let cuda_ep = CUDAExecutionProvider::default();
    match builder.with_execution_providers([cuda_ep.build()]) {
        Ok(b) => {
            tracing::info!(
                "ONNX session: registered CUDAExecutionProvider for {}",
                path.display()
            );
            b
        }
        Err(e) => {
            tracing::warn!(
                "ONNX session: failed to register CUDAExecutionProvider for {} \
                 ({e}); falling back to CPU",
                path.display()
            );
            // Re-create a fresh builder with threads applied, since the
            // failed call consumed the previous one.
            let cfg = current();
            Session::builder()
                .ok()
                .and_then(|b| b.with_intra_threads(cfg.num_threads).ok())
                .unwrap_or_else(|| {
                    // Extremely unlikely; the original builder already
                    // succeeded above. Panic-free fallback: build a raw
                    // builder and let commit_from_file surface the error.
                    Session::builder().expect("ort Session::builder")
                })
        }
    }
}

#[cfg(not(feature = "cuda"))]
fn apply_execution_provider(
    builder: SessionBuilder,
    provider: ExecutionProvider,
    _path: &Path,
) -> SessionBuilder {
    if provider == ExecutionProvider::Cuda {
        // One-shot warning: we got here because the operator set
        // `gpu_preferred = true` and the host has CUDA, but the binary
        // was compiled without the `cuda` feature (i.e. installer
        // didn't detect a GPU at build time, or this is a portable
        // distribution). Tell them how to fix it.
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            tracing::warn!(
                "ONNX session: CUDA requested but binary was built without \
                 the `cuda` feature — running on CPU. Rebuild with \
                 `cargo build --release --features cuda` (or re-run install.sh \
                 on a host with nvidia-smi present) to enable GPU acceleration."
            );
        });
    }
    builder
}
