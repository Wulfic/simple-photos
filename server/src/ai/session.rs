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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

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
    build_session_with_threads(path, current().num_threads)
}

/// Like [`build_session`] but with an explicit intra-op thread count, used by
/// [`build_session_pool`] to hand each pooled session a *divided* slice of the
/// core budget so `pool_size` concurrent inferences don't oversubscribe the CPU.
pub fn build_session_with_threads(path: &Path, num_threads: usize) -> anyhow::Result<Session> {
    let cfg = current();
    let num_threads = num_threads.max(1);
    let builder = Session::builder().map_err(|e| anyhow::anyhow!("Session::builder: {e}"))?;
    let builder = builder
        .with_intra_threads(num_threads)
        .map_err(|e| anyhow::anyhow!("with_intra_threads({num_threads}): {e}"))?;

    let mut builder = apply_execution_provider(builder, cfg.provider, path, num_threads);

    builder
        .commit_from_file(path)
        .map_err(|e| anyhow::anyhow!("commit_from_file({}): {e}", path.display()))
}

// ── Multi-session pool (CPU parallelism) ─────────────────────────────────────

/// Minimum intra-op threads kept per pooled session. Below this, ONNX Runtime's
/// own thread pool has too little to work with and per-inference latency spikes,
/// so it bounds how many sessions we split the cores into.
const AI_MIN_THREADS_PER_SESSION: usize = 2;

/// Default ceiling on pooled sessions. Each session re-loads the model weights
/// (the ArcFace recogniser alone is ~166 MB), so this caps the *memory* cost of
/// parallelism on a many-core host. Operators who want to trade RAM for more
/// throughput raise it via `SIMPLE_PHOTOS_AI_JOBS`.
const AI_MAX_SESSIONS: usize = 8;

