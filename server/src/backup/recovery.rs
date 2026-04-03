//! Disaster-recovery endpoints.
//!
//! `POST /api/admin/backup/servers/:id/recover` — pull ALL photos from a
//! backup server and re‐register them locally (admin only).
//! The actual recovery engine lives in [`super::recovery_engine`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::*;
use super::recovery_engine::run_recovery;
use super::sync::try_acquire_sync;

// ── Recovery ─────────────────────────────────────────────────────────────────

/// POST /api/admin/backup/servers/:id/recover
/// Recover all photos from a backup server that don't already exist locally.
/// Runs as a background task; returns immediately with a recovery_id.
///
/// This endpoint:
/// 1. Connects to the backup server's /api/backup/list endpoint
/// 2. Deduplicates by **photo ID** against the local `photos` table
/// 3. Downloads any missing photos and stores them locally
/// 4. Registers them in the photos table (INSERT OR IGNORE for idempotency)
pub async fn recover_from_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<(StatusCode, Json<RecoveryResponse>), AppError> {
    require_admin(&state, &auth).await?;

    // Fetch backup server details
    let server = sqlx::query_as::<_, BackupServer>(
        "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
         last_sync_status, last_sync_error, enabled, created_at \
         FROM backup_servers WHERE id = ?",
    )
    .bind(&server_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // Fetch the API key for the remote server
    let api_key: Option<String> =
        sqlx::query_scalar("SELECT api_key FROM backup_servers WHERE id = ?")
            .bind(&server_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();

    // Create a sync log entry for tracking
    let recovery_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let server_name = server.name.clone();

    sqlx::query(
        "INSERT INTO backup_sync_log (id, server_id, started_at, status) \
         VALUES (?, ?, ?, 'recovering')",
    )
    .bind(&recovery_id)
    .bind(&server_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    // Prevent overlapping recoveries to the same server
    let guard = try_acquire_sync(&server_id).ok_or_else(|| {
        AppError::BadRequest("A sync or recovery is already in progress for this server".into())
    })?;

    // Spawn recovery as a background task (guard moves into the task
    // so the lock is held for the full duration and released on drop).
    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();
    let user_id = auth.user_id.clone();
    let recovery_id_clone = recovery_id.clone();

    tokio::spawn(async move {
        let _guard = guard; // hold lock until task completes
        run_recovery(
            &pool,
            &storage_root,
            &server,
            &api_key,
            &user_id,
            &recovery_id_clone,
        )
        .await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(RecoveryResponse {
            message: format!("Recovery from '{}' started", server_name),
            recovery_id,
        }),
    ))
}
