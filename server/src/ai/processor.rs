//! Background AI processor.
//!
//! Runs as a Tokio task (spawned from `tasks.rs`). Periodically scans for
//! unprocessed photos, runs face detection, object detection, face clustering,
//! and auto-tagging.
//!
//! Rate-limited by `photos_per_minute` config to avoid overwhelming the CPU.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::time;
use tracing;

use crate::ai::clustering;
use crate::ai::engine::AiEngine;
use crate::ai::face;
use crate::ai::object;
use crate::ai::tagging;
use crate::config::AiConfig;

/// Spawn the background AI processor task.
///
/// Always spawns the processor regardless of `config.enabled`. The processor
/// checks per-user `ai_enabled` settings each cycle, using `config.enabled`
/// as the default for users who haven't explicitly toggled. This allows the
/// runtime toggle (`POST /api/ai/toggle`) to actually work.
pub fn spawn_ai_processor(pool: SqlitePool, config: AiConfig, storage_root: PathBuf) {
    let engine = AiEngine::new(&config);
    if !engine.has_any_capability() {
        tracing::warn!(
            "AI processor: no ONNX models found in '{}'. \
             Using fallback heuristic detectors.",
            config.model_dir
        );
    }

    tokio::spawn(async move {
        // Wait for server startup to complete before starting AI processing
        time::sleep(Duration::from_secs(30)).await;

        let photos_per_minute = config.photos_per_minute.max(1);
        let interval = Duration::from_secs(60 / photos_per_minute as u64);

        tracing::info!(
            "AI processor started: {} photos/min, batch_size={}, provider={}, config_default={}",
            photos_per_minute,
            config.batch_size,
            engine.provider(),
            if config.enabled { "enabled" } else { "disabled" }
        );

        loop {
            if let Err(e) = process_batch(&pool, &engine, &config, &storage_root).await {
                tracing::error!("AI processor error: {}", e);
            }

            time::sleep(interval).await;
        }
    });
}

/// Process a batch of unprocessed photos.
async fn process_batch(
    pool: &SqlitePool,
    engine: &AiEngine,
    config: &AiConfig,
    storage_root: &PathBuf,
) -> anyhow::Result<()> {
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
         AND p.file_path IS NOT NULL \
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
        return Ok(());
    }

    tracing::info!(
        "AI processor: batch of {} photo(s) queued for recognition [provider={}]",
        unprocessed.len(),
        engine.provider()
    );

    let batch_start = Instant::now();
    let mut total_faces = 0usize;
    let mut total_objects = 0usize;

    for (photo_id, user_id, filename) in &unprocessed {
        let photo_start = Instant::now();
        match process_single_photo(pool, engine, config, storage_root, photo_id, user_id, filename).await {
            Ok((nf, no)) => {
                total_faces += nf;
                total_objects += no;
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
                // Mark as processed anyway to avoid infinite retry loops
                mark_processed(pool, photo_id, user_id).await?;
            }
        }
    }

    tracing::info!(
        photos = unprocessed.len(),
        faces_found = total_faces,
        objects_found = total_objects,
        elapsed_ms = batch_start.elapsed().as_millis(),
        "AI processor: batch complete"
    );

    // After processing a batch, re-run clustering for any users that had new detections
    let users_with_new: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT user_id FROM face_detections WHERE cluster_id IS NULL"
    )
    .fetch_all(pool)
    .await?;

    for (user_id,) in &users_with_new {
        if let Err(e) = run_clustering(pool, user_id, config.face_similarity_threshold).await {
            tracing::warn!("AI processor: clustering failed for user {}: {}", user_id, e);
        }
    }

    Ok(())
}

