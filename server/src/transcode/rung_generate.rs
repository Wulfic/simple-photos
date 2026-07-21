//! Producing the renditions the ladder plans (#49).
//!
//! [`rung_queue`](super::rung_queue) says which videos owe a rung,
//! [`ladder`](super::ladder) says what shape it should be, and
//! [`renditions`](super::renditions) records the result. This module is the part
//! that actually spends CPU: decrypt → probe → plan → transcode → encrypt →
//! record, once per candidate.
//!
//! # The probe is the only source of geometry
//!
//! Every dimension used here comes from probing the file about to be encoded.
//! `photos.width`/`height` are used by the SQL prefilter and nowhere else, for
//! two measured reasons (see [`rung_queue`](super::rung_queue) for the numbers):
//! 58 live videos have no recorded geometry at all, and for a large part of the
//! library the stored pair is transposed relative to what ffprobe reports.
//! Transposition is harmless to a rule keyed on `min(w, h)` and fatal to a
//! `scale=W:H`, which would squash a landscape frame into a portrait box.
//!
//! # Why this is a separate pass and not part of conversion
//!
//! A first-pass conversion is what makes a video playable at all; a rung is a
//! convenience. Running them together would mean a user waiting on a 4K
//! downscale before seeing any video, which is the one outcome `todo.md`
//! forbids. So this runs after `run_conversion_pass`, one file at a time,
//! bounded by a wall-clock budget.
//!
//! # Failure is expected and must be bounded
//!
//! Attempts are charged before ffmpeg starts, never after it fails — a file that
//! OOMs or hard-kills the encoder never reaches an error handler, and it is
//! exactly the file that must stop being retried. See
//! `036_video_rendition_attempts.sql`.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::blobs::{chunked, storage};

use super::ladder::{self, TIER_1080_SHORT_EDGE};
use super::probe;
use super::renditions::{upsert_rendition, StoredRendition};
use super::rung_queue::{self, RungCandidate, MAX_RUNG_ATTEMPTS};

/// Candidates fetched per sweep.
///
/// A ceiling on the query, not a target: [`SWEEP_TIME_BUDGET`] usually stops the
/// sweep first. It exists so a boot against the 114-candidate live backlog reads
/// a bounded number of rows rather than the whole set it will not finish.
const SWEEP_CANDIDATE_LIMIT: i64 = 16;

/// Wall-clock budget for one sweep, checked **between** files.
///
/// Never mid-encode: killing a 4K transcode at 90% to respect a budget wastes
/// everything it has done and charges an attempt for it. A single file may
/// therefore overrun this; the budget bounds how many files a sweep starts, not
/// how long the last one runs.
const SWEEP_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Serialises sweeps. Autoscan calls this from several sites and an interval
/// tick can land while the previous sweep is still encoding; two concurrent
/// sweeps would select the same candidates and burn two 4K encodes to produce
/// one rendition.
static SWEEP_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What one sweep did. Returned for logging and asserted by the tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepOutcome {
    pub examined: usize,
    pub produced: usize,
    pub not_needed: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// A file that must not outlive the step that made it.
///
/// The decrypted source of a 4K video is gigabytes of plaintext, and the whole
/// point of encrypted mode is that it does not sit on disk. Cleanup lives in
/// `Drop` rather than at the end of the happy path because every failure branch
/// here returns early, and a `?` that skipped the cleanup would leak the user's
/// decrypted video permanently.
struct ScratchFile(PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = ?self.0, "failed to remove rendition scratch file: {e}");
            }
        }
    }
}

impl ScratchFile {
    fn path(&self) -> &Path {
        &self.0
    }
}

/// Run one ladder sweep, if one is not already running.
///
/// Mirrors `server_migrate::auto_migrate_after_scan`: it loads its own key, does
/// nothing when there is no work, and never returns an error to its caller —
/// this is background maintenance, and a failure to produce a rendition must not
/// affect the scan that triggered it.
pub async fn generate_rungs_after_scan(
    pool: sqlx::SqlitePool,
    storage_root: PathBuf,
    jwt_secret: String,
) {
    // Encryption comes first. A ladder encode competing with the encryption
    // backlog delays photos becoming *viewable at all* in order to add a quality
    // option to a video the user can already play.
    if crate::photos::server_migrate::migration_active().await {
        tracing::debug!("[LADDER] encryption migration active — deferring rung sweep");
        return;
    }

    if SWEEP_RUNNING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        tracing::debug!("[LADDER] sweep already running — skipping this trigger");
        return;
    }

    let outcome = run_sweep(&pool, &storage_root, &jwt_secret).await;

    SWEEP_RUNNING.store(false, std::sync::atomic::Ordering::Release);

    if outcome.examined > 0 {
        tracing::info!(
            examined = outcome.examined,
            produced = outcome.produced,
            not_needed = outcome.not_needed,
            failed = outcome.failed,
            skipped = outcome.skipped,
            "[LADDER] rung sweep complete"
        );
    }
}

