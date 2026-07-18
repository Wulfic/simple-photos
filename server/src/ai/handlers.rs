//! HTTP handlers for AI recognition endpoints.
//!
//! Provides routes for:
//! - AI status and configuration
//! - Face cluster management (list, rename, merge, split)
//! - Object detection results
//! - AI processing control (enable/disable, reprocess)

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::*;
use super::tagging;

// ── Status & config ──────────────────────────────────────────────────

/// GET /api/ai/status — current AI processing status and capabilities.
pub async fn ai_status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<AiStatusResponse>, AppError> {
    let config = &state.config.ai;

    // Check user-level toggle
    let user_enabled: Option<(String,)> =
        sqlx::query_as("SELECT value FROM user_settings WHERE user_id = ?1 AND key = 'ai_enabled'")
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await?;

    let enabled = match user_enabled {
        Some((val,)) => val != "false",
        None => config.enabled,
    };

    // Count processed and pending photos
    let processed: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM ai_processed_photos WHERE user_id = ?1")
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    let total: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM photos WHERE user_id = ?1 AND file_path IS NOT NULL")
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    let face_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM face_detections WHERE user_id = ?1")
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    let cluster_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT cluster_id) FROM face_detections WHERE user_id = ?1 AND cluster_id IS NOT NULL"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let object_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM object_detections WHERE user_id = ?1")
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    let pet_det_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM pet_detections WHERE user_id = ?1")
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    let pet_cluster_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM pet_clusters WHERE user_id = ?1")
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    // Re-derive model availability the same way AiEngine does so the
    // status report is honest about whether real ONNX models are loaded.
    let model_dir = std::path::PathBuf::from(&config.model_dir);
    let face_model_loaded = model_dir.join("det_10g.onnx").exists()
        || model_dir.join("ultraface-RFB-320.onnx").exists()
        || model_dir.join("face_detection.onnx").exists();
    let object_model_loaded = model_dir.join("mobilenetv2-12.onnx").exists()
        || model_dir.join("object_detection.onnx").exists();
    let degraded_mode =
        !face_model_loaded && !object_model_loaded && !config.allow_heuristic_fallback;

    Ok(Json(AiStatusResponse {
        enabled,
        // Reflect the actual execution provider negotiated by AiEngine at
        // startup (honours `ai.gpu_preferred`, runtime CUDA availability,
        // AND the compile-time `cuda` cargo feature so we don't lie when
        // the binary lacks the EP). See `crate::ai::engine::AiEngine::new`.
        gpu_available: matches!(
            crate::ai::session::current().provider,
            crate::ai::engine::ExecutionProvider::Cuda
        ),
        photos_processed: processed.0,
        photos_pending: total.0 - processed.0,
        face_detections: face_count.0,
        face_clusters: cluster_count.0,
        object_detections: object_count.0,
        pet_detections: pet_det_count.0,
        pet_clusters: pet_cluster_count.0,
        face_model_loaded,
        object_model_loaded,
        degraded_mode,
        allow_heuristic_fallback: config.allow_heuristic_fallback,
    }))
}

// ── Enable / disable ─────────────────────────────────────────────────

