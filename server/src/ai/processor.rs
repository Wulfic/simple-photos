//! Background AI processor.
//!
//! Runs as a Tokio task (spawned from `tasks.rs`). Periodically scans for
//! unprocessed photos, runs face detection, object detection, face clustering,
//! and auto-tagging.
//!
//! Rate-limited by `photos_per_minute` config to avoid overwhelming the CPU.

use std::time::Duration;

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
pub fn spawn_ai_processor(pool: SqlitePool, config: AiConfig) {
    if !config.enabled {
        tracing::info!("AI processing is disabled by config");
        return;
    }

    let engine = AiEngine::new(&config);
    if !engine.has_any_capability() {
        tracing::warn!(
            "AI processing is enabled but no models are available. \
             Place ONNX models in '{}' to enable AI features. \
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
            "AI processor started: {} photos/min, batch_size={}, provider={}",
            photos_per_minute,
            config.batch_size,
            engine.provider()
        );

        loop {
            if let Err(e) = process_batch(&pool, &engine, &config).await {
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
) -> anyhow::Result<()> {
    // Check if AI is still enabled (could be toggled via user_settings)
    let enabled: Option<(String,)> = sqlx::query_as(
        "SELECT value FROM user_settings WHERE key = 'ai_enabled' LIMIT 1"
    )
    .fetch_optional(pool)
    .await?;

    if let Some((val,)) = &enabled {
        if val == "false" {
            return Ok(());
        }
    }

    // Find unprocessed photos (photos not in ai_processed_photos)
    let unprocessed: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT p.id, p.user_id, p.filename FROM photos p \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM ai_processed_photos ap \
             WHERE ap.photo_id = p.id AND ap.user_id = p.user_id \
         ) \
         AND p.encrypted_blob_id IS NULL \
         ORDER BY p.created_at DESC \
         LIMIT ?1"
    )
    .bind(config.batch_size as i64)
    .fetch_all(pool)
    .await?;

    if unprocessed.is_empty() {
        return Ok(());
    }

    tracing::debug!("AI processor: processing {} photos", unprocessed.len());

    for (photo_id, user_id, filename) in &unprocessed {
        if let Err(e) = process_single_photo(pool, engine, config, photo_id, user_id, &filename).await {
            tracing::warn!("AI processor: failed to process photo {}: {}", photo_id, e);
            // Mark as processed anyway to avoid infinite retry loops
            mark_processed(pool, photo_id, user_id).await?;
        }
    }

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
async fn process_single_photo(
    pool: &SqlitePool,
    _engine: &AiEngine,
    config: &AiConfig,
    photo_id: &str,
    user_id: &str,
    filename: &str,
) -> anyhow::Result<()> {
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
            tracing::debug!("AI: photo {} not found in DB, skipping", photo_id);
            mark_processed(pool, photo_id, user_id).await?;
            return Ok(());
        }
    };

    // Read the image bytes
    let image_bytes = match tokio::fs::read(&file_path).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::debug!("AI: cannot read {}: {}", file_path, e);
            mark_processed(pool, photo_id, user_id).await?;
            return Ok(());
        }
    };

    // Skip very small files (probably thumbnails)
    if image_bytes.len() < 1000 {
        mark_processed(pool, photo_id, user_id).await?;
        return Ok(());
    }

    // Decode image once for both pipelines
    let img = match image::load_from_memory(&image_bytes) {
        Ok(img) => img,
        Err(e) => {
            tracing::debug!("AI: cannot decode {}: {}", filename, e);
            mark_processed(pool, photo_id, user_id).await?;
            return Ok(());
        }
    };

    // Clear any previous AI tags before re-processing
    tagging::clear_ai_tags(pool, user_id, photo_id).await?;

    // Face detection
    let face_detections = face::detect_faces_from_image(&img, config.face_confidence)?;
    tracing::debug!(
        "AI: {} faces detected in {} ({})",
        face_detections.len(),
        filename,
        photo_id
    );

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
    let obj_detections = object::detect_objects_from_image(&img, config.object_confidence)?;
    tracing::debug!(
        "AI: {} objects detected in {} ({})",
        obj_detections.len(),
        filename,
        photo_id
    );

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

    Ok(())
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
    let mut cluster_id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();

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
        "AI clustering: assigned {} faces to {} clusters for user {}",
        assignments.len(),
        unique_clusters.len(),
        user_id
    );

    Ok(())
}