/// The sweep body, separated from the re-entrancy guard so tests can drive it
/// directly without racing a static.
pub async fn run_sweep(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    jwt_secret: &str,
) -> SweepOutcome {
    let mut outcome = SweepOutcome::default();

    let candidates = match rung_queue::find_rung_candidates(pool, SWEEP_CANDIDATE_LIMIT).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[LADDER] failed to select rung candidates: {e}");
            return outcome;
        }
    };
    if candidates.is_empty() {
        return outcome;
    }

    // Loaded once per sweep, not once per file: it is the same key every time and
    // unwrapping it is a KDF. `None` is legitimate — an unencrypted install has
    // no stored key, and its candidates are served from `file_path` instead.
    let key = match crate::crypto::load_wrapped_key(pool, jwt_secret).await {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("[LADDER] failed to load encryption key: {e}");
            None
        }
    };

    tracing::info!(
        candidates = candidates.len(),
        "[LADDER] starting video rendition sweep"
    );

    let started = std::time::Instant::now();

    for candidate in candidates {
        if started.elapsed() >= SWEEP_TIME_BUDGET {
            tracing::info!(
                elapsed_secs = started.elapsed().as_secs(),
                "[LADDER] sweep budget reached — remaining candidates deferred to the next sweep"
            );
            break;
        }

        outcome.examined += 1;
        let file_start = std::time::Instant::now();

        match generate_one(pool, storage_root, key.as_ref(), &candidate).await {
            Ok(Verdict::Produced { short_edge }) => {
                outcome.produced += 1;
                tracing::info!(
                    photo_id = %candidate.photo_id,
                    filename = %candidate.filename,
                    short_edge,
                    elapsed_secs = file_start.elapsed().as_secs(),
                    "[LADDER] produced video rendition"
                );
            }
            Ok(Verdict::NotNeeded) => {
                outcome.not_needed += 1;
                tracing::debug!(
                    photo_id = %candidate.photo_id,
                    filename = %candidate.filename,
                    "[LADDER] no rung owed"
                );
            }
            Ok(Verdict::Skipped(reason)) => {
                outcome.skipped += 1;
                // Info, not warn: every skip reason is environmental and
                // self-resolving (encryption pending, key absent, bytes not
                // where the row says). None needs an operator tonight, and the
                // encryption-backlog case would otherwise warn once per
                // candidate per sweep for as long as the backlog exists.
                tracing::info!(
                    photo_id = %candidate.photo_id,
                    filename = %candidate.filename,
                    "[LADDER] skipped: {reason}"
                );
            }
            Err(e) => {
                outcome.failed += 1;
                tracing::error!(
                    photo_id = %candidate.photo_id,
                    filename = %candidate.filename,
                    elapsed_secs = file_start.elapsed().as_secs(),
                    "[LADDER] rung generation failed: {e}"
                );
            }
        }
    }

    outcome
}

/// The outcome for one candidate that did not error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A rung was encoded, stored and recorded.
    Produced { short_edge: i64 },
    /// The probe found the source at or below the tier. Terminal.
    NotNeeded,
    /// Nothing could be attempted (no key, missing bytes). Not charged as an
    /// attempt, because no encode was tried and the cause is environmental.
    Skipped(String),
}

