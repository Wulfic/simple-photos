//! Background AI processor.
//!
//! Runs as a Tokio task (spawned from `tasks.rs`). Periodically scans for
//! unprocessed photos, runs face detection, object detection, face clustering,
//! and auto-tagging.
//!
//! Rate-limited by `photos_per_minute` config to avoid overwhelming the CPU.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt};
use sqlx::SqlitePool;
use tokio::time;
use tracing;

use crate::ai::animal;
use crate::ai::clustering;
use crate::ai::engine::AiEngine;
use crate::ai::face;
use crate::ai::object;
use crate::ai::tagging;
use crate::config::AiConfig;
use crate::state::AiHealth;

/// Decode-bomb guard (item #16). Stills whose *pixel* count exceeds this budget
/// are routed to the poster thumbnail instead of being full-decoded. A file that
/// is modest on disk can be enormous in pixels (e.g. a stitched ~300 MP
/// panorama, ~1.2 GB once expanded to an RGBA buffer) and OOM-kill the server —
/// the class of crash item #16 targets. Detection only ever sees a 640×640
/// letterbox, so nothing of value is lost. ~80 MP still admits full-res phone
/// photos (48–108 MP sensors bin down well below this).
const MAX_DECODE_PIXELS: u64 = 80_000_000;

/// Compile-time guard: the pixel budget must admit full-res phone photos
/// (~50 MP) yet reject the pathological stitched-panorama range (≥200 MP).
const _: () = {
    assert!(MAX_DECODE_PIXELS >= 50_000_000);
    assert!(MAX_DECODE_PIXELS < 200_000_000);
};

/// Hard allocation ceiling handed to the image decoder as a belt-and-suspenders
/// net for formats whose header dimensions `imagesize` can't read cheaply. A
/// decode that would allocate more than this fails gracefully (→ skip the photo)
/// instead of taking the process down.
const MAX_DECODE_ALLOC_BYTES: u64 = 1024 * 1024 * 1024; // 1 GiB

/// Minimum spacing between full re-clustering passes while an import is in
/// flight. Clustering is O(n²) over a user's *entire* face set; without this it
/// re-ran after every batch and pegged the CPU during large imports (item #16).
const CLUSTER_COOLDOWN: Duration = Duration::from_secs(120);

/// Spawn the background AI processor task.
///
/// Always spawns the processor regardless of `config.enabled`. The processor
/// checks per-user `ai_enabled` settings each cycle, using `config.enabled`
/// as the default for users who haven't explicitly toggled. This allows the
/// runtime toggle (`POST /api/ai/toggle`) to actually work.
pub fn spawn_ai_processor(
    pool: SqlitePool,
    config: AiConfig,
    storage_root: PathBuf,
    jwt_secret: String,
    active: Arc<AtomicBool>,
    health: Arc<AiHealth>,
) {
    let engine = AiEngine::new(&config);
    // Only warn about missing models when the operator has enabled AI globally.
    // When ai.enabled=false, models are intentionally not loaded yet; the
    // processor will call ensure_models_loaded() when photos actually need
    // processing (i.e. when a user has individually enabled AI).
    if config.enabled && !engine.has_any_capability() {
        if config.allow_heuristic_fallback {
            tracing::warn!(
                "AI processor: no ONNX models found in '{}'. \
                 Heuristic fallback ENABLED — results will be low-quality.",
                config.model_dir
            );
        } else {
            tracing::error!(
                "AI processor: no ONNX models found in '{}' and \
                 allow_heuristic_fallback=false. AI features are running in \
                 DEGRADED mode — face / object detection will produce no \
                 results. Run scripts/fetch_ai_models.sh or set \
                 ai.allow_heuristic_fallback = true in config.toml.",
                config.model_dir
            );
        }
    }

    tokio::spawn(async move {
        // Wait for server startup to complete before starting AI processing
        time::sleep(Duration::from_secs(30)).await;

        let photos_per_minute = config.photos_per_minute.max(1);
        // Guard the divide: photos_per_minute > 60 would floor to 0 and spin the
        // loop hot (a self-inflicted stability bug for item #16).
        let interval = Duration::from_secs((60 / photos_per_minute as u64).max(1));
        let batch_size = config.batch_size.max(1);

        tracing::info!(
            "AI processor started: {} photos/min, batch_size={}, provider={}, config_default={}",
            photos_per_minute,
            batch_size,
            engine.provider(),
            if config.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );

        // Clustering throttle + circuit-breaker state (item #16).
        //   last_cluster     — when the last full re-clustering pass ran.
        //   clustering_dirty — new detections landed but clustering was deferred
        //                      by the cooldown; flush once the queue drains.
        let mut last_cluster: Option<Instant> = None;
        let mut clustering_dirty = false;

        loop {
            // Defer AI work while the import → encrypt → convert pipeline is
            // still in flight, so we don't contend with it for CPU and SQLite's
            // single writer lock — the "jumping between encrypt, AI and geo one
            // photo at a time" stall. Re-check at the normal cadence until it
            // clears.
            if crate::ingest::ingest_pipeline_busy(&pool).await {
                tracing::debug!("AI processor: ingest pipeline busy, deferring batch");
                time::sleep(interval).await;
                continue;
            }

            // Run a batch with the activity flag held high so the web client
            // can spin its profile-avatar indicator while AI work is in progress.
            active.store(true, Ordering::Relaxed);
            let batch_start = Instant::now();
            let result = process_batch(&pool, &engine, &config, &storage_root, &jwt_secret).await;
            active.store(false, Ordering::Relaxed);

            let mut backoff = None;
            match result {
                Ok(processed) => {
                    health.record_success(processed, batch_start.elapsed().as_millis() as u64);

                    // Throttle clustering so it can't storm the CPU mid-import.
                    if processed > 0 {
                        clustering_dirty = true;
                        let cooled = last_cluster.is_none_or(|t| t.elapsed() >= CLUSTER_COOLDOWN);
                        // A short batch means the queue is nearly drained, so the
                        // user should see face groups now; a *full* batch means an
                        // import is still streaming, so wait out the cooldown.
                        let drained = processed < batch_size;
                        if cooled || drained {
                            run_clustering_pass(&pool, &config).await;
                            last_cluster = Some(Instant::now());
                            clustering_dirty = false;
                        }
                    } else if clustering_dirty {
                        // Queue fully drained after a throttled import — flush the
                        // deferred clustering exactly once so nothing is stranded.
                        run_clustering_pass(&pool, &config).await;
                        last_cluster = Some(Instant::now());
                        clustering_dirty = false;
                    }
                }
                Err(e) => {
                    let n = health.record_error();
                    // Exponential backoff so a systemic failure (storage gone,
                    // corrupt model, DB locked) can't hot-loop and starve the
                    // box — the circuit breaker item #16 calls for.
                    let mult = 1u64 << u32::min(n, 6); // 2,4,…,64×
                    let secs = (interval.as_secs().max(1) * mult).min(300);
                    backoff = Some(Duration::from_secs(secs));
                    tracing::error!(
                        consecutive_errors = n,
                        backoff_secs = secs,
                        "AI processor batch error: {}",
                        e
                    );
                }
            }

            time::sleep(backoff.unwrap_or(interval)).await;
        }
    });
}