/// POST /api/ai/toggle — enable or disable AI processing for this user.
pub async fn ai_toggle(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<AiToggleRequest>,
) -> Result<StatusCode, AppError> {
    let value = if body.enabled { "true" } else { "false" };

    sqlx::query(
        "INSERT INTO user_settings (user_id, key, value, updated_at) \
         VALUES (?1, 'ai_enabled', ?2, datetime('now')) \
         ON CONFLICT(user_id, key) DO UPDATE SET value = ?2, updated_at = datetime('now')",
    )
    .bind(&auth.user_id)
    .bind(value)
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Reprocess ────────────────────────────────────────────────────────

/// POST /api/ai/reprocess — clear and reprocess all (or specific) photos.
pub async fn ai_reprocess(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<AiReprocessRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let cleared = match &body.photo_ids {
        Some(ids) if !ids.is_empty() => {
            let mut count = 0i64;
            for id in ids {
                // Delete existing detections
                sqlx::query("DELETE FROM face_detections WHERE photo_id = ?1 AND user_id = ?2")
                    .bind(id)
                    .bind(&auth.user_id)
                    .execute(&state.pool)
                    .await?;

                sqlx::query("DELETE FROM object_detections WHERE photo_id = ?1 AND user_id = ?2")
                    .bind(id)
                    .bind(&auth.user_id)
                    .execute(&state.pool)
                    .await?;

                sqlx::query("DELETE FROM pet_detections WHERE photo_id = ?1 AND user_id = ?2")
                    .bind(id)
                    .bind(&auth.user_id)
                    .execute(&state.pool)
                    .await?;

                // Remove from processed list so the background processor picks it up
                let result = sqlx::query(
                    "DELETE FROM ai_processed_photos WHERE photo_id = ?1 AND user_id = ?2",
                )
                .bind(id)
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

                count += result.rows_affected() as i64;

                // Clear AI tags
                tagging::clear_ai_tags(&state.pool, &auth.user_id, id).await?;
            }
            // Clean up orphaned face clusters (clusters with no remaining detections)
            sqlx::query(
                "DELETE FROM face_clusters WHERE user_id = ?1 AND id NOT IN \
                 (SELECT DISTINCT cluster_id FROM face_detections WHERE user_id = ?1 AND cluster_id IS NOT NULL)"
            )
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;

            // Clean up orphaned pet clusters
            sqlx::query(
                "DELETE FROM pet_clusters WHERE user_id = ?1 AND id NOT IN \
                 (SELECT DISTINCT cluster_id FROM pet_detections WHERE user_id = ?1 AND cluster_id IS NOT NULL)"
            )
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;
            count
        }
        _ => {
            // Reprocess ALL photos
            sqlx::query("DELETE FROM face_detections WHERE user_id = ?1")
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

            sqlx::query("DELETE FROM object_detections WHERE user_id = ?1")
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

            // Clear pet detections and clusters
            sqlx::query("DELETE FROM pet_detections WHERE user_id = ?1")
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

            sqlx::query("DELETE FROM pet_clusters WHERE user_id = ?1")
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

            // Clear face clusters to prevent orphaned cluster data
            sqlx::query("DELETE FROM face_clusters WHERE user_id = ?1")
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

            let result = sqlx::query("DELETE FROM ai_processed_photos WHERE user_id = ?1")
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

            // Clear all AI tags for this user
            sqlx::query(
                "DELETE FROM photo_tags WHERE user_id = ?1 AND (tag LIKE 'person:%' OR tag LIKE 'object:%' OR tag LIKE 'pet:%')"
            )
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;

            result.rows_affected() as i64
        }
    };

    Ok(Json(serde_json::json!({
        "cleared": cleared,
        "message": "Photos queued for reprocessing"
    })))
}

// ── Face clusters ────────────────────────────────────────────────────

/// GET /api/ai/faces — list all face clusters for the current user.
pub async fn list_face_clusters(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<FaceClusterSummary>>, AppError> {
    // `representative` is not yet populated by the processor; fall back to
    // the highest-confidence face detection's photo so the People grid in
    // the UI shows a real thumbnail instead of a generic placeholder.
    // Smart-album rule: require at least 2 distinct photos before surfacing a
    // person card. This prevents random faces in group photos / crowds from
    // creating noisy single-photo "People" entries.
    let clusters = fetch_face_clusters(&state.pool, &auth.user_id).await?;
    Ok(Json(clusters))
}

/// Query the face-cluster summaries for a user, including the representative
/// face's bbox so clients can crop the People tile to the face.
///
/// `rep` is the highest-confidence detection on the representative photo. The
/// join is LEFT so clusters whose representative detection can't be resolved
/// still list (bbox simply comes back NULL). Extracted from the handler so it
/// can be unit-tested against an in-memory DB.
pub(crate) async fn fetch_face_clusters(
    pool: &sqlx::SqlitePool,
    user_id: &str,
) -> Result<Vec<FaceClusterSummary>, sqlx::Error> {
    sqlx::query_as(
        "SELECT fc.id, fc.label, fc.photo_count, \
                COALESCE(\
                    fc.representative, \
                    (SELECT fd.photo_id FROM face_detections fd \
                     WHERE fd.cluster_id = fc.id \
                     ORDER BY fd.confidence DESC LIMIT 1) \
                ) AS representative, \
                rep.bbox_x AS rep_bbox_x, rep.bbox_y AS rep_bbox_y, \
                rep.bbox_w AS rep_bbox_w, rep.bbox_h AS rep_bbox_h, \
                fc.created_at, fc.updated_at \
         FROM face_clusters fc \
         LEFT JOIN face_detections rep ON rep.id = (\
             SELECT fd2.id FROM face_detections fd2 \
             WHERE fd2.cluster_id = fc.id \
               AND fd2.photo_id = COALESCE(\
                   fc.representative, \
                   (SELECT fd3.photo_id FROM face_detections fd3 \
                    WHERE fd3.cluster_id = fc.id \
                    ORDER BY fd3.confidence DESC LIMIT 1)) \
             ORDER BY fd2.confidence DESC LIMIT 1) \
         WHERE fc.user_id = ?1 AND fc.photo_count >= 2 \
         ORDER BY fc.photo_count DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// GET /api/ai/faces/:cluster_id/photos — list photos in a face cluster.
pub async fn list_cluster_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cluster_id): Path<i64>,
) -> Result<Json<Vec<FaceDetectionRecord>>, AppError> {
    // Verify cluster belongs to user
    let _cluster: (i64,) =
        sqlx::query_as("SELECT id FROM face_clusters WHERE id = ?1 AND user_id = ?2")
            .bind(cluster_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let detections: Vec<FaceDetectionRecord> = sqlx::query_as(
        "SELECT fd.id, fd.photo_id, fd.cluster_id, fd.bbox_x, fd.bbox_y, fd.bbox_w, fd.bbox_h, \
                fd.confidence, fd.created_at \
         FROM face_detections fd \
         WHERE fd.cluster_id = ?1 AND fd.user_id = ?2 \
         ORDER BY fd.confidence DESC",
    )
    .bind(cluster_id)
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(detections))
}

/// PUT /api/ai/faces/:cluster_id/name — rename a face cluster.
pub async fn rename_face_cluster(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cluster_id): Path<i64>,
    Json(body): Json<RenameFaceRequest>,
) -> Result<StatusCode, AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("Name cannot be empty".into()));
    }
    if name.len() > 100 {
        return Err(AppError::BadRequest("Name too long (max 100 chars)".into()));
    }

    // Verify cluster belongs to user
    let result = sqlx::query(
        "UPDATE face_clusters SET label = ?1, updated_at = datetime('now') \
         WHERE id = ?2 AND user_id = ?3",
    )
    .bind(name)
    .bind(cluster_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // Update all associated photo tags
    tagging::rename_cluster_tags(&state.pool, &auth.user_id, cluster_id, name).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/ai/faces/merge — merge multiple face clusters into one.
pub async fn merge_face_clusters(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<MergeFacesRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.cluster_ids.len() < 2 {
        return Err(AppError::BadRequest(
            "Need at least 2 clusters to merge".into(),
        ));
    }

    // Verify all clusters belong to user
    let target_id = body.cluster_ids[0];
    for cid in &body.cluster_ids {
        let exists: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM face_clusters WHERE id = ?1 AND user_id = ?2")
                .bind(cid)
                .bind(&auth.user_id)
                .fetch_optional(&state.pool)
                .await?;

        if exists.is_none() {
            return Err(AppError::BadRequest(format!("Cluster {cid} not found")));
        }
    }

    // Move all face detections to the target cluster
    for cid in &body.cluster_ids[1..] {
        sqlx::query(
            "UPDATE face_detections SET cluster_id = ?1 WHERE cluster_id = ?2 AND user_id = ?3",
        )
        .bind(target_id)
        .bind(cid)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

        // Delete the source cluster
        sqlx::query("DELETE FROM face_clusters WHERE id = ?1 AND user_id = ?2")
            .bind(cid)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;
    }

    // Update photo count on target cluster
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT photo_id) FROM face_detections WHERE cluster_id = ?1 AND user_id = ?2"
    )
    .bind(target_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    sqlx::query(
        "UPDATE face_clusters SET photo_count = ?1, updated_at = datetime('now') WHERE id = ?2",
    )
    .bind(count.0)
    .bind(target_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({
        "merged_into": target_id,
        "photo_count": count.0
    })))
}