/// Decrypt → probe → plan → transcode → encrypt → record, for one video.
pub async fn generate_one(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    key: Option<&[u8; 32]>,
    candidate: &RungCandidate,
) -> Result<Verdict, String> {
    let scratch_dir = storage_root.join(".rendition_tmp");
    tokio::fs::create_dir_all(&scratch_dir)
        .await
        .map_err(|e| format!("create rendition scratch dir: {e}"))?;

    // ── Materialise plaintext ────────────────────────────────────────────────
    // `_decrypted` is bound (not `_`) so it lives to the end of this function:
    // `let _ = ScratchFile(..)` drops immediately and would delete the file
    // before ffmpeg ever opened it.
    let (source_path, _decrypted): (PathBuf, Option<ScratchFile>) =
        match candidate.encrypted_blob_id.as_deref() {
            Some(blob_id) => {
                let Some(key) = key else {
                    return Ok(Verdict::Skipped(
                        "photo is encrypted but no key is available".into(),
                    ));
                };
                let Some(enc_path) = blob_file_path(pool, storage_root, blob_id).await? else {
                    return Ok(Verdict::Skipped(format!("blob {blob_id} has no stored path")));
                };
                if !tokio::fs::try_exists(&enc_path).await.unwrap_or(false) {
                    return Ok(Verdict::Skipped(format!(
                        "blob {blob_id} is recorded but missing on disk"
                    )));
                }

                let dst = scratch_dir.join(format!("{}.src.mp4", candidate.photo_id));
                let scratch = ScratchFile(dst.clone());
                let key_copy = *key;
                let src = enc_path.clone();
                let out = dst.clone();
                tokio::task::spawn_blocking(move || {
                    chunked::decrypt_blob_file_to_file(&key_copy, &src, &out)
                })
                .await
                .map_err(|e| format!("decrypt task panicked: {e}"))?
                .map_err(|e| format!("decrypt source blob: {e}"))?;

                (dst, Some(scratch))
            }
            None => {
                // An encrypted install with a photo that has no blob yet: this
                // row is in the encryption backlog (2,494 of them live). Its
                // bytes are about to move, and a rendition recorded as a
                // `file_path` now would be one neither client can play —
                // both play from blobs. Wait for the migration.
                if key.is_some() {
                    return Ok(Verdict::Skipped(
                        "photo is still awaiting encryption; deferring its rung".into(),
                    ));
                }
                if candidate.file_path.is_empty() {
                    return Ok(Verdict::Skipped(
                        "photo has neither an encrypted blob nor a file path".into(),
                    ));
                }
                let path = storage_root.join(&candidate.file_path);
                if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
                    return Ok(Verdict::Skipped(format!(
                        "source file is missing: {}",
                        candidate.file_path
                    )));
                }
                (path, None)
            }
        };

    // ── Probe: the only geometry this function trusts ────────────────────────
    let info = probe::probe_video_stream(&source_path)
        .await
        .map_err(|e| format!("probe source: {e}"))?;

    if info.width <= 0 || info.height <= 0 {
        return Ok(Verdict::Skipped(
            "probe returned no usable dimensions".into(),
        ));
    }

    // Bounded by ffprobe's decode-health window, so this costs a few seconds
    // rather than a full decode of a 4K file. `None` on failure means "not
    // checked", which `source_rung_is_offerable` treats as the pre-#46 level of
    // confidence — never as evidence of health.
    let health = probe::probe_decode_health(&source_path).await.ok();
    let offerable = ladder::source_rung_is_offerable(&info, health.as_ref());

    let plan = ladder::plan_ladder(info.width, info.height, offerable);
    let Some(rung) = plan.iter().find(|r| !r.is_source).copied() else {
        // Either the source is at or below the tier, or the only rung planned
        // was the source itself. Both are terminal: no encode will ever be owed
        // for this file as it stands, and recording that is what stops the
        // deliberately-wide prefilter from re-selecting it forever.
        rung_queue::mark_rung_not_needed(pool, &candidate.photo_id, TIER_1080_SHORT_EDGE)
            .await
            .map_err(|e| format!("record not-needed verdict: {e}"))?;
        return Ok(Verdict::NotNeeded);
    };

    // ── Charge the attempt, then encode ──────────────────────────────────────
    let attempt = rung_queue::begin_attempt(pool, &candidate.photo_id, rung.short_edge)
        .await
        .map_err(|e| format!("charge rung attempt: {e}"))?;
    if attempt > MAX_RUNG_ATTEMPTS {
        // The candidate query filters these out, so reaching this means a sweep
        // raced another writer. Retiring here keeps the cap true regardless.
        return Ok(Verdict::Skipped(format!(
            "attempt {attempt} exceeds the cap of {MAX_RUNG_ATTEMPTS}"
        )));
    }

    let out_path = scratch_dir.join(format!(
        "{}.{}.mp4",
        candidate.photo_id, rung.short_edge
    ));
    let encoded = ScratchFile(out_path.clone());

    if let Err(e) =
        crate::conversion::transcode_to_rung(&source_path, &out_path, (rung.width, rung.height))
            .await
    {
        rung_queue::record_failure(pool, &candidate.photo_id, rung.short_edge, attempt, &e)
            .await
            .map_err(|e| format!("record rung failure: {e}"))?;
        return Err(e);
    }

    // ── Store ────────────────────────────────────────────────────────────────
    // A rendition is stored in whatever mode its parent photo is, which the
    // materialise step above has already narrowed to exactly two cases: an
    // encrypted photo with a key, or an unencrypted install. The mixed case
    // (encrypted install, photo still in the backlog) returned Skipped there.
    let stored = match (key, candidate.encrypted_blob_id.is_some()) {
        (Some(key), true) => {
            store_encrypted(pool, storage_root, key, candidate, &rung, encoded.path()).await?
        }
        _ => store_plaintext(storage_root, candidate, &rung, encoded.path()).await?,
    };

    if let Err(e) = upsert_rendition(pool, &stored).await {
        let msg = format!("record produced rendition: {e}");
        rung_queue::record_failure(pool, &candidate.photo_id, rung.short_edge, attempt, &msg)
            .await
            .map_err(|e| format!("record rung failure: {e}"))?;
        return Err(msg);
    }

    // The source rung, recorded only now. A picker with a 1080p entry and no
    // "original" is worse than no picker: the user can no longer reach the
    // quality they already had. It is recorded *after* the rung it accompanies
    // so that state can never exist, not even briefly.
    if offerable {
        let source_row = StoredRendition {
            photo_id: candidate.photo_id.clone(),
            short_edge: ladder::short_edge(info.width, info.height),
            width: info.width,
            height: info.height,
            is_source: 1,
            // Points at the bytes the PHOTO already owns — this row is a second
            // reference, never a copy. `037`'s orphan trigger excludes
            // `is_source` rows for exactly this reason.
            blob_id: candidate.encrypted_blob_id.clone(),
            file_path: candidate
                .encrypted_blob_id
                .is_none()
                .then(|| candidate.file_path.clone()),
            codec: Some(info.codec.clone()),
            bitrate: None,
            size_bytes: tokio::fs::metadata(&source_path)
                .await
                .map(|m| m.len() as i64)
                .unwrap_or(0),
        };
        if let Err(e) = upsert_rendition(pool, &source_row).await {
            // Not fatal: the 1080p rung exists and plays. Logged loudly because
            // the picker is now missing its top entry, which looks like the
            // ladder downgraded the user's video.
            tracing::error!(
                photo_id = %candidate.photo_id,
                "[LADDER] produced a rung but failed to record the source rendition: {e}"
            );
        }
    }

    Ok(Verdict::Produced {
        short_edge: rung.short_edge,
    })
}

