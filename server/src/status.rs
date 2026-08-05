//! Server-authoritative encryption status — the single source of truth for the
//! "Encrypting photos…" banner shown by **every** client (web + Android).
//!
//! Historically each client counted pending-encryption photos itself by
//! paginating `/api/photos/encrypted-sync` and running its own batch/ETA state
//! machine. Web and Android drifted apart (different totals, different ETAs)
//! and Android's local upload queue was invisible to the server total.
//!
//! This module centralises that logic:
//!   * `GET  /api/status/encryption`            — aggregated totals + ETA.
//!   * `POST /api/status/encryption/contribute` — a client reports the number
//!     of items it still has queued for upload (e.g. Android local backup), so
//!     the server total reflects work the server can't see yet (item #2).
//!
//! Batch + ETA tracking lives here (shared per-user), so two devices polling the
//! same endpoint observe identical `total`, `done`, and `eta_seconds` — the
//! acceptance criterion for item #1.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// A contribution goes stale if a client stops refreshing it. Without expiry an
/// Android device that dies mid-upload would inflate the server total forever.
/// 90 s comfortably covers the clients' 2 s heartbeat plus network jitter.
const CONTRIBUTION_TTL_SECS: u64 = 90;

/// Per-user, server-side batch tracker. Mirrors the state machine the web
/// `EncryptionBanner` used to run client-side, but shared across all of a
/// user's devices so they agree to the last item.
struct EncryptionBatch {
    /// Denominator for the progress bar. Set when the batch starts (idle → work)
    /// and expanded — never shrunk — when more work arrives mid-batch, so the
    /// bar advances monotonically instead of snapping backwards.
    batch_total: i64,
    /// When the batch started, used for the throughput-based ETA.
    started_at: Instant,
    /// Last observed total pending (server + client). Lets us detect growth.
    last_pending: i64,
}

/// A single client's self-reported pending upload count (item #2).
struct Contribution {
    count: i64,
    updated_at: Instant,
}

#[derive(Default)]
struct UserStatus {
    batch: Option<EncryptionBatch>,
    /// `source id` → its latest contribution. Keyed so a client can update its
    /// own figure idempotently and repeated heartbeats don't stack.
    contributions: HashMap<String, Contribution>,
}

// The count-based ETA estimator now lives in `crate::progress`, alongside the
// work-weighted one the conversion banner needs (#40). The encryption banner
// deliberately stays on this one: its queue items are one photo each, so there
// is no cost heterogeneity for a weighted estimator to correct.
use crate::progress::progress_math;

/// Global registry: `user_id` → tracking state. Guarded by a plain `Mutex`
/// because every critical section is a handful of map ops with no `.await`.
fn registry() -> &'static Mutex<HashMap<String, UserStatus>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, UserStatus>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Request body for `POST /api/status/encryption/contribute`.
#[derive(Debug, Deserialize)]
pub struct ContributeRequest {
    /// Stable per-device identifier so a device overwrites its own figure
    /// instead of stacking a new one on every heartbeat.
    pub source: String,
    /// Items the client still has queued for upload/encryption. `0` clears the
    /// contribution (client finished draining its queue).
    pub pending: i64,
}

#[derive(Debug, Serialize)]
pub struct ContributeResponse {
    pub ok: bool,
}