/// POST /api/ai/faces/split — move specific face detections to a new cluster.
pub async fn split_face_cluster(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SplitFacesRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.detection_ids.is_empty() {
        return Err(AppError::BadRequest("No detection IDs provided".into()));
    }

    // Verify all detections belong to user
    for did in &body.detection_ids {
        let exists: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM face_detections WHERE id = ?1 AND user_id = ?2")
                .bind(did)
                .bind(&auth.user_id)
                .fetch_optional(&state.pool)
                .await?;

        if exists.is_none() {
            return Err(AppError::BadRequest(format!("Detection {did} not found")));
        }
    }

    // Create a new cluster
    let result = sqlx::query(
        "INSERT INTO face_clusters (user_id, photo_count, created_at, updated_at) \
         VALUES (?1, ?2, datetime('now'), datetime('now'))",
    )
    .bind(&auth.user_id)
    .bind(body.detection_ids.len() as i64)
    .execute(&state.pool)
    .await?;

    let new_cluster_id = result.last_insert_rowid();

    // Move detections to the new cluster
    // Track the old cluster IDs so we can update their counts
    let mut old_cluster_ids = std::collections::HashSet::new();
    for did in &body.detection_ids {
        let old: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT cluster_id FROM face_detections WHERE id = ?1")
                .bind(did)
                .fetch_optional(&state.pool)
                .await?;

        if let Some((Some(old_cid),)) = old {
            old_cluster_ids.insert(old_cid);
        }

        sqlx::query("UPDATE face_detections SET cluster_id = ?1 WHERE id = ?2 AND user_id = ?3")
            .bind(new_cluster_id)
            .bind(did)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;
    }

    // Update photo counts on old clusters
    for old_cid in &old_cluster_ids {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(DISTINCT photo_id) FROM face_detections WHERE cluster_id = ?1 AND user_id = ?2"
        )
        .bind(old_cid)
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;

        if count.0 == 0 {
            // Delete empty clusters
            sqlx::query("DELETE FROM face_clusters WHERE id = ?1 AND user_id = ?2")
                .bind(old_cid)
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;
        } else {
            sqlx::query(
                "UPDATE face_clusters SET photo_count = ?1, updated_at = datetime('now') WHERE id = ?2"
            )
            .bind(count.0)
            .bind(old_cid)
            .execute(&state.pool)
            .await?;
        }
    }

    Ok(Json(serde_json::json!({
        "new_cluster_id": new_cluster_id,
        "detection_count": body.detection_ids.len()
    })))
}