/// Resolve a blob's on-disk path from its recorded `storage_path`.
///
/// Read from the row rather than recomputed with [`storage::blob_path`]: the
/// stored path is what the serve layer uses, and a rendition pass that derives
/// its own would silently diverge for any blob written before a path-scheme
/// change.
async fn blob_file_path(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    blob_id: &str,
) -> Result<Option<PathBuf>, String> {
    let rel: Option<String> = sqlx::query_scalar("SELECT storage_path FROM blobs WHERE id = ?")
        .bind(blob_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("look up blob {blob_id}: {e}"))?;

    Ok(rel.map(|r| storage_root.join(r)))
}

/// Encrypt the encoded rung into a new blob and register it.
///
/// Chunked (`SPCHNKB2`) unconditionally, never `crypto::encrypt`: a v1 encrypt
/// of a 1080p video holds roughly five times the file on the heap, which is the
/// OOM that migration `024` exists to prevent.
async fn store_encrypted(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    key: &[u8; 32],
    candidate: &RungCandidate,
    rung: &ladder::Rendition,
    encoded: &Path,
) -> Result<StoredRendition, String> {
    let size_bytes = tokio::fs::metadata(encoded)
        .await
        .map(|m| m.len())
        .map_err(|e| format!("stat encoded rung: {e}"))?;

    // The envelope a client decrypts before playing. `mime_type` is the only
    // load-bearing field — `decryptPhotoBlobToBlob` uses it as the Blob's type,
    // and a wrong value hands the player bytes it will not decode. The rest is
    // informational, and the dimensions are the RUNG's, not the source's.
    let meta = serde_json::json!({
        "v": chunked::FORMAT_V2,
        "filename": candidate.filename,
        "mime_type": "video/mp4",
        "media_type": "video",
        "width": rung.width,
        "height": rung.height,
        "chunk_size": chunked::CHUNK_SIZE,
        "data_len": size_bytes,
    });
    let meta_json =
        serde_json::to_vec(&meta).map_err(|e| format!("serialize rendition envelope: {e}"))?;

    let blob_id = Uuid::new_v4().to_string();
    let blob_abs = storage::blob_path(storage_root, &candidate.user_id, &blob_id);
    if let Some(parent) = blob_abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create rendition blob directory: {e}"))?;
    }
    let blob_rel = storage::relative_path(&candidate.user_id, &blob_id);

    let key_copy = *key;
    let src = encoded.to_path_buf();
    let dst = blob_abs.clone();
    let result = tokio::task::spawn_blocking(move || {
        chunked::encrypt_file_chunked(&key_copy, &src, &dst, &meta_json)
    })
    .await
    .map_err(|e| format!("rendition encrypt task panicked: {e}"))?
    .map_err(|e| format!("encrypt rendition: {e}"))?;

    // `content_hash` stays NULL, as thumbnail blobs already do. It is the dedup
    // key: a rendition sharing content with a photo's own blob would let the
    // dedup path link the two, and a later "reuse this existing blob" would hand
    // a photo its own downscale as the original.
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, \
         storage_path, content_hash, blob_format) \
         VALUES (?, ?, 'video', ?, ?, ?, ?, NULL, 1)",
    )
    .bind(&blob_id)
    .bind(&candidate.user_id)
    .bind(size_bytes as i64)
    .bind(hex::encode(result.blob_sha256))
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(&blob_rel)
    .execute(pool)
    .await
    .map_err(|e| {
        // The bytes are already on disk; without a row nothing will ever find
        // them again, so say so rather than letting it read as a lost encode.
        tracing::error!(
            blob_id = %blob_id,
            path = %blob_rel,
            "[LADDER] rendition encrypted but its blob row failed to insert — orphaned bytes"
        );
        format!("insert rendition blob row: {e}")
    })?;

    Ok(StoredRendition {
        photo_id: candidate.photo_id.clone(),
        short_edge: rung.short_edge,
        width: rung.width,
        height: rung.height,
        is_source: 0,
        blob_id: Some(blob_id),
        file_path: None,
        codec: Some("h264".into()),
        bitrate: None,
        size_bytes: size_bytes as i64,
    })
}