/// Process a batch of unprocessed photos. Returns the number of photos handled
/// so the caller can pace itself and decide when to run a clustering pass — a
/// full batch signals an import is still streaming (defer clustering), a short
/// or empty batch signals the queue is drained (cluster now). Clustering itself
/// is deliberately *not* run here; the caller owns it via [`run_clustering_pass`]
/// so it can be throttled against re-cluster storms (item #16).
async fn process_batch(
    pool: &SqlitePool,
    engine: &AiEngine,
    config: &AiConfig,
    storage_root: &PathBuf,
    jwt_secret: &str,
) -> anyhow::Result<usize> {
    // Find unprocessed photos only for users who have AI enabled.
    // - Users who explicitly set ai_enabled = 'true' → included
    // - Users who explicitly set ai_enabled = 'false' → excluded
    // - Users with no setting → included only if config.enabled is true
    let config_default_enabled = if config.enabled { 1i32 } else { 0i32 };

    let unprocessed: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT p.id, p.user_id, p.filename FROM photos p \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM ai_processed_photos ap \
             WHERE ap.photo_id = p.id AND ap.user_id = p.user_id \
         ) \
         AND ( \
             (p.file_path IS NOT NULL AND p.file_path != '') \
             OR p.encrypted_blob_id IS NOT NULL \
         ) \
         AND ( \
             EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'ai_enabled' AND us.value = 'true') \
             OR ( \
                 ?2 = 1 AND NOT EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'ai_enabled') \
             ) \
         ) \
         ORDER BY p.created_at DESC \
         LIMIT ?1"
    )
    .bind(config.batch_size as i64)
    .bind(config_default_enabled)
    .fetch_all(pool)
    .await?;

    if unprocessed.is_empty() {
        return Ok(0);
    }

    // Lazily initialise ONNX model sessions on the first batch that actually
    // needs processing.  When ai.enabled=false at startup, models are not
    // loaded into memory until this point — only when a user has individually
    // enabled AI and photos are queued.  The call is idempotent (OnceLock).
    engine.ensure_models_loaded();

    tracing::info!(
        "AI processor: batch of {} photo(s) queued for recognition [provider={}]",
        unprocessed.len(),
        engine.provider()
    );

    let batch_start = Instant::now();
    let total_faces = AtomicUsize::new(0);
    let total_objects = AtomicUsize::new(0);

    // Process the batch concurrently. Each ONNX model is a *pool* of sessions
    // (see `session::build_session_pool`), so `concurrency` inferences run in
    // parallel — one per pooled session — while their decrypt / decode / DB
    // steps overlap. The pool size already divides the core budget across
    // sessions, so aggregate CPU use matches the old single-session path; this
    // just fills the cores that used to sit idle between serial inferences.
    // A pool of 1 (single-core host / `SIMPLE_PHOTOS_AI_JOBS=1`) is exactly the
    // old serial loop.
    let concurrency = crate::ai::session::ai_pool_plan().0.max(1);
    tracing::debug!(concurrency, "AI processor: batch concurrency");

    stream::iter(unprocessed.iter())
        .for_each_concurrent(concurrency, |(photo_id, user_id, filename)| {
            let total_faces = &total_faces;
            let total_objects = &total_objects;
            async move {
                let photo_start = Instant::now();
                match process_single_photo(
                    pool,
                    engine,
                    config,
                    storage_root,
                    jwt_secret,
                    photo_id,
                    user_id,
                    filename,
                )
                .await
                {
                    Ok((nf, no)) => {
                        total_faces.fetch_add(nf, Ordering::Relaxed);
                        total_objects.fetch_add(no, Ordering::Relaxed);
                        tracing::info!(
                            photo_id = %photo_id,
                            filename = %filename,
                            faces = nf,
                            objects = no,
                            elapsed_ms = photo_start.elapsed().as_millis(),
                            "AI processor: photo processed"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            photo_id = %photo_id,
                            filename = %filename,
                            error = %e,
                            "AI processor: failed to process photo — marking done to skip retry"
                        );
                        // Mark as processed anyway to avoid infinite retry loops.
                        if let Err(me) = mark_processed(pool, photo_id, user_id).await {
                            tracing::warn!(
                                photo_id = %photo_id,
                                error = %me,
                                "AI processor: failed to mark errored photo processed"
                            );
                        }
                    }
                }
            }
        })
        .await;

    let total_faces = total_faces.into_inner();
    let total_objects = total_objects.into_inner();

    tracing::info!(
        photos = unprocessed.len(),
        faces_found = total_faces,
        objects_found = total_objects,
        elapsed_ms = batch_start.elapsed().as_millis(),
        "AI processor: batch complete"
    );

    Ok(unprocessed.len())
}

