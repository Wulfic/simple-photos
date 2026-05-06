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
use super::local_ca::{self, GenerateRequest, LocalCaMeta};

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
    /// When a self-signed local CA has been generated, surfaces the
    /// `[tls.local_ca]` block so the UI can show the "Download CA bundle"
    /// button and the SHA-256 fingerprint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_ca: Option<crate::config::LocalCaConfig>,
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
        local_ca: tls.local_ca.clone(),
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
        local_ca: state.config.tls.local_ca.clone(),
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

// ── Self-signed Local CA ─────────────────────────────────────────────
//
// 4th TLS option offered by the wizard / settings panel.  Generates a
// local root CA + leaf cert under `data/local_ca/` and a downloadable
// install bundle clients can use to pre-trust the CA on Linux, Windows
// and Android.  No third-party CA, no public DNS, no inbound firewall
// rules — everything stays on the LAN.

#[derive(Debug, Default, Deserialize)]
pub struct LocalCaRequest {
    /// Optional friendly name embedded in the CA's CommonName.
    #[serde(default)]
    pub label: Option<String>,
    /// Extra DNS names / IPs to embed in the leaf certificate's SAN list.
    #[serde(default)]
    pub extra_hosts: Vec<String>,
    /// When `true`, validate inputs and return early without touching the
    /// filesystem — used by the wizard to surface form errors before the
    /// real submit.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct LocalCaResponse {
    pub success: bool,
    pub dry_run: bool,
    pub fingerprint_sha256: String,
    pub hosts: Vec<String>,
    pub generated_at: String,
    pub ca_expires_at: String,
    pub cert_expires_at: String,
    pub cert_path: String,
    pub key_path: String,
    pub bundle_url: String,
    pub message: String,
}

/// POST /api/admin/ssl/local-ca — Generate a self-signed local CA and leaf
/// certificate.  Atomically writes the new PEM files plus a download bundle
/// containing the public CA cert and per-platform install scripts, and
/// updates `[tls]` in `config.toml` to point the HTTPS listener at the
/// freshly issued leaf.  A server restart is required for the new cert
/// to take effect (axum-server does not hot-reload PEM files).
pub async fn provision_local_ca(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<LocalCaRequest>,
) -> Result<Json<LocalCaResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Validate inputs — cheap, no I/O.  Mirrors the LE dry_run path.
    validate_local_ca_inputs(&req)?;

    if req.dry_run {
        return Ok(Json(LocalCaResponse {
            success: true,
            dry_run: true,
            fingerprint_sha256: String::new(),
            hosts: Vec::new(),
            generated_at: String::new(),
            ca_expires_at: String::new(),
            cert_expires_at: String::new(),
            cert_path: String::new(),
            key_path: String::new(),
            bundle_url: String::new(),
            message: "Inputs accepted. Submit again with dry_run=false to generate the CA.".into(),
        }));
    }

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "local_ca_generate_started",
            "label": req.label,
            "extra_hosts": req.extra_hosts,
        })),
    )
    .await;

    let gen_req = GenerateRequest {
        label: req.label.clone(),
        extra_hosts: req.extra_hosts.clone(),
    };
    let data_root = std::path::PathBuf::from("data");
    let outcome = tokio::task::spawn_blocking(move || {
        local_ca::generate_local_ca(&gen_req, &data_root)
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))??;

    // Persist [tls] (paths + enabled) and [tls.local_ca] (metadata).
    let cert_path_clone = outcome.cert_path.clone();
    let key_path_clone = outcome.key_path.clone();
    let meta = LocalCaMeta {
        generated_at: outcome.generated_at.clone(),
        ca_expires_at: outcome.ca_expires_at.clone(),
        cert_expires_at: outcome.cert_expires_at.clone(),
        hosts: outcome.hosts.clone(),
        fingerprint_sha256: outcome.fingerprint_sha256.clone(),
    };
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        update_config_toml_ssl(true, Some(&cert_path_clone), Some(&key_path_clone))?;
        update_config_toml_local_ca(&meta)
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))??;

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "local_ca_generate_success",
            "fingerprint": outcome.fingerprint_sha256,
            "hosts": outcome.hosts,
        })),
    )
    .await;

    Ok(Json(LocalCaResponse {
        success: true,
        dry_run: false,
        fingerprint_sha256: outcome.fingerprint_sha256,
        hosts: outcome.hosts,
        generated_at: outcome.generated_at,
        ca_expires_at: outcome.ca_expires_at,
        cert_expires_at: outcome.cert_expires_at,
        cert_path: outcome.cert_path,
        key_path: outcome.key_path,
        bundle_url: "/api/admin/ssl/local-ca/bundle".into(),
        message: "Local CA generated. Download the bundle and install it on each client device, then restart the server to begin serving HTTPS.".into(),
    }))
}

