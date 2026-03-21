//! Tag management handlers.
//!
//! Provides CRUD for per-photo tags and a full-text search endpoint that
//! matches against both tag names and filenames. Tags for search results
//! are batch-loaded in a single `WHERE photo_id IN (...)` query.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::*;

/// GET /api/tags — list all unique tags for the current user
pub async fn list_tags(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<TagListResponse>, AppError> {
    let tags: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT tag FROM photo_tags WHERE user_id = ? ORDER BY tag ASC")
            .bind(&auth.user_id)
            .fetch_all(&state.read_pool)
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
    .fetch_all(&state.read_pool)
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
    let tag = sanitize::sanitize_text(&body.tag).to_lowercase();
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
    let tag = sanitize::sanitize_text(&body.tag).to_lowercase();
    sqlx::query("DELETE FROM photo_tags WHERE photo_id = ? AND user_id = ? AND tag = ?")
        .bind(&photo_id)
        .bind(&auth.user_id)
        .bind(&tag)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/search — search photos by tag, filename, date, location, or media type
/// Supports multi-word fuzzy search: each word must match at least one field.
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

    // Tokenize query into individual words for multi-word matching
    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();

    if tokens.is_empty() {
        return Ok(Json(SearchResponse { results: vec![] }));
    }

    // Build dynamic SQL: each token must match at least one searchable field.
    // We use a concatenated "searchable text" approach per-row so each token
    // can match any field independently.
    //
    // For fuzzy matching we also generate "stem" variants (strip common suffixes)
    // and allow partial substring matches via LIKE.

    let field_expr = "COALESCE(pt.tag, '') || ' ' || COALESCE(p.filename, '') || ' ' || \
        COALESCE(p.file_path, '') || ' ' || COALESCE(p.media_type, '') || ' ' || \
        COALESCE(p.taken_at, '') || ' ' || COALESCE(p.created_at, '') || ' ' || \
        COALESCE(CAST(p.latitude AS TEXT), '') || ' ' || COALESCE(CAST(p.longitude AS TEXT), '') || ' ' || \
        COALESCE(sa.name, '')";

    // Each token: the concatenated text must LIKE %token%
    let mut where_clauses = Vec::new();
    let mut bind_values: Vec<String> = vec![auth.user_id.clone()];

    for token in &tokens {
        // Generate fuzzy variants: the original token, plus common stem variants
        let mut variants: Vec<String> = vec![token.clone()];

        // Strip common English suffixes for basic stemming
        if token.len() > 4 {
            if let Some(stem) = token.strip_suffix("ing") {
                variants.push(stem.to_string());
                // "running" -> "run" but also "running" -> "runn"
                if stem.len() > 2
                    && stem.ends_with(|c: char| c == stem.chars().last().unwrap_or(' '))
                {
                    variants.push(stem[..stem.len() - 1].to_string());
                }
            }
            if let Some(stem) = token.strip_suffix("ed") {
                variants.push(stem.to_string());
            }
            if let Some(stem) = token.strip_suffix("es") {
                variants.push(stem.to_string());
            } else if let Some(stem) = token.strip_suffix('s') {
                variants.push(stem.to_string());
            }
            if let Some(stem) = token.strip_suffix("tion") {
                variants.push(format!("{}t", stem));
            }
            if let Some(stem) = token.strip_suffix("ly") {
                variants.push(stem.to_string());
            }
        }

        // Build OR conditions for each variant of this token
        let variant_conditions: Vec<String> = variants
            .iter()
            .map(|v| {
                bind_values.push(format!("%{}%", sanitize::escape_like(v)));
                format!("LOWER({}) LIKE ? ESCAPE '\\'", field_expr)
            })
            .collect();

        where_clauses.push(format!("({})", variant_conditions.join(" OR ")));
    }

    let sql = format!(
        "SELECT DISTINCT p.id, p.filename, p.media_type, p.mime_type, p.thumb_path,
                p.created_at, p.taken_at, p.latitude, p.longitude, p.width, p.height
         FROM photos p
         LEFT JOIN photo_tags pt ON pt.photo_id = p.id AND pt.user_id = p.user_id
         LEFT JOIN shared_album_photos sap ON sap.photo_ref = p.id AND sap.ref_type = 'photo'
         LEFT JOIN shared_albums sa ON sa.id = sap.album_id
         WHERE p.user_id = ?
           AND {}
         ORDER BY p.created_at DESC
         LIMIT {}",
        where_clauses.join(" AND "),
        limit
    );

    // Build the query with dynamic bindings
    let mut q = sqlx::query_as::<_, SearchRow>(&sql);
    for val in &bind_values {
        q = q.bind(val);
    }

    let rows: Vec<SearchRow> = q.fetch_all(&state.read_pool).await?;

    // Batch-load tags for all results in a single query (avoids N+1).
    // Build a dynamic `WHERE photo_id IN (?, ?, ...)` clause.
    let photo_ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let mut tags_by_photo: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    if !photo_ids.is_empty() {
        let placeholders = photo_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let tags_sql = format!(
            "SELECT photo_id, tag FROM photo_tags WHERE photo_id IN ({}) AND user_id = ? ORDER BY tag",
            placeholders
        );
        let mut tags_q = sqlx::query_as::<_, (String, String)>(&tags_sql);
        for pid in &photo_ids {
            tags_q = tags_q.bind(pid);
        }
        tags_q = tags_q.bind(&auth.user_id);

        let tag_rows: Vec<(String, String)> = tags_q.fetch_all(&state.read_pool).await?;
        for (pid, tag) in tag_rows {
            tags_by_photo.entry(pid).or_default().push(tag);
        }
    }

    let results: Vec<SearchResult> = rows
        .into_iter()
        .map(|row| {
            let tags = tags_by_photo.remove(&row.id).unwrap_or_default();
            SearchResult {
                id: row.id,
                filename: row.filename,
                media_type: row.media_type,
                mime_type: row.mime_type,
                thumb_path: row.thumb_path,
                created_at: row.created_at,
                taken_at: row.taken_at,
                latitude: row.latitude,
                longitude: row.longitude,
                width: row.width,
                height: row.height,
                tags,
            }
        })
        .collect();

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
    taken_at: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    width: Option<i32>,
    height: Option<i32>,
}
