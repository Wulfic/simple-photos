//! Client diagnostic log ingestion.
//!
//! Mobile clients push batches of structured log entries here.
//! Inserts are best-effort: individual row failures are logged but
//! never surfaced to the caller so logging cannot disrupt backup.
//!
//! **Note:** `client_ts` is bound verbatim from the client payload.
//! It is not validated as a well-formed timestamp, so callers can
//! store arbitrary strings in that column.

use axum::extract::State;
use axum::Json;
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::{ClientLogBatch, ClientLogListResponse, ClientLogRecord};

/// POST /api/client-logs — receive a batch of diagnostic log entries from a
/// mobile client. Each entry is stored with the authenticated user's ID.
///
/// This is fire-and-forget from the client's perspective: partial insert
/// failures are logged server-side but never returned as errors to the
/// client (we don't want logging to interfere with the backup flow).
pub async fn submit_logs(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(batch): Json<ClientLogBatch>,
) -> Result<Json<serde_json::Value>, AppError> {
    let now = chrono::Utc::now().to_rfc3339();

    // Validate session_id
    if batch.session_id.is_empty() || batch.session_id.len() > 64 {
        return Err(AppError::BadRequest(
            "session_id must be 1-64 characters".into(),
        ));
    }

    // Cap batch size to prevent abuse
    if batch.entries.len() > 500 {
        return Err(AppError::BadRequest(
            "Maximum 500 log entries per batch".into(),
        ));
    }

    let valid_levels = ["debug", "info", "warn", "error"];
    let mut inserted: u32 = 0;

    for entry in &batch.entries {
        // Validate level
        let level = entry.level.to_lowercase();
        if !valid_levels.contains(&level.as_str()) {
            continue; // skip invalid entries silently
        }

        // Truncate excessively long messages and strip control chars
        let message = sanitize::sanitize_freeform(&entry.message, 4096);

        let tag = sanitize::sanitize_freeform(&entry.tag, 128);

        // Limit context JSON size to prevent storage abuse
        let context_str = entry
            .context
            .as_ref()
            .map(|c| {
                let s = c.to_string();
                if s.len() > 8192 {
                    // Truncate oversized JSON context
                    "{\"error\": \"context_truncated\"}".to_string()
                } else {
                    s
                }
            })
            .unwrap_or_else(|| "null".to_string());

        let id = Uuid::new_v4().to_string();

        let result = sqlx::query(
            "INSERT INTO client_logs (id, user_id, session_id, level, tag, message, context, client_ts, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&auth.user_id)
        .bind(&batch.session_id)
        .bind(&level)
        .bind(&tag)
        .bind(&message)
        .bind(&context_str)
        .bind(&entry.client_ts)
        .bind(&now)
        .execute(&state.pool)
        .await;

        match result {
            Ok(_) => inserted += 1,
            Err(e) => {
                tracing::warn!(
                    user_id = %auth.user_id,
                    session_id = %batch.session_id,
                    error = %e,
                    "Failed to insert client log entry"
                );
            }
        }
    }

    tracing::info!(
        user_id = %auth.user_id,
        session_id = %batch.session_id,
        count = inserted,
        "Received client diagnostic logs"
    );

    Ok(Json(json!({ "inserted": inserted })))
}

/// GET /api/admin/client-logs — list client diagnostic logs (admin only).
///
/// Query parameters:
///   - user_id: filter by user
///   - session_id: filter by session
///   - level: filter by level (debug/info/warn/error)
///   - after: cursor for pagination
///   - limit: max entries (default 100, max 500)
pub async fn list_logs(
    auth: AuthUser,
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<ListLogsParams>,
) -> Result<Json<ClientLogListResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let limit = params.limit.unwrap_or(100).min(500) as i64;

    // Build query dynamically based on filters
    let mut conditions = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref user_id) = params.user_id {
        conditions.push("user_id = ?");
        binds.push(user_id.clone());
    }
    if let Some(ref session_id) = params.session_id {
        conditions.push("session_id = ?");
        binds.push(session_id.clone());
    }
    if let Some(ref level) = params.level {
        conditions.push("level = ?");
        binds.push(level.clone());
    }
    if let Some(ref after) = params.after {
        conditions.push("created_at < ?");
        binds.push(after.clone());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT id, user_id, session_id, level, tag, message, context, client_ts, created_at \
         FROM client_logs {} ORDER BY created_at DESC LIMIT ?",
        where_clause
    );

    // We need to build the query dynamically with binds
    let mut query = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            String,
        ),
    >(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    query = query.bind(limit + 1); // fetch one extra to detect next page

    let rows = query.fetch_all(&state.read_pool).await?;

    let has_more = rows.len() as i64 > limit;
    let entries: Vec<ClientLogRecord> = rows
        .into_iter()
        .take(limit as usize)
        .map(
            |(id, user_id, session_id, level, tag, message, context, client_ts, created_at)| {
                let context_value = context.and_then(|c| serde_json::from_str(&c).ok());
                ClientLogRecord {
                    id,
                    user_id,
                    session_id,
                    level,
                    tag,
                    message,
                    context: context_value,
                    client_ts,
                    created_at: created_at.clone(),
                }
            },
        )
        .collect();

    let next_cursor = if has_more {
        entries.last().map(|e| e.created_at.clone())
    } else {
        None
    };

    Ok(Json(ClientLogListResponse {
        logs: entries,
        next_cursor,
    }))
}

/// Query parameters for listing client-submitted log entries with optional filters and pagination.
#[derive(Debug, serde::Deserialize)]
pub struct ListLogsParams {
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub level: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
}