/// GET /api/admin/ssl/local-ca/bundle — Download the install zip.
///
/// Streams `data/local_ca/simple-photos-ca-bundle.zip` (containing only
/// the public `ca.pem`, install scripts, and README) to the authenticated
/// admin.  Never serves private keys.
pub async fn download_local_ca_bundle(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<axum::response::Response, AppError> {
    use axum::body::Body;
    use axum::http::header;
    use axum::http::StatusCode;
    use axum::response::Response;

    require_admin(&state, &auth).await?;

    let data_root = std::path::PathBuf::from("data");
    let (path, bytes) = tokio::task::spawn_blocking(move || local_ca::read_bundle(&data_root))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))??;

    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "simple-photos-ca-bundle.zip".into());

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        )
        .header(header::CONTENT_LENGTH, bytes.len())
        // Prevent intermediaries from stripping the cert; private CAs must
        // not be cached by shared proxies.
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("response build: {}", e)))?;
    Ok(response)
}

fn validate_local_ca_inputs(req: &LocalCaRequest) -> Result<(), AppError> {
    if let Some(label) = &req.label {
        let trimmed = label.trim();
        if trimmed.len() > 128 {
            return Err(AppError::BadRequest(
                "Label must be 128 characters or fewer.".into(),
            ));
        }
        if trimmed
            .chars()
            .any(|c| c.is_control() || c == '\0')
        {
            return Err(AppError::BadRequest(
                "Label must not contain control characters or null bytes.".into(),
            ));
        }
    }
    if req.extra_hosts.len() > 32 {
        return Err(AppError::BadRequest(
            "At most 32 extra hosts may be supplied.".into(),
        ));
    }
    for host in &req.extra_hosts {
        let trimmed = host.trim();
        if trimmed.is_empty() || trimmed.len() > 253 {
            return Err(AppError::BadRequest(format!(
                "Invalid extra host: {:?} (length 1-253 required).",
                host
            )));
        }
        if trimmed
            .chars()
            .any(|c| c.is_control() || c == '\0' || c == ' ')
        {
            return Err(AppError::BadRequest(format!(
                "Extra host {:?} contains invalid characters.",
                host
            )));
        }
    }
    Ok(())
}

/// Persist `[tls.local_ca]` metadata to config.toml.
fn update_config_toml_local_ca(meta: &LocalCaMeta) -> Result<(), AppError> {
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

    let mut t = toml::Table::new();
    t.insert(
        "generated_at".into(),
        toml::Value::String(meta.generated_at.clone()),
    );
    t.insert(
        "ca_expires_at".into(),
        toml::Value::String(meta.ca_expires_at.clone()),
    );
    t.insert(
        "cert_expires_at".into(),
        toml::Value::String(meta.cert_expires_at.clone()),
    );
    t.insert(
        "fingerprint_sha256".into(),
        toml::Value::String(meta.fingerprint_sha256.clone()),
    );
    let hosts_array = meta
        .hosts
        .iter()
        .map(|h| toml::Value::String(h.clone()))
        .collect::<Vec<_>>();
    t.insert("hosts".into(), toml::Value::Array(hosts_array));
    tls_table.insert("local_ca".into(), toml::Value::Table(t));

    let output = toml::to_string_pretty(&doc)
        .map_err(|e| AppError::Internal(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(&config_path, output)
        .map_err(|e| AppError::Internal(format!("Failed to write config file: {}", e)))?;

    Ok(())
}