/// POST /api/status/encryption/contribute
///
/// Auth-gated: a caller can only ever affect **their own** aggregate total, so
/// a spoofed contribution can't poison another user's banner (item #1 risk:
/// "spoofed client contributions"). The `source` is namespaced under the
/// authenticated `user_id`, never trusted as a global key.
pub async fn contribute(
    State(_state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ContributeRequest>,
) -> Result<Json<ContributeResponse>, AppError> {
    // Guard against absurd values that would blow up the banner.
    let pending = req.pending.clamp(0, 10_000_000);
    let source = req.source.chars().take(128).collect::<String>();
    if source.is_empty() {
        return Err(AppError::BadRequest("source must not be empty".into()));
    }

    let mut reg = registry().lock().unwrap();
    let entry = reg.entry(auth.user_id.clone()).or_default();
    if pending <= 0 {
        entry.contributions.remove(&source);
    } else {
        entry.contributions.insert(
            source,
            Contribution {
                count: pending,
                updated_at: Instant::now(),
            },
        );
    }
    Ok(Json(ContributeResponse { ok: true }))
}

/// GET /api/status/encryption
///
/// Returns the authoritative encryption progress for the authenticated user:
/// server-side pending count + live client contributions, folded through a
/// shared batch tracker that yields a stable `total`, `done`, and `eta_seconds`.
pub async fn encryption_status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, AppError> {
    // ── Server-visible pending encryption count ──────────────────────────────
    // Mirrors the `encrypted-sync` eligibility filter (excludes secure-gallery
    // items) and counts rows still lacking an encrypted blob.
    //
    // `encryption_deferred` rows stay out of this count because they are not
    // *pending* — nothing will ever process them — and folding them in would
    // wedge the progress bar at a non-zero count forever. That exclusion is
    // correct and stays. What was wrong is that it was the ONLY treatment they
    // got: excluded from the bar and reported nowhere else, so 2,500 photos
    // (~17% of the live library) sat as plaintext originals at rest for a month
    // with every surface reporting "encryption complete". They are reported
    // separately as `parked` below (B3a).
    //
    // The only writer of `encryption_deferred = 1` is `record_encryption_failure`
    // at the attempt cap — this comment previously claimed a "Windows
    // LocalSystem import defer" path, which does not exist.
    let server_pending: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM photos p \
             WHERE p.user_id = ?1 \
             AND p.encrypted_blob_id IS NULL \
             AND p.encryption_deferred = 0 \
             AND p.id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND p.id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL)",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.read_pool)
    .await
    .unwrap_or(0);

    // Photos the encryption migration has given up on. Deliberately read
    // *outside* the batch tracker below and never added to `pending` — this is a
    // standing condition, not queued work, and the whole reason parked rows were
    // hidden in the first place is that feeding them to a progress bar wedges it.
    // Reporting them to the operator is a different question from counting them
    // as progress, and B3a is the case for answering the first one.
    let parked =
        crate::photos::server_migrate::count_parked(&state.read_pool, Some(auth.user_id.as_str()))
            .await;

    let mut reg = registry().lock().unwrap();
    let entry = reg.entry(auth.user_id.clone()).or_default();

    // Drop stale contributions, then sum what remains per source.
    let now = Instant::now();
    entry
        .contributions
        .retain(|_, c| now.duration_since(c.updated_at).as_secs() < CONTRIBUTION_TTL_SECS);

    let mut sources = serde_json::Map::new();
    sources.insert("server".to_string(), json!(server_pending));
    let mut client_pending: i64 = 0;
    for (src, c) in entry.contributions.iter() {
        client_pending += c.count;
        sources.insert(src.clone(), json!(c.count));
    }

    let total_pending = server_pending + client_pending;

    // ── Shared batch tracker (mirrors the old client state machine) ──────────
    let (batch_total, done, eta_seconds) = if total_pending == 0 {
        entry.batch = None;
        (0i64, 0i64, None)
    } else {
        match entry.batch.as_mut() {
            None => {
                // Idle → work: open a new batch.
                entry.batch = Some(EncryptionBatch {
                    batch_total: total_pending,
                    started_at: now,
                    last_pending: total_pending,
                });
                (total_pending, 0i64, None)
            }
            Some(batch) => {
                // More work arrived mid-batch → grow the denominator so the bar
                // never jumps backwards.
                if total_pending > batch.last_pending {
                    batch.batch_total += total_pending - batch.last_pending;
                }
                batch.last_pending = total_pending;

                let elapsed = now.duration_since(batch.started_at).as_secs_f64();
                let (done, eta) = progress_math(batch.batch_total, total_pending, elapsed);
                (batch.batch_total, done, eta)
            }
        }
    };

    let active = total_pending > 0;

    Ok(Json(json!({
        "active": active,
        "total": batch_total,
        "done": done,
        "pending": total_pending,
        "server_pending": server_pending,
        "client_pending": client_pending,
        // Parked photos: encryption failed `MIGRATION_MAX_ATTEMPTS` times and
        // was abandoned, leaving the original UNENCRYPTED at rest. Not part of
        // `pending`/`total`/`done` — a client must render this as a standing
        // warning with its own remedy (`/admin/encryption/retry-parked`), never
        // as remaining progress.
        "parked": parked,
        "eta_seconds": eta_seconds,
        // Per-source breakdown for debug UIs only; production banners read the
        // aggregate above.
        "sources": Value::Object(sources),
    })))
}

// `progress_math`'s unit tests moved to `crate::progress` with the function.