/// Run one throttled clustering pass for every user that has new unclustered
/// face or pet detections. Split out of `process_batch` so the processor loop
/// can gate how often it runs (a full O(n²) re-cluster of a user's whole face
/// set after every 8-photo batch pegged the CPU during large imports — item
/// #16). Per-user failures are logged and swallowed so one bad user can't stall
/// clustering for everyone.
async fn run_clustering_pass(pool: &SqlitePool, config: &AiConfig) {
    // Faces.
    match sqlx::query_as::<_, (String,)>(
        "SELECT DISTINCT user_id FROM face_detections WHERE cluster_id IS NULL",
    )
    .fetch_all(pool)
    .await
    {
        Ok(users) => {
            for (user_id,) in &users {
                if let Err(e) =
                    run_clustering(pool, user_id, config.face_similarity_threshold).await
                {
                    tracing::warn!(
                        "AI processor: clustering failed for user {}: {}",
                        user_id,
                        e
                    );
                }
            }
        }
        Err(e) => tracing::warn!("AI processor: could not list users for clustering: {}", e),
    }

    // Pets.
    match sqlx::query_as::<_, (String,)>(
        "SELECT DISTINCT user_id FROM pet_detections WHERE cluster_id IS NULL",
    )
    .fetch_all(pool)
    .await
    {
        Ok(users) => {
            for (user_id,) in &users {
                if let Err(e) =
                    run_pet_clustering(pool, user_id, config.pet_similarity_threshold).await
                {
                    tracing::warn!(
                        "AI processor: pet clustering failed for user {}: {}",
                        user_id,
                        e
                    );
                }
            }
        }
        Err(e) => tracing::warn!(
            "AI processor: could not list users for pet clustering: {}",
            e
        ),
    }
}