/// Operator override for the AI session-pool size via `SIMPLE_PHOTOS_AI_JOBS`.
/// `Some(n)` (n ≥ 1) pins the pool size; unset / unparseable / non-positive
/// yields `None` (fully automatic).
fn ai_jobs_override() -> Option<usize> {
    std::env::var("SIMPLE_PHOTOS_AI_JOBS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
}

/// Plan the AI session pool from a core `budget` (physical cores, or the
/// `[ai].threads` config override): `(pool_size, threads_per_session)`.
///
/// `pool_size * threads_per_session ≈ budget`, so running `pool_size` inferences
/// at once uses roughly all the cores in aggregate without oversubscribing them
/// — the same total CPU as the old single session, just spread across parallel
/// photos. Scales from a single core (pool of 1 = serial) upward, capped by
/// [`AI_MAX_SESSIONS`] for memory unless overridden.
pub fn ai_pool_plan_for(budget: usize) -> (usize, usize) {
    let budget = budget.max(1);
    let pool_size = match ai_jobs_override() {
        // Honour the override, but keep it sane so a typo can't spawn hundreds
        // of model copies. `threads_per` floors at 1, so an override above the
        // core count intentionally oversubscribes (operator's call).
        Some(jobs) => jobs.clamp(1, 64),
        None => (budget / AI_MIN_THREADS_PER_SESSION).clamp(1, AI_MAX_SESSIONS),
    };
    let threads_per = (budget / pool_size).max(1);
    (pool_size, threads_per)
}

/// [`ai_pool_plan_for`] applied to the active thread budget (which is
/// `num_cpus::get_physical()` unless `[ai].threads` was set in config).
pub fn ai_pool_plan() -> (usize, usize) {
    ai_pool_plan_for(current().num_threads)
}

/// A small pool of independent ONNX sessions over the same model file, so the
/// AI batch can run several inferences truly in parallel on a multi-core host.
///
/// The per-session `Mutex` still serialises two callers that land on the same
/// slot, but [`acquire`](Self::acquire) round-robins across the pool so with
/// `pool_size` workers and `pool_size` sessions each worker usually gets its
/// own. Each session was built with a divided thread budget (see
/// [`ai_pool_plan`]), so aggregate CPU use matches the old single-session path.
pub struct SessionPool {
    sessions: Vec<Arc<Mutex<Session>>>,
    next: AtomicUsize,
}

impl SessionPool {
    /// Round-robin the next session handle. Cheap `Arc` clone; the caller locks
    /// it for the duration of one inference. Never empty — `build_session_pool`
    /// always creates at least one session.
    pub fn acquire(&self) -> Arc<Mutex<Session>> {
        let i = self.next.fetch_add(1, Ordering::Relaxed) % self.sessions.len();
        self.sessions[i].clone()
    }
}

/// Build a [`SessionPool`] for `path`, sizing it via [`ai_pool_plan`] and giving
/// each session a divided intra-op thread budget. Falls back to a single session
/// if only one is planned (single-core host / `SIMPLE_PHOTOS_AI_JOBS=1`).
pub fn build_session_pool(path: &Path) -> anyhow::Result<SessionPool> {
    let (pool_size, threads_per) = ai_pool_plan();
    let mut sessions = Vec::with_capacity(pool_size);
    for _ in 0..pool_size {
        sessions.push(Arc::new(Mutex::new(build_session_with_threads(
            path,
            threads_per,
        )?)));
    }
    tracing::info!(
        model = %path.display(),
        pool_size,
        threads_per,
        "ONNX session pool built (CPU parallelism)"
    );
    Ok(SessionPool {
        sessions,
        next: AtomicUsize::new(0),
    })
}

#[cfg(feature = "cuda")]
fn apply_execution_provider(
    builder: SessionBuilder,
    provider: ExecutionProvider,
    path: &Path,
    num_threads: usize,
) -> SessionBuilder {
    use ort::execution_providers::CUDAExecutionProvider;

    if provider != ExecutionProvider::Cuda {
        return builder;
    }
    let cuda_ep = CUDAExecutionProvider::default();
    // `error_on_failure()` makes ONNX Runtime return an error if the CUDA EP
    // cannot be registered (e.g. the CUDA 12 runtime DLLs are missing) instead
    // of silently falling back to CPU while we log a misleading "registered"
    // message. We then perform an explicit, logged CPU fallback below.
    match builder.with_execution_providers([cuda_ep.build().error_on_failure()]) {
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
            Session::builder()
                .ok()
                .and_then(|b| b.with_intra_threads(num_threads).ok())
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
    _num_threads: usize,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_plan_single_core_is_serial() {
        // A 1-core budget must produce a pool of 1 (i.e. the old serial path).
        if ai_jobs_override().is_some() {
            return; // env override in effect; auto formula not exercised
        }
        assert_eq!(ai_pool_plan_for(1), (1, 1));
        // 2 cores: still one session, but it keeps both threads.
        assert_eq!(ai_pool_plan_for(2), (1, 2));
    }

    #[test]
    fn pool_plan_divides_cores_without_oversubscribing() {
        if ai_jobs_override().is_some() {
            return;
        }
        for budget in [4usize, 8, 16, 32, 64, 128] {
            let (pool, threads) = ai_pool_plan_for(budget);
            assert!(pool >= 1 && threads >= 1);
            // Never split so far that a session drops below the useful minimum.
            assert!(
                threads >= AI_MIN_THREADS_PER_SESSION || pool == 1,
                "budget {budget}: {threads} threads/session is below the floor"
            );
            // Aggregate threads must not exceed the core budget (no oversubscribe).
            assert!(
                pool * threads <= budget,
                "budget {budget}: pool {pool} * threads {threads} oversubscribes"
            );
            // Memory guard: never more than the default ceiling on the auto path.
            assert!(pool <= AI_MAX_SESSIONS);
        }
    }

    #[test]
    fn pool_plan_scales_up_on_a_many_core_host() {
        if ai_jobs_override().is_some() {
            return;
        }
        // A big CPU should run several inferences at once, each still adequately
        // threaded — that's the whole point of the pool.
        let (pool, threads) = ai_pool_plan_for(64);
        assert_eq!(pool, AI_MAX_SESSIONS);
        assert!(threads >= AI_MIN_THREADS_PER_SESSION);
    }
}
