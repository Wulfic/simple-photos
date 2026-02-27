use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::*;

/// GET /api/tags — list all unique tags for the current user
pub async fn list_tags(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<TagListResponse>, AppError> {
    let tags: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT tag FROM photo_tags WHERE user_id = ? ORDER BY tag ASC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(TagListResponse {
        tags: tags.into_iter().map(|(t,)| t).collect(),
    }))
}

/// GET /api/photos/:id/tags — list tags on a specific photo
pub async fn get_photo_tags(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<PhotoTagsResponse>, AppError> {
    let tags: Vec<(String,)> = sqlx::query_as(
        "SELECT tag FROM photo_tags WHERE photo_id = ? AND user_id = ? ORDER BY tag ASC",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(PhotoTagsResponse {
        photo_id,
        tags: tags.into_iter().map(|(t,)| t).collect(),
    }))
}

/// POST /api/photos/:id/tags — add a tag to a photo
pub async fn add_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(body): Json<AddTagRequest>,
) -> Result<StatusCode, AppError> {
    let tag = body.tag.trim().to_lowercase();
    if tag.is_empty() || tag.len() > 100 {
        return Err(AppError::BadRequest("Tag must be 1-100 characters".into()));
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT OR IGNORE INTO photo_tags (photo_id, user_id, tag, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&tag)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::CREATED)
}

/// DELETE /api/photos/:id/tags — remove a tag from a photo
pub async fn remove_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(body): Json<RemoveTagRequest>,
) -> Result<StatusCode, AppError> {
    let tag = body.tag.trim().to_lowercase();
    sqlx::query(
        "DELETE FROM photo_tags WHERE photo_id = ? AND user_id = ? AND tag = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&tag)
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/search — search photos by tag (and optionally filename)
pub async fn search_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, AppError> {
    let query = params.q.trim().to_lowercase();
    let limit = params.limit.unwrap_or(100).min(500);

    if query.is_empty() {
        return Ok(Json(SearchResponse { results: vec![] }));
    }

    // Search both plain photos and blobs by matching tags OR filename
    let like_pattern = format!("%{}%", query);

    // Search plain-mode photos that match by tag or filename
    let rows: Vec<SearchRow> = sqlx::query_as(
        "SELECT DISTINCT p.id, p.filename, p.media_type, p.mime_type, p.thumb_path, p.created_at
         FROM photos p
         LEFT JOIN photo_tags pt ON pt.photo_id = p.id AND pt.user_id = p.user_id
         WHERE p.user_id = ? AND p.encrypted_blob_id IS NULL
           AND (pt.tag LIKE ? OR p.filename LIKE ?)
         ORDER BY p.created_at DESC
         LIMIT ?",
    )
    .bind(&auth.user_id)
    .bind(&like_pattern)
    .bind(&like_pattern)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    // Gather tags for each result
    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        let tags: Vec<(String,)> = sqlx::query_as(
            "SELECT tag FROM photo_tags WHERE photo_id = ? AND user_id = ? ORDER BY tag",
        )
        .bind(&row.id)
        .bind(&auth.user_id)
        .fetch_all(&state.pool)
        .await?;

        results.push(SearchResult {
            id: row.id,
            filename: row.filename,
            media_type: row.media_type,
            mime_type: row.mime_type,
            thumb_path: row.thumb_path,
            created_at: row.created_at,
            tags: tags.into_iter().map(|(t,)| t).collect(),
        });
    }

    Ok(Json(SearchResponse { results }))
}

#[derive(Debug, sqlx::FromRow)]
struct SearchRow {
    id: String,
    filename: String,
    media_type: String,
    mime_type: String,
    thumb_path: Option<String>,
    created_at: String,
}
