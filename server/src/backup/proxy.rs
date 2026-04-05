//! Backup view proxy endpoints.
//!
//! These endpoints let the admin UI browse and preview photos on a remote
//! backup server without running into CORS restrictions — the request is
//! proxied through this server.

use axum::extract::{Path, State};
use axum::Json;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::*;

// ── Backup View Proxy ────────────────────────────────────────────────────────

/// GET /api/admin/backup/servers/:id/photos
/// Proxy endpoint: fetches the photo list from a backup server and returns it.
/// Used by the frontend to show a backup view of photos.
pub async fn proxy_backup_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<Json<Vec<BackupPhotoRecord>>, AppError> {
    require_admin(&state, &auth).await?;

    let address: String = sqlx::query_scalar("SELECT address FROM backup_servers WHERE id = ?")
        .bind(&server_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;

    let api_key: Option<String> =
        sqlx::query_scalar("SELECT api_key FROM backup_servers WHERE id = ?")
            .bind(&server_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::BadRequest(format!("HTTP client error: {}", e)))?;

    let mut req = client.get(format!("http://{}/api/backup/list", address));
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            let photos: Vec<BackupPhotoRecord> = resp.json().await.map_err(|e| {
                AppError::BadRequest(format!("Failed to parse backup server response: {}", e))
            })?;
            Ok(Json(photos))
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                "Backup server at {} returned HTTP {}: {}",
                address,
                status,
                body
            );
            Err(AppError::BadRequest(format!(
                "Backup server returned HTTP {}",
                status
            )))
        }
        Err(e) => {
            tracing::warn!("Failed to connect to backup server at {}: {}", address, e);
            Err(AppError::BadRequest(format!(
                "Cannot reach backup server at {}: {}",
                address, e
            )))
        }
    }
}

/// GET /api/admin/backup/servers/:id/photos/:photo_id/thumb
/// Proxy endpoint: streams a thumbnail from a backup server through this
/// server to the browser, avoiding CORS restrictions.
pub async fn proxy_backup_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((server_id, photo_id)): Path<(String, String)>,
) -> Result<axum::response::Response, AppError> {
    require_admin(&state, &auth).await?;

    let address: String = sqlx::query_scalar("SELECT address FROM backup_servers WHERE id = ?")
        .bind(&server_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;

    let api_key: Option<String> =
        sqlx::query_scalar("SELECT api_key FROM backup_servers WHERE id = ?")
            .bind(&server_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::BadRequest(format!("HTTP client error: {}", e)))?;

    let url = format!("http://{}/api/backup/download/{}/thumb", address, photo_id);
    let mut req = client.get(&url);
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/jpeg")
                .to_string();

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("Failed to read thumbnail: {}", e)))?;

            Ok(axum::response::Response::builder()
                .status(axum::http::StatusCode::OK)
                .header("Content-Type", content_type)
                .header("Cache-Control", "private, max-age=3600")
                .body(axum::body::Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?)
        }
        Ok(resp) if resp.status() == axum::http::StatusCode::NOT_FOUND => Err(AppError::NotFound),
        Ok(resp) => Err(AppError::BadRequest(format!(
            "Backup server returned HTTP {} for thumbnail",
            resp.status()
        ))),
        Err(e) => Err(AppError::BadRequest(format!(
            "Cannot reach backup server for thumbnail: {}",
            e
        ))),
    }
}