/// Process a single photo: detect faces and objects, save to DB.
/// Returns (face_count, object_count) on success.
async fn process_single_photo(
    pool: &SqlitePool,
    _engine: &AiEngine,
    config: &AiConfig,
    storage_root: &PathBuf,
    photo_id: &str,
    user_id: &str,
    filename: &str,
) -> anyhow::Result<(usize, usize)> {
    // Load the photo file
    let file_path: Option<(String,)> = sqlx::query_as(
        "SELECT file_path FROM photos WHERE id = ?1 AND user_id = ?2"
    )
    .bind(photo_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let file_path = match file_path {
        Some((p,)) => p,
        None => {
            tracing::debug!(photo_id = %photo_id, "AI: photo not found in DB, skipping");
            mark_processed(pool, photo_id, user_id).await?;
            return Ok((0, 0));
        }
    };

    // Resolve relative file_path against storage root
    let abs_path = storage_root.join(&file_path);

    // Read the image bytes
    let image_bytes = match tokio::fs::read(&abs_path).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::debug!(file_path = %file_path, abs_path = ?abs_path, error = %e, "AI: cannot read photo file, skipping");
            mark_processed(pool, photo_id, user_id).await?;
            return Ok((0, 0));
        }
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

    // Decode image once for both pipelines
    let img = match image::load_from_memory(&image_bytes) {
        Ok(img) => img,
        Err(e) => {
            tracing::debug!(filename = %filename, error = %e, "AI: cannot decode image, skipping");
            mark_processed(pool, photo_id, user_id).await?;
            return Ok((0, 0));
        }
    };
    tracing::debug!(
        photo_id = %photo_id,
        width = img.width(),
        height = img.height(),
        "AI: image decoded, running detection pipelines"
    );

    // Clear any previous AI tags before re-processing
    tagging::clear_ai_tags(pool, user_id, photo_id).await?;

    // Face detection
    let face_start = Instant::now();
    let face_detections = face::detect_faces_from_image(&img, config.face_confidence)?;
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
        // Extract embedding for clustering
        let embedding = face::extract_face_embedding(&img, &det.bbox);
        let embedding_bytes: Vec<u8> = embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        sqlx::query(
            "INSERT INTO face_detections \
             (photo_id, user_id, bbox_x, bbox_y, bbox_w, bbox_h, confidence, embedding) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
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
    let obj_detections = object::detect_objects_with_quality(&img, config.object_confidence, quality)?;
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
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
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

    mark_processed(pool, photo_id, user_id).await?;

    Ok((face_detections.len(), obj_detections.len()))
}

/// Mark a photo as AI-processed.
async fn mark_processed(pool: &SqlitePool, photo_id: &str, user_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO ai_processed_photos (photo_id, user_id, processed_at) \
         VALUES (?1, ?2, datetime('now'))"
    )
    .bind(photo_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(())
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
        "SELECT id, embedding FROM face_detections WHERE user_id = ?1 AND embedding IS NOT NULL"
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
    let existing_clusters: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM face_clusters WHERE user_id = ?1 ORDER BY id"
    )
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
            let existing: Option<(Option<i64>,)> = sqlx::query_as(
                "SELECT cluster_id FROM face_detections WHERE id = ?1"
            )
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
                     VALUES (?1, ?2, datetime('now'), datetime('now'))"
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
            sqlx::query(
                "UPDATE face_detections SET cluster_id = ?1 WHERE id = ?2"
            )
            .bind(db_cluster_id)
            .bind(face_id)
            .execute(pool)
            .await?;
        }
    }

    // Apply face tags for all clustered faces
    for (face_id, _) in &assignments {
        let face_info: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT fd.photo_id, fd.cluster_id, COALESCE(fc.label, '') \
             FROM face_detections fd \
             LEFT JOIN face_clusters fc ON fc.id = fd.cluster_id \
             WHERE fd.id = ?1"
        )
        .bind(face_id)
        .fetch_optional(pool)
        .await?;

        if let Some((photo_id, cluster_id, label)) = face_info {
            let label_opt = if label.is_empty() { None } else { Some(label.as_str()) };
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