/// Process a single photo: detect faces and objects, save to DB.
/// Returns (face_count, object_count) on success.
async fn process_single_photo(
    pool: &SqlitePool,
    _engine: &AiEngine,
    config: &AiConfig,
    storage_root: &PathBuf,
    jwt_secret: &str,
    photo_id: &str,
    user_id: &str,
    filename: &str,
) -> anyhow::Result<(usize, usize)> {
    // Load the photo file (plain or encrypted)
    let row: Option<(
        String,
        Option<String>,
        String,
        i64,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT file_path, encrypted_blob_id, media_type, size_bytes, thumb_path, \
             encrypted_thumb_blob_id FROM photos WHERE id = ?1 AND user_id = ?2",
    )
    .bind(photo_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let (file_path, encrypted_blob_id, media_type, size_bytes, thumb_path, encrypted_thumb_blob_id) =
        match row {
            Some(r) => r,
            None => {
                tracing::debug!(photo_id = %photo_id, "AI: photo not found in DB, skipping");
                mark_processed(pool, photo_id, user_id).await?;
                return Ok((0, 0));
            }
        };

    // ── Choose the decode source ──────────────────────────────────────────
    // Videos must NEVER be read in full here: a single video blob can be
    // several GB, and decrypting it whole into RAM just to hand it to
    // `image::load_from_memory` (which can't decode video anyway) OOM-kills the
    // server on large libraries — the root cause of issues #5 and #13. Run
    // detection on the already-generated poster thumbnail instead. Oversized
    // still images take the same path as a safety net against pathological
    // multi-hundred-MP files.
    const MAX_FULL_DECODE_BYTES: i64 = 128 * 1024 * 1024; // 128 MiB
    let is_video = media_type.eq_ignore_ascii_case("video");
    let oversized = size_bytes > MAX_FULL_DECODE_BYTES;

    // item #8: for videos, first try the frame ~5 s in (sliding ±2 s) that
    // actually contains a face — decoded directly, far better than the poster
    // thumbnail. Any failure leaves this None and we fall back to the thumbnail
    // path below, so the pipeline never destabilises on video handling (#16).
    let video_selection: Option<(image::DynamicImage, Vec<crate::ai::models::FaceDetection>)> =
        if is_video {
            select_video_face_frame(
                pool,
                storage_root,
                jwt_secret,
                &file_path,
                encrypted_blob_id.as_deref(),
                user_id,
                config.face_confidence,
                config.allow_heuristic_fallback,
            )
            .await
        } else {
            None
        };

    // `img` + optionally precomputed face detections come from either the
    // video-frame path (item #8) or the existing thumbnail / full-image decode.
    let (img, precomputed_faces): (
        image::DynamicImage,
        Option<Vec<crate::ai::models::FaceDetection>>,
    ) = if let Some((frame_img, faces)) = video_selection {
        tracing::debug!(
            photo_id = %photo_id, faces = faces.len(),
            "AI: using ~5s video frame for detection (item #8)"
        );
        (frame_img, Some(faces))
    } else {
        // `full_still` = a still image we intend to full-decode (not already
        // routed to a thumbnail for being a video or byte-oversized). Only these
        // need the pixel-count decode-bomb guard below.
        let full_still = !(is_video || oversized);
        // Read the image bytes: thumbnail for video/oversized, else full media.
        let mut image_bytes = if is_video || oversized {
            match load_thumbnail_bytes(
                pool,
                storage_root,
                jwt_secret,
                thumb_path.as_deref(),
                encrypted_thumb_blob_id.as_deref(),
                user_id,
            )
            .await
            {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    tracing::debug!(
                        photo_id = %photo_id, is_video, oversized,
                        "AI: no thumbnail for video/oversized media — skipping (refusing to read full blob)"
                    );
                    mark_processed(pool, photo_id, user_id).await?;
                    return Ok((0, 0));
                }
                Err(e) => {
                    tracing::debug!(photo_id = %photo_id, error = %e, "AI: thumbnail load failed, skipping");
                    mark_processed(pool, photo_id, user_id).await?;
                    return Ok((0, 0));
                }
            }
        } else if !file_path.is_empty() {
            let abs_path = storage_root.join(&file_path);
            match tokio::fs::read(&abs_path).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::debug!(file_path = %file_path, abs_path = ?abs_path, error = %e, "AI: cannot read photo file, skipping");
                    mark_processed(pool, photo_id, user_id).await?;
                    return Ok((0, 0));
                }
            }
        } else if let Some(enc_blob_id) = encrypted_blob_id.as_ref() {
            match load_encrypted_photo_bytes(pool, storage_root, jwt_secret, enc_blob_id, user_id)
                .await
            {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::debug!(
                        photo_id = %photo_id,
                        encrypted_blob_id = %enc_blob_id,
                        error = %e,
                        "AI: cannot read encrypted photo, skipping"
                    );
                    mark_processed(pool, photo_id, user_id).await?;
                    return Ok((0, 0));
                }
            }
        } else {
            tracing::debug!(photo_id = %photo_id, "AI: photo has neither file_path nor encrypted_blob_id, skipping");
            mark_processed(pool, photo_id, user_id).await?;
            return Ok((0, 0));
        };

        tracing::debug!(
            photo_id = %photo_id,
            filename = %filename,
            size_bytes = image_bytes.len(),
            "AI: starting recognition"
        );

        // Skip very small files (probably thumbnails)
        if image_bytes.len() < 1000 {
            tracing::debug!(photo_id = %photo_id, size_bytes = image_bytes.len(), "AI: file too small, skipping");
            mark_processed(pool, photo_id, user_id).await?;
            return Ok((0, 0));
        }

        // ── Decode-bomb guard (item #16) ──────────────────────────────────
        // A still can be small on disk yet enormous in pixels (stitched
        // panorama, upscaled export). Full-decoding it to RGBA can allocate
        // >1 GB and OOM-kill the server. Read the header dimensions cheaply
        // (no full decode) and, if over budget, fall back to the poster
        // thumbnail; if there is none, skip rather than risk the decode.
        if full_still {
            if let Ok(sz) = imagesize::blob_size(&image_bytes) {
                let pixels = sz.width as u64 * sz.height as u64;
                if pixels > MAX_DECODE_PIXELS {
                    match load_thumbnail_bytes(
                        pool,
                        storage_root,
                        jwt_secret,
                        thumb_path.as_deref(),
                        encrypted_thumb_blob_id.as_deref(),
                        user_id,
                    )
                    .await
                    {
                        Ok(Some(t)) => {
                            tracing::debug!(
                                photo_id = %photo_id, pixels,
                                "AI: still exceeds pixel budget — using thumbnail (decode-bomb guard)"
                            );
                            image_bytes = t;
                        }
                        _ => {
                            tracing::debug!(
                                photo_id = %photo_id, pixels,
                                "AI: still exceeds pixel budget and no thumbnail — skipping"
                            );
                            mark_processed(pool, photo_id, user_id).await?;
                            return Ok((0, 0));
                        }
                    }
                }
            }
        }

        // Decode image once for both pipelines, with a hard allocation ceiling
        // as a safety net for files whose header dimensions couldn't be read.
        let img = match decode_image_bounded(&image_bytes) {
            Ok(img) => img,
            Err(e) => {
                tracing::debug!(filename = %filename, error = %e, "AI: cannot decode image, skipping");
                mark_processed(pool, photo_id, user_id).await?;
                return Ok((0, 0));
            }
        };
        // Apply EXIF orientation so face/object detection sees the image upright.
        // `image::load_from_memory` ignores EXIF; SCRFD only detects upright faces,
        // so rotated selfies would otherwise be missed entirely.
        let img = crate::photos::thumbnail::apply_exif_orientation_from_bytes(&image_bytes, img);
        (img, None)
    };
    tracing::debug!(
        photo_id = %photo_id,
        width = img.width(),
        height = img.height(),
        "AI: image decoded, running detection pipelines"
    );

    // Clear any previous AI tags before re-processing
    tagging::clear_ai_tags(pool, user_id, photo_id).await?;

    // Face detection. For videos we may already have detections from the
    // ~5 s frame selection (item #8) — reuse them instead of re-running.
    let face_start = Instant::now();
    let face_detections = match precomputed_faces {
        Some(faces) => faces,
        None => face::detect_faces_from_image(
            &img,
            config.face_confidence,
            config.allow_heuristic_fallback,
        )?,
    };
    tracing::debug!(
        photo_id = %photo_id,
        filename = %filename,
        faces = face_detections.len(),
        confidence_threshold = config.face_confidence,
        elapsed_ms = face_start.elapsed().as_millis(),
        "AI: face detection complete"
    );
    for (i, det) in face_detections.iter().enumerate() {
        tracing::debug!(
            photo_id = %photo_id,
            face_index = i,
            confidence = format!("{:.3}", det.confidence),
            bbox = format!("x={:.3} y={:.3} w={:.3} h={:.3}", det.bbox.x, det.bbox.y, det.bbox.w, det.bbox.h),
            "AI: face detected"
        );
    }

    for det in &face_detections {
        // Use embedding from detection (SCRFD+ArcFace populates this),
        // fall back to extract_face_embedding for legacy paths.
        let embedding = if !det.embedding.is_empty() {
            det.embedding.clone()
        } else {
            face::extract_face_embedding(&img, &det.bbox)
        };
        let embedding_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

        sqlx::query(
            "INSERT INTO face_detections \
             (photo_id, user_id, bbox_x, bbox_y, bbox_w, bbox_h, confidence, embedding) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(photo_id)
        .bind(user_id)
        .bind(det.bbox.x)
        .bind(det.bbox.y)
        .bind(det.bbox.w)
        .bind(det.bbox.h)
        .bind(det.confidence)
        .bind(&embedding_bytes)
        .execute(pool)
        .await?;
    }

    // Object detection
    let obj_start = Instant::now();
    let quality = config.detection_quality();
    let obj_detections = object::detect_objects_with_quality(
        &img,
        config.object_confidence,
        quality,
        config.allow_heuristic_fallback,
    )?;
    tracing::debug!(
        photo_id = %photo_id,
        filename = %filename,
        objects = obj_detections.len(),
        confidence_threshold = config.object_confidence,
        elapsed_ms = obj_start.elapsed().as_millis(),
        "AI: object detection complete"
    );
    for det in &obj_detections {
        tracing::debug!(
            photo_id = %photo_id,
            class = %det.class_name,
            confidence = format!("{:.3}", det.confidence),
            "AI: object detected"
        );
    }

    for det in &obj_detections {
        sqlx::query(
            "INSERT INTO object_detections \
             (photo_id, user_id, class_name, confidence, bbox_x, bbox_y, bbox_w, bbox_h) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(photo_id)
        .bind(user_id)
        .bind(&det.class_name)
        .bind(det.confidence)
        .bind(det.bbox.x)
        .bind(det.bbox.y)
        .bind(det.bbox.w)
        .bind(det.bbox.h)
        .execute(pool)
        .await?;

        // Apply object tag immediately
        tagging::apply_object_tag(pool, user_id, photo_id, &det.class_name).await?;
    }

    // ── Pet detection ────────────────────────────────────────────────────
    // For each pet-class object detection, extract an embedding and store
    // a pet_detections row for later clustering.  We deduplicate by species
    // so a photo with two "dog" detections produces one pet row.
    let mut seen_pet_species: std::collections::HashSet<String> = std::collections::HashSet::new();
    for det in &obj_detections {
        if let Some(species) = animal::map_to_species(&det.class_name) {
            if !seen_pet_species.insert(species.to_string()) {
                continue; // already wrote a row for this species in this photo
            }
            let pet_start = Instant::now();
            match animal::extract_pet_embedding(&img, Some(&det.bbox)) {
                Some(embedding) => {
                    let emb_bytes: Vec<u8> =
                        embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
                    // The originating object detection's box is stored with the
                    // pet row so Pet tiles can crop to the animal the way People
                    // tiles crop to the face (#48). Migration 039 recovers it for
                    // rows written before this line existed by joining back to
                    // `object_detections`; that recovery only works while the pet
                    // row keeps `det.confidence` verbatim, as it does here.
                    sqlx::query(
                        "INSERT INTO pet_detections \
                         (photo_id, user_id, species, confidence, embedding, \
                          bbox_x, bbox_y, bbox_w, bbox_h) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    )
                    .bind(photo_id)
                    .bind(user_id)
                    .bind(species)
                    .bind(det.confidence)
                    .bind(&emb_bytes)
                    .bind(det.bbox.x)
                    .bind(det.bbox.y)
                    .bind(det.bbox.w)
                    .bind(det.bbox.h)
                    .execute(pool)
                    .await?;

                    // Apply a generic pet species tag immediately
                    tagging::apply_pet_tag(pool, user_id, photo_id, None, species).await?;

                    tracing::debug!(
                        photo_id = %photo_id,
                        species = %species,
                        elapsed_ms = pet_start.elapsed().as_millis(),
                        "AI: pet embedding stored"
                    );
                }
                None => {
                    tracing::debug!(
                        photo_id = %photo_id,
                        species = %species,
                        "AI: no pet embedding model available — skipping pet re-ID"
                    );
                }
            }
        }
    }

    mark_processed(pool, photo_id, user_id).await?;

    Ok((face_detections.len(), obj_detections.len()))
}

/// Mark a photo as AI-processed.
async fn mark_processed(pool: &SqlitePool, photo_id: &str, user_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO ai_processed_photos (photo_id, user_id, processed_at) \
         VALUES (?1, ?2, datetime('now'))",
    )
    .bind(photo_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Decode image bytes with a hard allocation ceiling so a pathological or
/// malformed file can't OOM the server (item #16). This is the belt-and-braces
/// net behind the pixel-count pre-check in `process_single_photo`: it catches
/// files whose header dimensions `imagesize` couldn't read. A decode that would
/// allocate more than [`MAX_DECODE_ALLOC_BYTES`] returns an error instead of
/// aborting the process. EXIF orientation is applied by the caller afterwards,
/// exactly as with the previous `image::load_from_memory` path.
fn decode_image_bounded(bytes: &[u8]) -> anyhow::Result<image::DynamicImage> {
    use std::io::Cursor;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| anyhow::anyhow!("guess format: {e}"))?;
    let mut limits = image::Limits::no_limits();
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    reader.limits(limits);
    reader.decode().map_err(|e| anyhow::anyhow!("decode: {e}"))
}

/// Load the decrypted plaintext image bytes for a server-side-encrypted photo.
///
/// The blob on disk is an AEAD-encrypted JSON envelope of the form
/// `{"data": "<base64 raw image bytes>"}`. We load the wrapped key, decrypt,
/// parse the envelope, and return the raw image bytes.
async fn load_encrypted_photo_bytes(
    pool: &SqlitePool,
    storage_root: &PathBuf,
    jwt_secret: &str,
    encrypted_blob_id: &str,
    user_id: &str,
) -> anyhow::Result<Vec<u8>> {
    let key = crate::crypto::load_wrapped_key(pool, jwt_secret)
        .await
        .map_err(|e| anyhow::anyhow!("load wrapped key: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("no encryption key configured"))?;

    let (blob_storage_path,): (String,) =
        sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
            .bind(encrypted_blob_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("encrypted blob row not found: {encrypted_blob_id}"))?;

    let enc_data = crate::blobs::storage::read_blob(storage_root.as_path(), &blob_storage_path)
        .await
        .map_err(|e| anyhow::anyhow!("read encrypted blob: {e}"))?;

    // Format-aware: handles both the legacy monolithic envelope and the v2
    // chunked container — see blobs/chunked.rs.
    let raw_bytes = tokio::task::spawn_blocking(move || {
        crate::blobs::chunked::decrypt_photo_blob(&key, &enc_data)
    })
    .await
    .map_err(|e| anyhow::anyhow!("decrypt panicked: {e}"))?
    .map_err(|e| anyhow::anyhow!("decrypt failed: {e}"))?;

    Ok(raw_bytes)
}

/// Load image bytes for AI detection from a photo's *thumbnail* rather than the
/// full-resolution media. Used for videos and oversized stills so we never pull
/// a multi-GB blob into memory (issues #5 / #13). Returns `Ok(None)` when no
/// thumbnail is available, letting the caller simply skip the photo.
async fn load_thumbnail_bytes(
    pool: &SqlitePool,
    storage_root: &PathBuf,
    jwt_secret: &str,
    thumb_path: Option<&str>,
    encrypted_thumb_blob_id: Option<&str>,
    user_id: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    // Prefer a plaintext thumbnail on disk when present.
    if let Some(tp) = thumb_path {
        if !tp.is_empty() {
            let abs = storage_root.join(tp);
            if let Ok(bytes) = tokio::fs::read(&abs).await {
                if bytes.len() >= 100 {
                    return Ok(Some(bytes));
                }
            }
        }
    }
    // Fall back to the encrypted thumbnail blob (the server's normal mode).
    if let Some(blob_id) = encrypted_thumb_blob_id {
        if !blob_id.is_empty() {
            let bytes =
                load_encrypted_photo_bytes(pool, storage_root, jwt_secret, blob_id, user_id)
                    .await?;
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

/// Pick the best face-detection frame from a video (item #8): try ~5 s in, then
/// slide to 3 s and 7 s. Returns the first frame with a detected face (and its
/// detections); if none has a face, returns the primary frame with empty
/// detections so object detection still runs. `None` only when no frame could be
/// produced at all, letting the caller fall back to the poster thumbnail.
#[allow(clippy::too_many_arguments)]
async fn select_video_face_frame(
    pool: &SqlitePool,
    storage_root: &PathBuf,
    jwt_secret: &str,
    file_path: &str,
    encrypted_blob_id: Option<&str>,
    user_id: &str,
    face_confidence: f32,
    allow_heuristic_fallback: bool,
) -> Option<(image::DynamicImage, Vec<crate::ai::models::FaceDetection>)> {
    let src = super::video_frame::open(
        pool,
        storage_root,
        jwt_secret,
        file_path,
        encrypted_blob_id,
        user_id,
    )
    .await?;

    let mut fallback: Option<image::DynamicImage> = None;
    for secs in src.candidate_secs() {
        let Some(jpg) = src.frame_at(secs).await else {
            continue;
        };
        let frame_img = match image::load_from_memory(&jpg) {
            Ok(img) => img,
            Err(_) => continue,
        };
        match face::detect_faces_from_image(&frame_img, face_confidence, allow_heuristic_fallback) {
            Ok(faces) if !faces.is_empty() => return Some((frame_img, faces)),
            Ok(_) => {
                if fallback.is_none() {
                    fallback = Some(frame_img);
                }
            }
            Err(_) => {}
        }
    }
    // No frame had a face — hand back the primary frame with empty detections so
    // object detection still runs and the video is recorded as "no faces".
    fallback.map(|img| (img, Vec::new()))
}

/// Run face clustering for a user.
///
/// Loads all unclustered face detections, runs agglomerative clustering,
/// and assigns them to existing or new clusters.
async fn run_clustering(
    pool: &SqlitePool,
    user_id: &str,
    similarity_threshold: f32,
) -> anyhow::Result<()> {
    // Load all face detections with embeddings for this user
    let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT id, embedding FROM face_detections WHERE user_id = ?1 AND embedding IS NOT NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    tracing::info!(
        user_id = %user_id,
        embeddings = rows.len(),
        threshold = similarity_threshold,
        "AI clustering: running agglomerative clustering"
    );

    // Convert embeddings from bytes to f32 vectors
    let faces: Vec<(i64, Vec<f32>)> = rows
        .into_iter()
        .filter_map(|(id, bytes)| {
            if bytes.len() % 4 != 0 {
                return None;
            }
            let embedding: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            Some((id, embedding))
        })
        .collect();

    // Run clustering
    let assignments = clustering::cluster_faces(&faces, similarity_threshold);

    let cluster_start = Instant::now();
    // Map cluster assignments to database cluster IDs.
    // First, get existing clusters for this user.
    let existing_clusters: Vec<(i64,)> =
        sqlx::query_as("SELECT id FROM face_clusters WHERE user_id = ?1 ORDER BY id")
            .bind(user_id)
            .fetch_all(pool)
            .await?;

    // Find unique cluster IDs from the clustering output
    let mut unique_clusters: Vec<i64> = assignments.iter().map(|(_, c)| *c).collect();
    unique_clusters.sort();
    unique_clusters.dedup();

    // Create new clusters in the database for clusters that don't have a mapping
    let existing_count = existing_clusters.len();
    let mut cluster_id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut new_clusters_created = 0usize;
    let mut clusters_updated = 0usize;

    for cluster_idx in &unique_clusters {
        // Count faces in this cluster
        let count = assignments.iter().filter(|(_, c)| c == cluster_idx).count();

        // Check if we can match this to an existing cluster by finding
        // the face detection that's already assigned to a cluster
        let mut matched_db_cluster = None;
        for (face_id, c) in &assignments {
            if c != cluster_idx {
                continue;
            }
            let existing: Option<(Option<i64>,)> =
                sqlx::query_as("SELECT cluster_id FROM face_detections WHERE id = ?1")
                    .bind(face_id)
                    .fetch_optional(pool)
                    .await?;

            if let Some((Some(cid),)) = existing {
                matched_db_cluster = Some(cid);
                break;
            }
        }

        let db_cluster_id = match matched_db_cluster {
            Some(cid) => {
                // Update photo count
                sqlx::query(
                    "UPDATE face_clusters SET photo_count = ?1, updated_at = datetime('now') WHERE id = ?2"
                )
                .bind(count as i64)
                .bind(cid)
                .execute(pool)
                .await?;
                clusters_updated += 1;
                cid
            }
            None => {
                // Create new cluster
                let result = sqlx::query(
                    "INSERT INTO face_clusters (user_id, photo_count, created_at, updated_at) \
                     VALUES (?1, ?2, datetime('now'), datetime('now'))",
                )
                .bind(user_id)
                .bind(count as i64)
                .execute(pool)
                .await?;
                new_clusters_created += 1;
                result.last_insert_rowid()
            }
        };

        cluster_id_map.insert(*cluster_idx, db_cluster_id);
    }

    // Update face detections with cluster assignments
    for (face_id, cluster_idx) in &assignments {
        if let Some(db_cluster_id) = cluster_id_map.get(cluster_idx) {
            sqlx::query("UPDATE face_detections SET cluster_id = ?1 WHERE id = ?2")
                .bind(db_cluster_id)
                .bind(face_id)
                .execute(pool)
                .await?;
        }
    }

    // Recompute photo_count authoritatively as the number of DISTINCT photos
    // in each affected cluster. The provisional `count` written above counts
    // face *detections*, which over-counts whenever a single photo (e.g. a
    // collage / movie-poster montage) contains multiple faces of the same
    // person — that is the "33 photos but only 6 in the album" bug. This must
    // run AFTER the loop above so newly-created clusters have their detections
    // assigned. Mirrors the COUNT(DISTINCT photo_id) rule used by the
    // merge/split handlers.
    let affected_clusters: std::collections::HashSet<i64> =
        cluster_id_map.values().copied().collect();
    for db_cluster_id in &affected_clusters {
        sqlx::query(
            "UPDATE face_clusters SET photo_count = (\
                 SELECT COUNT(DISTINCT photo_id) FROM face_detections \
                 WHERE cluster_id = ?1 AND user_id = ?2\
             ), updated_at = datetime('now') \
             WHERE id = ?1 AND user_id = ?2",
        )
        .bind(db_cluster_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    }

    // Apply face tags for all clustered faces
    for (face_id, _) in &assignments {
        let face_info: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT fd.photo_id, fd.cluster_id, COALESCE(fc.label, '') \
             FROM face_detections fd \
             LEFT JOIN face_clusters fc ON fc.id = fd.cluster_id \
             WHERE fd.id = ?1",
        )
        .bind(face_id)
        .fetch_optional(pool)
        .await?;

        if let Some((photo_id, cluster_id, label)) = face_info {
            let label_opt = if label.is_empty() {
                None
            } else {
                Some(label.as_str())
            };
            tagging::apply_face_tag(pool, user_id, &photo_id, cluster_id, label_opt).await?;
        }
    }

    tracing::info!(
        user_id = %user_id,
        faces_assigned = assignments.len(),
        total_clusters = unique_clusters.len(),
        existing_clusters = existing_count,
        new_clusters = new_clusters_created,
        updated_clusters = clusters_updated,
        elapsed_ms = cluster_start.elapsed().as_millis(),
        "AI clustering: complete"
    );

    Ok(())
}

/// Run pet individual clustering for a user.
///
/// Mirrors `run_clustering` but operates on `pet_detections` /
/// `pet_clusters`.  Pets of different species are never merged — we only
/// compare embeddings within the same species bucket.
async fn run_pet_clustering(
    pool: &SqlitePool,
    user_id: &str,
    similarity_threshold: f32,
) -> anyhow::Result<()> {
    // Load all pet detections with embeddings for this user, grouped by species
    let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT id, species, embedding FROM pet_detections \
         WHERE user_id = ?1 AND embedding IS NOT NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    // Group by species so we only cluster within-species
    let mut by_species: std::collections::HashMap<String, Vec<(i64, Vec<f32>)>> =
        std::collections::HashMap::new();

    for (id, species, bytes) in rows {
        if bytes.len() % 4 != 0 {
            continue;
        }
        let embedding: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        by_species.entry(species).or_default().push((id, embedding));
    }

    let cluster_start = Instant::now();
    let mut total_assigned = 0usize;

    for (species, pets) in &by_species {
        tracing::info!(
            user_id = %user_id,
            species = %species,
            detections = pets.len(),
            threshold = similarity_threshold,
            "AI pet clustering: running"
        );

        // Use the same agglomerative algorithm as face clustering
        let assignments = clustering::cluster_faces(pets, similarity_threshold);

        // Deduplicate cluster indices
        let mut unique_clusters: Vec<i64> = assignments.iter().map(|(_, c)| *c).collect();
        unique_clusters.sort();
        unique_clusters.dedup();

        // Fetch existing pet clusters for this user+species
        let existing: Vec<(i64,)> = sqlx::query_as(
            "SELECT id FROM pet_clusters WHERE user_id = ?1 AND species = ?2 ORDER BY id",
        )
        .bind(user_id)
        .bind(species)
        .fetch_all(pool)
        .await?;

        let mut cluster_id_map: std::collections::HashMap<i64, i64> =
            std::collections::HashMap::new();

        for cluster_idx in &unique_clusters {
            let count = assignments.iter().filter(|(_, c)| c == cluster_idx).count() as i64;

            // Check if any detection in this cluster is already assigned to a DB cluster
            let mut matched_db_cluster: Option<i64> = None;
            for (det_id, c) in &assignments {
                if c != cluster_idx {
                    continue;
                }
                let row: Option<(Option<i64>,)> =
                    sqlx::query_as("SELECT cluster_id FROM pet_detections WHERE id = ?1")
                        .bind(det_id)
                        .fetch_optional(pool)
                        .await?;
                if let Some((Some(cid),)) = row {
                    matched_db_cluster = Some(cid);
                    break;
                }
            }

            let db_cluster_id = if let Some(cid) = matched_db_cluster {
                // Update photo count on existing cluster
                sqlx::query(
                    "UPDATE pet_clusters SET photo_count = ?1, updated_at = datetime('now') \
                     WHERE id = ?2",
                )
                .bind(count)
                .bind(cid)
                .execute(pool)
                .await?;
                cid
            } else {
                // Create new cluster
                let result = sqlx::query(
                    "INSERT INTO pet_clusters (user_id, species, photo_count, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, datetime('now'), datetime('now'))"
                )
                .bind(user_id)
                .bind(species)
                .bind(count)
                .execute(pool)
                .await?;
                result.last_insert_rowid()
            };

            cluster_id_map.insert(*cluster_idx, db_cluster_id);
        }

        // Assign detections to clusters and apply tags
        for (det_id, cluster_idx) in &assignments {
            if let Some(db_cluster_id) = cluster_id_map.get(cluster_idx) {
                sqlx::query("UPDATE pet_detections SET cluster_id = ?1 WHERE id = ?2")
                    .bind(db_cluster_id)
                    .bind(det_id)
                    .execute(pool)
                    .await?;

                // Re-apply cluster-aware pet tag
                let info: Option<(String, Option<String>)> = sqlx::query_as(
                    "SELECT pd.photo_id, pc.label \
                     FROM pet_detections pd \
                     LEFT JOIN pet_clusters pc ON pc.id = pd.cluster_id \
                     WHERE pd.id = ?1",
                )
                .bind(det_id)
                .fetch_optional(pool)
                .await?;

                if let Some((photo_id, label)) = info {
                    tagging::apply_pet_tag(
                        pool,
                        user_id,
                        &photo_id,
                        Some(*db_cluster_id),
                        label.as_deref().unwrap_or(species),
                    )
                    .await?;
                }

                // Set representative thumbnail (highest-confidence photo)
                sqlx::query(
                    "UPDATE pet_clusters SET representative = \
                     COALESCE(representative, (SELECT photo_id FROM pet_detections \
                      WHERE cluster_id = ?1 ORDER BY confidence DESC LIMIT 1)) \
                     WHERE id = ?1",
                )
                .bind(db_cluster_id)
                .execute(pool)
                .await?;

                total_assigned += 1;
            }
        }

        // Recompute photo_count as DISTINCT photos, not pet detections — same
        // over-count bug as faces when one photo holds several crops of the
        // same animal. Runs after assignment so new clusters are populated.
        let affected: std::collections::HashSet<i64> = cluster_id_map.values().copied().collect();
        for db_cluster_id in &affected {
            sqlx::query(
                "UPDATE pet_clusters SET photo_count = (\
                     SELECT COUNT(DISTINCT photo_id) FROM pet_detections \
                     WHERE cluster_id = ?1 AND user_id = ?2\
                 ), updated_at = datetime('now') \
                 WHERE id = ?1 AND user_id = ?2",
            )
            .bind(db_cluster_id)
            .bind(user_id)
            .execute(pool)
            .await?;
        }

        let _ = existing; // suppress unused warning
    }

    tracing::info!(
        user_id = %user_id,
        assigned = total_assigned,
        elapsed_ms = cluster_start.elapsed().as_millis(),
        "AI pet clustering: complete"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};
    use std::io::Cursor;

    /// Encode a small valid PNG in memory for decode tests.
    fn tiny_png(w: u32, h: u32) -> Vec<u8> {
        let img = DynamicImage::ImageRgb8(RgbImage::new(w, h));
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .expect("encode png");
        buf.into_inner()
    }

    /// A well-formed image within the allocation ceiling decodes normally.
    #[test]
    fn decode_bounded_accepts_normal_image() {
        let png = tiny_png(32, 24);
        let img = decode_image_bounded(&png).expect("small png should decode");
        assert_eq!(img.width(), 32);
        assert_eq!(img.height(), 24);
    }

    /// Garbage bytes must return an error, never panic or abort the process —
    /// the guarantee the decode-bomb guard relies on to keep the loop alive.
    #[test]
    fn decode_bounded_rejects_garbage() {
        let junk = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0x03];
        assert!(decode_image_bounded(&junk).is_err());
    }
}