/// Move the encoded rung into place for an unencrypted install.
async fn store_plaintext(
    storage_root: &Path,
    candidate: &RungCandidate,
    rung: &ladder::Rendition,
    encoded: &Path,
) -> Result<StoredRendition, String> {
    let rel = format!(
        "renditions/{}/{}.{}.mp4",
        candidate.user_id, candidate.photo_id, rung.short_edge
    );
    let abs = storage_root.join(&rel);
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create rendition directory: {e}"))?;
    }

    // Copy rather than rename: the scratch file and the storage root can be on
    // different filesystems (a scratch dir under the root is not guaranteed to
    // share a mount with it), and `rename` fails across devices. The scratch
    // copy is removed by its `ScratchFile` guard either way.
    tokio::fs::copy(encoded, &abs)
        .await
        .map_err(|e| format!("place rendition file: {e}"))?;

    let size_bytes = tokio::fs::metadata(&abs)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    Ok(StoredRendition {
        photo_id: candidate.photo_id.clone(),
        short_edge: rung.short_edge,
        width: rung.width,
        height: rung.height,
        is_source: 0,
        blob_id: None,
        file_path: Some(rel),
        codec: Some("h264".into()),
        bitrate: None,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    //! These drive real FFmpeg through the real pass, because the two defects
    //! they guard are both invisible to a mocked encode: one is the *shape* of
    //! the bytes ffmpeg produced, and the other is which row the DB ends up
    //! holding after a probe that a mock would have to fake.
    //!
    //! Skipped when FFmpeg is unavailable so minimal CI images stay green —
    //! same convention as the #46 probe tests in `ingest.rs`.
    use super::*;
    use crate::transcode::renditions::list_renditions;
    use std::str::FromStr;

    async fn test_pool() -> sqlx::SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at) \
             VALUES ('u1', 'u1', 'x', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    /// A scratch storage root that cleans itself up.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "sp_ladder_{}_{tag}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Encode a real clip of an exact size. `None` when FFmpeg is unavailable.
    fn make_video(root: &Path, rel: &str, size: &str) -> Option<PathBuf> {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=duration=1:size={size}:rate=10"),
                "-c:v",
                "libx264",
                "-profile:v",
                "high",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok && std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
            Some(path)
        } else {
            None
        }
    }

    /// Register a video. `width`/`height` are what the DB *claims*, which the
    /// pass must never use for geometry.
    async fn insert_video(pool: &sqlx::SqlitePool, id: &str, rel: &str, width: i64, height: i64) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, created_at) \
             VALUES (?, 'u1', ?, ?, 'video/mp4', 'video', 0, ?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("{id}.mp4"))
        .bind(rel)
        .bind(width)
        .bind(height)
        .execute(pool)
        .await
        .unwrap();
    }

    /// **The transposition test.** `photos.width`/`height` are transposed
    /// against ffprobe for a large part of the live library — the census counted
    /// 126 × `3840x2160` where the DB holds 78 of them as `2160x3840`.
    ///
    /// A pass that plans from the stored pair computes
    /// `rung_dimensions(2560, 1440, 1080)` = `1920x1080` for this portrait
    /// source and hands `scale=1920:1080` to ffmpeg, which squashes a portrait
    /// frame into a landscape box. The output is not merely mis-recorded — it is
    /// visibly distorted, and nothing downstream can recover it.
    ///
    /// So this asserts the dimensions of the file ffmpeg actually produced, not
    /// the row we wrote about it. Those are the two things that can disagree.
    #[tokio::test]
    async fn a_transposed_row_does_not_squash_a_portrait_rendition() {
        let root = TempRoot::new("transposed");
        // A real portrait 1440x2560 source: short edge 1440 > 1080, so a rung
        // is owed and it must come out 1080x1920.
        let Some(_) = make_video(&root.0, "videos/portrait.mp4", "1440x2560") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        // Transposed exactly as the live DB holds it.
        insert_video(&pool, "p1", "videos/portrait.mp4", 2560, 1440).await;

        let candidates = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        assert_eq!(candidates.len(), 1, "the video must be selected as a candidate");

        let verdict = generate_one(&pool, &root.0, None, &candidates[0])
            .await
            .expect("rung generation must succeed");
        assert_eq!(verdict, Verdict::Produced { short_edge: 1080 });

        let rows = list_renditions(&pool, "p1").await.unwrap();
        let rung = rows
            .iter()
            .find(|r| !r.is_source())
            .expect("a 1080p rung must be recorded");
        assert_eq!(
            (rung.width, rung.height),
            (1080, 1920),
            "the rung must stay portrait; taking geometry from photos.width/height \
             would have produced 1920x1080"
        );

        // What ffmpeg actually wrote — the assertion the recorded row cannot make.
        let produced = root.0.join(rung.file_path.as_ref().unwrap());
        let info = probe::probe_video_stream(&produced).await.unwrap();
        assert_eq!(
            (info.width, info.height),
            (1080, 1920),
            "the encoded file itself must be portrait 1080x1920, not a squashed frame"
        );
    }

    /// The source rung must accompany the downscale, pointing at the bytes the
    /// photo already owns. A picker offering only 1080p on a 1440p video has
    /// silently taken away the quality the user had.
    #[tokio::test]
    async fn the_source_rung_is_offered_alongside_the_downscale() {
        let root = TempRoot::new("source_rung");
        let Some(_) = make_video(&root.0, "videos/big.mp4", "1440x2560") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        insert_video(&pool, "p1", "videos/big.mp4", 2560, 1440).await;

        let candidates = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        generate_one(&pool, &root.0, None, &candidates[0])
            .await
            .unwrap();

        let rows = list_renditions(&pool, "p1").await.unwrap();
        assert_eq!(rows.len(), 2, "picker must offer source + 1080p, got {rows:?}");
        // Highest first — the order a "default to highest" client reads.
        assert!(rows[0].is_source(), "the source rung must sort first");
        assert_eq!(rows[0].short_edge, 1440);
        assert_eq!(
            rows[0].file_path.as_deref(),
            Some("videos/big.mp4"),
            "the source rung must point at the photo's existing bytes, not a copy"
        );
        assert_eq!(rows[1].short_edge, 1080);
    }

    /// **The terminal-verdict test.** 58 live videos have no recorded geometry,
    /// so the prefilter selects them blind and lets the probe decide. Most need
    /// no rung — and without `not_needed` (037) that answer has nowhere to go:
    /// the row keeps both locators NULL, the candidate query reads it as "still
    /// owed", and the file is re-probed on every sweep until the attempt cap
    /// retires it with a warning claiming it will never get a picker.
    ///
    /// Verified RED by dropping the `r.not_needed = 1` arm from the candidate
    /// query: the second selection returns the photo again.
    #[tokio::test]
    async fn a_video_below_the_tier_leaves_the_candidate_set_permanently() {
        let root = TempRoot::new("not_needed");
        let Some(_) = make_video(&root.0, "videos/small.mp4", "320x240") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        // Geometry unknown — the live shape that forces a blind selection.
        insert_video(&pool, "p1", "videos/small.mp4", 0, 0).await;

        let candidates = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        assert_eq!(
            candidates.len(),
            1,
            "a row with no recorded geometry must be selected and resolved by a probe"
        );

        let verdict = generate_one(&pool, &root.0, None, &candidates[0])
            .await
            .unwrap();
        assert_eq!(verdict, Verdict::NotNeeded);

        let again = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        assert!(
            again.is_empty(),
            "a probed-and-not-owed video must never be selected again, got {again:?}"
        );

        // And the verdict must not masquerade as a playable rendition.
        assert!(
            list_renditions(&pool, "p1").await.unwrap().is_empty(),
            "a not-needed verdict must not surface in the picker"
        );

        // No attempt was spent: nothing was encoded, so a file later replaced
        // with a genuine 4K source starts with its full retry budget.
        let attempts: i64 = sqlx::query_scalar(
            "SELECT attempt_count FROM video_renditions WHERE photo_id = 'p1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(attempts, 0);
    }

    /// A photo still in the encryption backlog must be left alone. Both clients
    /// play from blobs, so a `file_path` rendition produced for a row that is
    /// about to be encrypted is bytes nothing can play — and the encode that
    /// made them would have to be repeated afterwards anyway.
    ///
    /// 2,494 live rows are in this state, so this is the normal case on a box
    /// mid-migration, not an edge one.
    #[tokio::test]
    async fn a_photo_awaiting_encryption_is_deferred_not_encoded() {
        let root = TempRoot::new("backlog");
        let Some(_) = make_video(&root.0, "videos/big.mp4", "1440x2560") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        // No encrypted_blob_id, but the install has a key → encrypted mode.
        insert_video(&pool, "p1", "videos/big.mp4", 2560, 1440).await;

        let candidates = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        let verdict = generate_one(&pool, &root.0, Some(&[7u8; 32]), &candidates[0])
            .await
            .unwrap();

        assert!(
            matches!(verdict, Verdict::Skipped(ref r) if r.contains("awaiting encryption")),
            "expected a deferral, got {verdict:?}"
        );
        // Nothing recorded: no claim, no attempt charged, no rendition. The row
        // must come back as a candidate once the migration gives it a blob.
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM video_renditions WHERE photo_id = 'p1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 0, "a deferral must not spend an attempt or claim a rung");
    }

    /// A produced rung removes its photo from the queue — the property that
    /// makes the candidate set self-limiting on success, and therefore the
    /// reason a sweep is safe to run on every autoscan.
    #[tokio::test]
    async fn a_produced_rung_is_not_re_encoded_on_the_next_sweep() {
        let root = TempRoot::new("idempotent");
        let Some(_) = make_video(&root.0, "videos/big.mp4", "1440x2560") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        insert_video(&pool, "p1", "videos/big.mp4", 2560, 1440).await;

        let first = run_sweep(&pool, &root.0, "test-secret").await;
        assert_eq!(first.produced, 1, "first sweep must produce the rung: {first:?}");

        let second = run_sweep(&pool, &root.0, "test-secret").await;
        assert_eq!(
            second,
            SweepOutcome::default(),
            "a second sweep must find no work at all, got {second:?}"
        );
    }
}
