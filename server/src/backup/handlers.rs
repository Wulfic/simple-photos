//! Backup server management endpoints — CRUD for registered backup destinations.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::*;

// ── Backup Server Management ─────────────────────────────────────────────────

/// GET /api/admin/backup/servers
/// List all configured backup servers (admin only).
pub async fn list_backup_servers(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<BackupServerListResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let servers = sqlx::query_as::<_, BackupServer>(
        "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
         last_sync_status, last_sync_error, enabled, created_at \
         FROM backup_servers ORDER BY created_at ASC",
    )
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(BackupServerListResponse { servers }))
}

/// POST /api/admin/backup/servers
/// Add a new backup server (admin only).
pub async fn add_backup_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<AddBackupServerRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    require_admin(&state, &auth).await?;

    // Validate and sanitize inputs
    let address = req.address.trim().to_string();
    if address.is_empty() {
        return Err(AppError::BadRequest("Address is required".into()));
    }
    let name = sanitize::sanitize_display_name(&req.name, 200)
        .map_err(|reason| AppError::BadRequest(reason.into()))?;

    // Check for duplicates
    let exists: bool =
        sqlx::query_scalar("SELECT COUNT(*) > 0 FROM backup_servers WHERE address = ?")
            .bind(&address)
            .fetch_one(&state.read_pool)
            .await?;

    if exists {
        return Err(AppError::Conflict(format!(
            "Backup server at {} already exists",
            address
        )));
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let freq = req.sync_frequency_hours.unwrap_or(24).max(1);

    sqlx::query(
        "INSERT INTO backup_servers (id, name, address, api_key, sync_frequency_hours, \
         last_sync_status, enabled, created_at) \
         VALUES (?, ?, ?, ?, ?, 'never', 1, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&address)
    .bind(&req.api_key)
    .bind(freq)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    tracing::info!("Added backup server '{}' at {}", name, address);

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "name": name,
            "address": address,
            "sync_frequency_hours": freq,
        })),
    ))
}

/// PUT /api/admin/backup/servers/:id
/// Update a backup server's configuration (admin only).
pub async fn update_backup_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
    Json(req): Json<UpdateBackupServerRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // Verify server exists
    let exists: bool = sqlx::query_scalar("SELECT COUNT(*) > 0 FROM backup_servers WHERE id = ?")
        .bind(&server_id)
        .fetch_one(&state.read_pool)
        .await?;

    if !exists {
        return Err(AppError::NotFound);
    }

    // Validate name outside the transaction (sanitize may reject input).
    let safe_name = req
        .name
        .as_ref()
        .map(|n| sanitize::sanitize_display_name(n, 200))
        .transpose()
        .map_err(|reason| AppError::BadRequest(reason.into()))?;

    // Transaction: apply all field updates atomically so a mid-request
    // failure can't leave the row in a half-updated state.
    let mut tx = state.pool.begin().await?;

    if let Some(ref name) = safe_name {
        sqlx::query("UPDATE backup_servers SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(ref address) = req.address {
        sqlx::query("UPDATE backup_servers SET address = ? WHERE id = ?")
            .bind(address.trim())
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(ref api_key) = req.api_key {
        sqlx::query("UPDATE backup_servers SET api_key = ? WHERE id = ?")
            .bind(api_key)
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(freq) = req.sync_frequency_hours {
        sqlx::query("UPDATE backup_servers SET sync_frequency_hours = ? WHERE id = ?")
            .bind(freq.max(1))
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(enabled) = req.enabled {
        sqlx::query("UPDATE backup_servers SET enabled = ? WHERE id = ?")
            .bind(enabled)
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    Ok(Json(serde_json::json!({
        "message": "Backup server updated",
        "id": server_id,
    })))
}

/// DELETE /api/admin/backup/servers/:id
/// Remove a backup server (admin only).
pub async fn remove_backup_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<StatusCode, AppError> {
    require_admin(&state, &auth).await?;

    let result = sqlx::query("DELETE FROM backup_servers WHERE id = ?")
        .bind(&server_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    tracing::info!("Removed backup server {}", server_id);

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/admin/backup/servers/:id/status
/// Check if a backup server is reachable and get its version.
pub async fn check_backup_server_status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<Json<BackupServerStatus>, AppError> {
    require_admin(&state, &auth).await?;

    let address: String = sqlx::query_scalar("SELECT address FROM backup_servers WHERE id = ?")
        .bind(&server_id)
        .fetch_optional(&state.read_pool)
        .await?
        .ok_or(AppError::NotFound)?;

    // Try to reach the server's health endpoint
    let url = format!("http://{}/health", address);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    match client.get(&url).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let version = body
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                Ok(Json(BackupServerStatus {
                    reachable: true,
                    version: Some(version),
                    error: None,
                }))
            } else {
                Ok(Json(BackupServerStatus {
                    reachable: false,
                    version: None,
                    error: Some(format!("Server responded with status {}", resp.status())),
                }))
            }
        }
        Err(e) => Ok(Json(BackupServerStatus {
            reachable: false,
            version: None,
            error: Some(format!("Connection failed: {}", e)),
        })),
    }
}
