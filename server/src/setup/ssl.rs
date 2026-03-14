//! SSL/TLS configuration endpoints.
//!
//! These endpoints let admins view and update TLS settings (enable/disable,
//! set certificate and key paths). Changes are persisted to config.toml.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::admin::require_admin;

// ── Response / Request types ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SslStatusResponse {
    pub enabled: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSslRequest {
    pub enabled: bool,
    #[serde(default)]
    pub cert_path: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// GET /api/admin/ssl — Get current TLS configuration.
pub async fn get_ssl(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<SslStatusResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let tls = &state.config.tls;
    Ok(Json(SslStatusResponse {
        enabled: tls.enabled,
        cert_path: tls.cert_path.clone(),
        key_path: tls.key_path.clone(),
        message: if tls.enabled {
            "TLS is enabled. A server restart is needed for changes to take effect.".into()
        } else {
            "TLS is disabled. The server is running on plain HTTP.".into()
        },
    }))
}

/// PUT /api/admin/ssl — Update TLS configuration.
///
/// Persists changes to config.toml. A server restart is required for
/// TLS changes to take effect.
pub async fn update_ssl(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<UpdateSslRequest>,
) -> Result<Json<SslStatusResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Validate: if enabling, both cert_path and key_path must be provided
    if req.enabled {
        let cert = req.cert_path.as_deref().unwrap_or("");
        let key = req.key_path.as_deref().unwrap_or("");

        if cert.is_empty() || key.is_empty() {
            return Err(AppError::BadRequest(
                "Both certificate path and key path are required when enabling TLS".into(),
            ));
        }

        // Verify files exist
        if !std::path::Path::new(cert).exists() {
            return Err(AppError::BadRequest(format!(
                "Certificate file not found: {}",
                cert
            )));
        }
        if !std::path::Path::new(key).exists() {
            return Err(AppError::BadRequest(format!(
                "Private key file not found: {}",
                key
            )));
        }
    }

    // Persist to config.toml
    update_config_toml_ssl(req.enabled, req.cert_path.as_deref(), req.key_path.as_deref())?;

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "tls_enabled": req.enabled,
            "cert_path": req.cert_path,
        })),
    )
    .await;

    tracing::info!(
        "TLS configuration updated: enabled={}, cert={:?}",
        req.enabled,
        req.cert_path
    );

    Ok(Json(SslStatusResponse {
        enabled: req.enabled,
        cert_path: req.cert_path,
        key_path: req.key_path,
        message: "TLS configuration updated. Restart the server for changes to take effect.".into(),
    }))
}

/// Persist TLS settings to config.toml.
fn update_config_toml_ssl(
    enabled: bool,
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<(), AppError> {
    let config_path = std::env::var("SIMPLE_PHOTOS_CONFIG").unwrap_or_else(|_| "config.toml".into());

    let contents = std::fs::read_to_string(&config_path)
        .map_err(|e| AppError::Internal(format!("Failed to read config file: {}", e)))?;

    let mut doc: toml::Table = contents
        .parse()
        .map_err(|e| AppError::Internal(format!("Failed to parse config TOML: {}", e)))?;

    // Create or update the [tls] section
    let tls_table = doc
        .entry("tls")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| AppError::Internal("[tls] is not a table in config.toml".into()))?;

    tls_table.insert("enabled".into(), toml::Value::Boolean(enabled));

    if let Some(cert) = cert_path {
        tls_table.insert("cert_path".into(), toml::Value::String(cert.into()));
    }
    if let Some(key) = key_path {
        tls_table.insert("key_path".into(), toml::Value::String(key.into()));
    }

    // If disabling, we can remove paths or leave them — leave them for easy re-enable
    let output = toml::to_string_pretty(&doc)
        .map_err(|e| AppError::Internal(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(&config_path, output)
        .map_err(|e| AppError::Internal(format!("Failed to write config file: {}", e)))?;

    Ok(())
}
