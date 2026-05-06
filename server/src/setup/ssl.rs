//! SSL/TLS configuration endpoints.
//!
//! These endpoints let admins view and update TLS settings (enable/disable,
//! set certificate and key paths, or fully automate issuance via
//! Let's Encrypt). Changes are persisted to config.toml.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::admin::require_admin;
use super::letsencrypt::{self, ProvisionRequest};

// ── Response / Request types ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SslStatusResponse {
    pub enabled: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub message: String,
    /// When a Let's Encrypt cert has been provisioned, surfaces the
    /// `[tls.letsencrypt]` block so the UI can show "Auto-renewing for
    /// {domain}" instead of the manual-cert paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub letsencrypt: Option<crate::config::LetsEncryptConfig>,
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
        letsencrypt: tls.letsencrypt.clone(),
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

    // Persist to config.toml (blocking I/O — offload to spawn_blocking)
    let ssl_enabled = req.enabled;
    let ssl_cert = req.cert_path.clone();
    let ssl_key = req.key_path.clone();
    tokio::task::spawn_blocking(move || {
        update_config_toml_ssl(ssl_enabled, ssl_cert.as_deref(), ssl_key.as_deref())
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))??;

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
        letsencrypt: state.config.tls.letsencrypt.clone(),
    }))
}

/// Persist TLS settings to config.toml.
fn update_config_toml_ssl(
    enabled: bool,
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<(), AppError> {
    let config_path =
        std::env::var("SIMPLE_PHOTOS_CONFIG").unwrap_or_else(|_| "config.toml".into());

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

// ── Let's Encrypt provisioning ────────────────────────────────────

/// Request body for `POST /api/admin/ssl/letsencrypt`.
///
/// `dry_run` lets callers (and the wizard) validate inputs locally without
/// contacting the Let's Encrypt CA — useful for surfacing form errors
/// before the real submit (and burning rate-limit budget).
#[derive(Debug, Deserialize)]
pub struct LetsEncryptRequest {
    pub domain: String,
    pub email: String,
    pub agree_tos: bool,
    #[serde(default)]
    pub staging: bool,
    #[serde(default = "default_le_port")]
    pub challenge_port: u16,
    /// When `true`, validate inputs and return early without contacting
    /// the CA.
    #[serde(default)]
    pub dry_run: bool,
}

fn default_le_port() -> u16 {
    80
}

#[derive(Debug, Serialize)]
pub struct LetsEncryptResponse {
    pub success: bool,
    pub dry_run: bool,
    pub domain: String,
    pub staging: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub message: String,
}

/// POST /api/admin/ssl/letsencrypt — Provision a Let's Encrypt certificate.
///
/// Drives the full ACME-v2 handshake (HTTP-01 challenge), atomically writes
/// `fullchain.pem` and `privkey.pem` under `data/letsencrypt/{domain}/`, and
/// persists the new paths plus `[tls.letsencrypt]` metadata to config.toml.
///
/// A server restart is required for the new certificate to take effect —
/// `axum-server` does not hot-reload PEM files.
pub async fn provision_letsencrypt(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<LetsEncryptRequest>,
) -> Result<Json<LetsEncryptResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let provision_req = ProvisionRequest {
        domain: req.domain.clone(),
        email: req.email.clone(),
        agree_tos: req.agree_tos,
        staging: req.staging,
        challenge_port: req.challenge_port,
    };

    // Always validate inputs first — cheap, no network, surfaces UI errors.
    letsencrypt::validate_inputs(&provision_req)?;

    if req.dry_run {
        return Ok(Json(LetsEncryptResponse {
            success: true,
            dry_run: true,
            domain: provision_req.domain.trim().to_string(),
            staging: provision_req.staging,
            cert_path: None,
            key_path: None,
            message: "Inputs accepted. Submit again with dry_run=false to contact Let's Encrypt.".into(),
        }));
    }

    // Audit-log the *attempt* before contacting the CA — makes it possible
    // to correlate a failed provisioning with the operator who tried it.
    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "letsencrypt_provision_started",
            "domain": provision_req.domain,
            "staging": provision_req.staging,
            "challenge_port": provision_req.challenge_port,
        })),
    )
    .await;

    // Honour an optional ACME directory URL override (used by E2E tests
    // pointing at a Pebble instance).  Production callers leave it unset.
    let directory_override = std::env::var("SIMPLE_PHOTOS_ACME_DIRECTORY_URL").ok();

    // The data root for issued certs.  Lives next to the SQLite DB so
    // operator backups already cover it.
    let data_root = std::path::PathBuf::from("data/letsencrypt");

    let outcome = letsencrypt::provision_certificate(
        &provision_req,
        &data_root,
        directory_override.as_deref(),
    )
    .await?;

    // Persist new TLS settings + LE metadata.
    let cert_path_clone = outcome.cert_path.clone();
    let key_path_clone = outcome.key_path.clone();
    let domain_clone = outcome.domain.clone();
    let email_clone = req.email.trim().to_string();
    let staging_flag = req.staging;
    let challenge_port = req.challenge_port;
    let issued_at = outcome.issued_at.clone();
    tokio::task::spawn_blocking(move || {
        update_config_toml_ssl(true, Some(&cert_path_clone), Some(&key_path_clone))?;
        update_config_toml_letsencrypt(&domain_clone, &email_clone, staging_flag, challenge_port, &issued_at)
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))??;

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "letsencrypt_provision_success",
            "domain": outcome.domain,
            "staging": outcome.staging,
        })),
    )
    .await;

    Ok(Json(LetsEncryptResponse {
        success: true,
        dry_run: false,
        domain: outcome.domain,
        staging: outcome.staging,
        cert_path: Some(outcome.cert_path),
        key_path: Some(outcome.key_path),
        message: "Certificate issued. Restart the server to begin serving HTTPS.".into(),
    }))
}

/// Persist `[tls.letsencrypt]` metadata to config.toml.
fn update_config_toml_letsencrypt(
    domain: &str,
    email: &str,
    staging: bool,
    challenge_port: u16,
    issued_at: &str,
) -> Result<(), AppError> {
    let config_path =
        std::env::var("SIMPLE_PHOTOS_CONFIG").unwrap_or_else(|_| "config.toml".into());

    let contents = std::fs::read_to_string(&config_path)
        .map_err(|e| AppError::Internal(format!("Failed to read config file: {}", e)))?;

    let mut doc: toml::Table = contents
        .parse()
        .map_err(|e| AppError::Internal(format!("Failed to parse config TOML: {}", e)))?;

    let tls_table = doc
        .entry("tls")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| AppError::Internal("[tls] is not a table in config.toml".into()))?;

    let mut le_table = toml::Table::new();
    le_table.insert("domain".into(), toml::Value::String(domain.into()));
    le_table.insert("email".into(), toml::Value::String(email.into()));
    le_table.insert("staging".into(), toml::Value::Boolean(staging));
    le_table.insert(
        "challenge_port".into(),
        toml::Value::Integer(challenge_port as i64),
    );
    le_table.insert(
        "last_issued_at".into(),
        toml::Value::String(issued_at.into()),
    );
    tls_table.insert("letsencrypt".into(), toml::Value::Table(le_table));

    let output = toml::to_string_pretty(&doc)
        .map_err(|e| AppError::Internal(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(&config_path, output)
        .map_err(|e| AppError::Internal(format!("Failed to write config file: {}", e)))?;

    Ok(())
}