/// GET /api/ai/photos/:photo_id/faces — list the faces detected in one photo,
/// each joined to its current person (cluster) label. Powers the manual
/// "fix the person" UI in the photo info panel.
pub async fn list_photo_faces(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<Vec<PhotoFaceRecord>>, AppError> {
    let faces: Vec<PhotoFaceRecord> = sqlx::query_as(
        "SELECT fd.id, fd.cluster_id, fc.label AS cluster_label, \
                fd.bbox_x, fd.bbox_y, fd.bbox_w, fd.bbox_h, fd.confidence \
         FROM face_detections fd \
         LEFT JOIN face_clusters fc ON fc.id = fd.cluster_id AND fc.user_id = fd.user_id \
         WHERE fd.user_id = ?1 AND fd.photo_id = ?2 \
         ORDER BY fd.confidence DESC",
    )
    .bind(&auth.user_id)
    .bind(&photo_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(faces))
}

/// Recompute and persist `photo_count` for a face cluster. When `prune_empty`
/// is set and the cluster no longer has any detections it is deleted (used for
/// the *source* cluster of a move so orphans don't linger in the People grid).
async fn refresh_face_cluster_count(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    cluster_id: i64,
    prune_empty: bool,
) -> Result<(), AppError> {
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT photo_id) FROM face_detections WHERE cluster_id = ?1 AND user_id = ?2",
    )
    .bind(cluster_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    if prune_empty && count.0 == 0 {
        sqlx::query("DELETE FROM face_clusters WHERE id = ?1 AND user_id = ?2")
            .bind(cluster_id)
            .bind(user_id)
            .execute(pool)
            .await?;
    } else {
        sqlx::query(
            "UPDATE face_clusters SET photo_count = ?1, updated_at = datetime('now') \
             WHERE id = ?2 AND user_id = ?3",
        )
        .bind(count.0)
        .bind(cluster_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// POST /api/ai/faces/assign — manually move one face detection into a chosen
/// person (cluster). This is the correction path when the AI grouped a face
/// under the wrong person. Keeps cluster photo-counts and the photo's
/// `person:*` tags consistent, and prunes the source cluster if it empties out.
pub async fn assign_face(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<AssignFaceRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Look up the detection (must belong to this user) and remember where it was.
    let detection: Option<(Option<i64>, String)> = sqlx::query_as(
        "SELECT cluster_id, photo_id FROM face_detections WHERE id = ?1 AND user_id = ?2",
    )
    .bind(body.detection_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;
    let (old_cluster_id, photo_id) = detection.ok_or(AppError::NotFound)?;

    // The target person must exist and belong to this user.
    let target: Option<(Option<String>,)> =
        sqlx::query_as("SELECT label FROM face_clusters WHERE id = ?1 AND user_id = ?2")
            .bind(body.cluster_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await?;
    if target.is_none() {
        return Err(AppError::BadRequest("Target person not found".into()));
    }

    // No-op if it's already assigned to the requested person.
    if old_cluster_id == Some(body.cluster_id) {
        return Ok(Json(serde_json::json!({
            "detection_id": body.detection_id,
            "cluster_id": body.cluster_id,
            "photo_id": photo_id,
            "unchanged": true,
        })));
    }

    sqlx::query("UPDATE face_detections SET cluster_id = ?1 WHERE id = ?2 AND user_id = ?3")
        .bind(body.cluster_id)
        .bind(body.detection_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    // Keep counts honest: bump the target, and prune the source if now empty.
    refresh_face_cluster_count(&state.pool, &auth.user_id, body.cluster_id, false).await?;
    if let Some(old) = old_cluster_id {
        refresh_face_cluster_count(&state.pool, &auth.user_id, old, true).await?;
    }

    // Person tags on a photo are derived from every cluster present in it, so
    // rebuild them for this photo after the move.
    tagging::resync_photo_face_tags(&state.pool, &auth.user_id, &photo_id).await?;

    tracing::info!(
        user_id = %auth.user_id,
        detection_id = body.detection_id,
        photo_id = %photo_id,
        from_cluster = ?old_cluster_id,
        to_cluster = body.cluster_id,
        "Manual face reassignment"
    );

    Ok(Json(serde_json::json!({
        "detection_id": body.detection_id,
        "cluster_id": body.cluster_id,
        "photo_id": photo_id,
    })))
}

// ── Object detections ────────────────────────────────────────────────

/// GET /api/ai/objects — list unique object classes detected for this user.
pub async fn list_object_classes(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ObjectClassSummary>>, AppError> {
    let classes: Vec<ObjectClassSummary> = sqlx::query_as(
        "SELECT class_name, COUNT(*) as photo_count, AVG(confidence) as avg_confidence \
         FROM object_detections \
         WHERE user_id = ?1 \
         GROUP BY class_name \
         ORDER BY photo_count DESC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(classes))
}

/// GET /api/ai/objects/:class_name/photos — list photos containing a specific object.
pub async fn list_object_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(class_name): Path<String>,
) -> Result<Json<Vec<ObjectDetectionRecord>>, AppError> {
    let detections: Vec<ObjectDetectionRecord> = sqlx::query_as(
        "SELECT id, photo_id, class_name, confidence, bbox_x, bbox_y, bbox_w, bbox_h, created_at \
         FROM object_detections \
         WHERE user_id = ?1 AND class_name = ?2 \
         ORDER BY confidence DESC",
    )
    .bind(&auth.user_id)
    .bind(&class_name)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(detections))
}

// ── Pet clusters ────────────────────────────────────────────────────

/// GET /api/ai/pets — list all pet clusters for the current user.
pub async fn list_pet_clusters(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<PetClusterSummary>>, AppError> {
    // Smart-album rule: require at least 2 distinct photos before surfacing a
    // pet card. Same rationale as faces — a single lone detection is noise.
    let clusters: Vec<PetClusterSummary> = sqlx::query_as(
        "SELECT pc.id, pc.label, pc.species, pc.photo_count, \
                COALESCE(\
                    pc.representative, \
                    (SELECT pd.photo_id FROM pet_detections pd \
                     WHERE pd.cluster_id = pc.id \
                     ORDER BY pd.confidence DESC LIMIT 1) \
                ) AS representative, \
                pc.created_at, pc.updated_at \
         FROM pet_clusters pc \
         WHERE pc.user_id = ?1 AND pc.photo_count >= 2 \
         ORDER BY pc.photo_count DESC, pc.species ASC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(clusters))
}

/// GET /api/ai/pets/:cluster_id/photos — list photos in a pet cluster.
pub async fn list_pet_cluster_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cluster_id): Path<i64>,
) -> Result<Json<Vec<PetDetectionRecord>>, AppError> {
    // Verify cluster belongs to user
    let _cluster: (i64,) =
        sqlx::query_as("SELECT id FROM pet_clusters WHERE id = ?1 AND user_id = ?2")
            .bind(cluster_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let detections: Vec<PetDetectionRecord> = sqlx::query_as(
        "SELECT id, photo_id, cluster_id, species, confidence, created_at \
         FROM pet_detections \
         WHERE cluster_id = ?1 AND user_id = ?2 \
         ORDER BY confidence DESC",
    )
    .bind(cluster_id)
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(detections))
}

/// PUT /api/ai/pets/:cluster_id/name — rename a pet cluster.
pub async fn rename_pet_cluster(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cluster_id): Path<i64>,
    Json(body): Json<RenameFaceRequest>,
) -> Result<StatusCode, AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("Name cannot be empty".into()));
    }
    if name.len() > 100 {
        return Err(AppError::BadRequest("Name too long (max 100 chars)".into()));
    }

    let result = sqlx::query(
        "UPDATE pet_clusters SET label = ?1, updated_at = datetime('now') \
         WHERE id = ?2 AND user_id = ?3",
    )
    .bind(name)
    .bind(cluster_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // Update all photo tags for this cluster
    tagging::rename_pet_cluster_tags(&state.pool, &auth.user_id, cluster_id, name).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/ai/pets/merge — merge multiple pet clusters into one.
pub async fn merge_pet_clusters(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<MergeFacesRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.cluster_ids.len() < 2 {
        return Err(AppError::BadRequest(
            "Need at least 2 clusters to merge".into(),
        ));
    }

    let target_id = body.cluster_ids[0];

    for cid in &body.cluster_ids {
        let exists: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM pet_clusters WHERE id = ?1 AND user_id = ?2")
                .bind(cid)
                .bind(&auth.user_id)
                .fetch_optional(&state.pool)
                .await?;

        if exists.is_none() {
            return Err(AppError::BadRequest(format!("Pet cluster {cid} not found")));
        }
    }

    for cid in &body.cluster_ids[1..] {
        sqlx::query(
            "UPDATE pet_detections SET cluster_id = ?1 WHERE cluster_id = ?2 AND user_id = ?3",
        )
        .bind(target_id)
        .bind(cid)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

        sqlx::query("DELETE FROM pet_clusters WHERE id = ?1 AND user_id = ?2")
            .bind(cid)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;
    }

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT photo_id) FROM pet_detections WHERE cluster_id = ?1 AND user_id = ?2"
    )
    .bind(target_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    sqlx::query(
        "UPDATE pet_clusters SET photo_count = ?1, updated_at = datetime('now') WHERE id = ?2",
    )
    .bind(count.0)
    .bind(target_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({
        "merged_into": target_id,
        "photo_count": count.0
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    async fn memory_pool() -> sqlx::SqlitePool {
        // See gallery::summary tests: single connection to a shared in-memory DB
        // and FKs off so we can insert bare face rows without the users/photos graph.
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn insert_cluster(pool: &sqlx::SqlitePool, id: i64, user: &str, photo_count: i64) {
        sqlx::query(
            "INSERT INTO face_clusters (id, user_id, label, representative, photo_count, created_at, updated_at) \
             VALUES (?, ?, NULL, NULL, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(user)
        .bind(photo_count)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_detection(
        pool: &sqlx::SqlitePool,
        user: &str,
        photo: &str,
        cluster: i64,
        bbox: (f64, f64, f64, f64),
        confidence: f64,
    ) {
        sqlx::query(
            "INSERT INTO face_detections (photo_id, user_id, cluster_id, bbox_x, bbox_y, bbox_w, bbox_h, confidence, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(photo)
        .bind(user)
        .bind(cluster)
        .bind(bbox.0)
        .bind(bbox.1)
        .bind(bbox.2)
        .bind(bbox.3)
        .bind(confidence)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn face_clusters_carry_representative_bbox() {
        let pool = memory_pool().await;
        let u = "user-1";
        insert_cluster(&pool, 1, u, 2).await;
        // Highest-confidence detection (0.9) on photo "p1" is the representative;
        // its bbox must be the one surfaced, not the lower-confidence one.
        insert_detection(&pool, u, "p1", 1, (0.1, 0.2, 0.3, 0.4), 0.9).await;
        insert_detection(&pool, u, "p2", 1, (0.5, 0.5, 0.1, 0.1), 0.5).await;

        let clusters = fetch_face_clusters(&pool, u).await.unwrap();
        assert_eq!(clusters.len(), 1);
        let c = &clusters[0];
        assert_eq!(c.representative.as_deref(), Some("p1"));
        assert_eq!(c.rep_bbox_x, Some(0.1));
        assert_eq!(c.rep_bbox_y, Some(0.2));
        assert_eq!(c.rep_bbox_w, Some(0.3));
        assert_eq!(c.rep_bbox_h, Some(0.4));
    }

    #[tokio::test]
    async fn cluster_below_two_photos_hidden_and_missing_bbox_is_null() {
        let pool = memory_pool().await;
        let u = "user-1";
        // Single-photo cluster is filtered out (photo_count < 2).
        insert_cluster(&pool, 1, u, 1).await;
        insert_detection(&pool, u, "p1", 1, (0.1, 0.1, 0.2, 0.2), 0.8).await;
        // Eligible cluster with no linked detections → representative + bbox NULL.
        insert_cluster(&pool, 2, u, 2).await;

        let clusters = fetch_face_clusters(&pool, u).await.unwrap();
        assert_eq!(clusters.len(), 1, "only the >=2-photo cluster surfaces");
        assert_eq!(clusters[0].id, 2);
        assert_eq!(clusters[0].rep_bbox_x, None);
    }
}
