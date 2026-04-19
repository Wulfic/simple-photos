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
    let user_enabled: Option<(String,)> = sqlx::query_as(
        "SELECT value FROM user_settings WHERE user_id = ?1 AND key = 'ai_enabled'"
    )
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let enabled = match user_enabled {
        Some((val,)) => val != "false",
        None => config.enabled,
    };

    // Count processed and pending photos
    let processed: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ai_processed_photos WHERE user_id = ?1"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let total: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM photos WHERE user_id = ?1 AND file_path IS NOT NULL"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let face_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM face_detections WHERE user_id = ?1"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let cluster_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT cluster_id) FROM face_detections WHERE user_id = ?1 AND cluster_id IS NOT NULL"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let object_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM object_detections WHERE user_id = ?1"
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(AiStatusResponse {
        enabled,
        gpu_available: false, // Will be updated when GPU detection is implemented
        photos_processed: processed.0,
        photos_pending: total.0 - processed.0,
        face_detections: face_count.0,
        face_clusters: cluster_count.0,
        object_detections: object_count.0,
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
         ON CONFLICT(user_id, key) DO UPDATE SET value = ?2, updated_at = datetime('now')"
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
                sqlx::query(
                    "DELETE FROM face_detections WHERE photo_id = ?1 AND user_id = ?2"
                )
                .bind(id)
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

                sqlx::query(
                    "DELETE FROM object_detections WHERE photo_id = ?1 AND user_id = ?2"
                )
                .bind(id)
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await?;

                // Remove from processed list so the background processor picks it up
                let result = sqlx::query(
                    "DELETE FROM ai_processed_photos WHERE photo_id = ?1 AND user_id = ?2"
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
                "DELETE FROM photo_tags WHERE user_id = ?1 AND (tag LIKE 'person:%' OR tag LIKE 'object:%')"
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
    let clusters: Vec<FaceClusterSummary> = sqlx::query_as(
        "SELECT fc.id, fc.label, fc.photo_count, fc.representative, fc.created_at, fc.updated_at \
         FROM face_clusters fc \
         WHERE fc.user_id = ?1 \
         ORDER BY fc.photo_count DESC"
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(clusters))
}

/// GET /api/ai/faces/:cluster_id/photos — list photos in a face cluster.
pub async fn list_cluster_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cluster_id): Path<i64>,
) -> Result<Json<Vec<FaceDetectionRecord>>, AppError> {
    // Verify cluster belongs to user
    let _cluster: (i64,) = sqlx::query_as(
        "SELECT id FROM face_clusters WHERE id = ?1 AND user_id = ?2"
    )
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
         ORDER BY fd.confidence DESC"
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
         WHERE id = ?2 AND user_id = ?3"
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
        return Err(AppError::BadRequest("Need at least 2 clusters to merge".into()));
    }

    // Verify all clusters belong to user
    let target_id = body.cluster_ids[0];
    for cid in &body.cluster_ids {
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM face_clusters WHERE id = ?1 AND user_id = ?2"
        )
        .bind(cid)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;

        if exists.is_none() {
            return Err(AppError::BadRequest(format!("Cluster {} not found", cid)));
        }
    }

    // Move all face detections to the target cluster
    for cid in &body.cluster_ids[1..] {
        sqlx::query(
            "UPDATE face_detections SET cluster_id = ?1 WHERE cluster_id = ?2 AND user_id = ?3"
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
        "UPDATE face_clusters SET photo_count = ?1, updated_at = datetime('now') WHERE id = ?2"
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
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM face_detections WHERE id = ?1 AND user_id = ?2"
        )
        .bind(did)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;

        if exists.is_none() {
            return Err(AppError::BadRequest(format!("Detection {} not found", did)));
        }
    }

    // Create a new cluster
    let result = sqlx::query(
        "INSERT INTO face_clusters (user_id, photo_count, created_at, updated_at) \
         VALUES (?1, ?2, datetime('now'), datetime('now'))"
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
        let old: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT cluster_id FROM face_detections WHERE id = ?1"
        )
        .bind(did)
        .fetch_optional(&state.pool)
        .await?;

        if let Some((Some(old_cid),)) = old {
            old_cluster_ids.insert(old_cid);
        }

        sqlx::query(
            "UPDATE face_detections SET cluster_id = ?1 WHERE id = ?2 AND user_id = ?3"
        )
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
         ORDER BY photo_count DESC"
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
         ORDER BY confidence DESC"
    )
    .bind(&auth.user_id)
    .bind(&class_name)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(detections))
}
