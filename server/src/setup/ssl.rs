//! SSL/TLS configuration endpoints.
//!
//! These endpoints let admins view and update TLS settings (enable/disable,
//! set certificate and key paths). Changes are persisted to config.toml.
//!
//! Includes an optional Let's Encrypt certificate generator that uses the
//! ACME HTTP-01 challenge flow.  The server temporarily binds port 80 to
//! serve the challenge token, so that port must be available and publicly
//! reachable on the domain being requested.

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
        &state.pool,
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

// ── Let's Encrypt ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LetsEncryptRequest {
    /// Fully-qualified domain name (e.g. "photos.example.com")
    pub domain: String,
    /// Contact e-mail for Let's Encrypt notifications
    pub email: String,
    /// If true, use the Let's Encrypt *staging* environment (for testing)
    #[serde(default)]
    pub staging: bool,
}

#[derive(Debug, Serialize)]
pub struct LetsEncryptResponse {
    pub success: bool,
    pub cert_path: String,
    pub key_path: String,
    pub message: String,
}

/// POST /api/admin/ssl/letsencrypt — Obtain a certificate from Let's Encrypt.
///
/// Flow:
/// 1. Create an ACME account
/// 2. Request a certificate order for the domain
/// 3. Spin up a temporary HTTP server on port 80 for the HTTP-01 challenge
/// 4. Tell the CA to verify, then poll until the order is ready
/// 5. Generate a key pair + CSR, finalize the order
/// 6. Save the certificate & private key to `certs/<domain>.{crt,key}`
/// 7. Persist the paths in config.toml and enable TLS
///
/// **Requirements:**
/// - Port 80 must be available on this machine.
/// - The domain must resolve to this machine's public IP.
pub async fn generate_letsencrypt(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<LetsEncryptRequest>,
) -> Result<Json<LetsEncryptResponse>, AppError> {
    use instant_acme::{
        Account, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder, OrderStatus,
    };
    use std::time::Duration;

    require_admin(&state, &auth).await?;

    // ── Validate inputs ────────────────────────────────────────────────────
    let domain = req.domain.trim().to_lowercase();
    if domain.is_empty() || !domain.contains('.') || domain.len() > 253 {
        return Err(AppError::BadRequest(
            "A valid domain name is required (e.g. photos.example.com)".into(),
        ));
    }
    if req.email.is_empty() || !req.email.contains('@') {
        return Err(AppError::BadRequest(
            "A valid contact e-mail is required".into(),
        ));
    }

    tracing::info!(
        "Starting Let's Encrypt certificate generation for {} (staging={})",
        domain,
        req.staging
    );

    // ── Prepare output paths ───────────────────────────────────────────────
    let certs_dir = std::path::PathBuf::from("certs");
    tokio::fs::create_dir_all(&certs_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create certs/ directory: {e}")))?;

    let cert_path = certs_dir.join(format!("{domain}.crt"));
    let key_path = certs_dir.join(format!("{domain}.key"));

    // ── ACME account ───────────────────────────────────────────────────────
    let directory_url = if req.staging {
        LetsEncrypt::Staging.url()
    } else {
        LetsEncrypt::Production.url()
    };

    let (account, _credentials) = Account::create(
        &NewAccount {
            contact: &[&format!("mailto:{}", req.email)],
            terms_of_service_agreed: true,
            only_return_existing: false,
        },
        directory_url,
        None,
    )
    .await
    .map_err(|e| AppError::Internal(format!("ACME account creation failed: {e}")))?;

    // ── Create order ───────────────────────────────────────────────────────
    let identifiers = vec![Identifier::Dns(domain.clone())];
    let mut order = account
        .new_order(&NewOrder {
            identifiers: &identifiers,
        })
        .await
        .map_err(|e| AppError::Internal(format!("ACME order creation failed: {e}")))?;

    // ── Get the HTTP-01 challenge ──────────────────────────────────────────
    let authorizations = order
        .authorizations()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch authorizations: {e}")))?;

    let authorization = authorizations
        .first()
        .ok_or_else(|| AppError::Internal("No authorizations returned".into()))?;

    let challenge = authorization
        .challenges
        .iter()
        .find(|c| c.r#type == ChallengeType::Http01)
        .ok_or_else(|| {
            AppError::Internal(
                "The CA did not offer an HTTP-01 challenge for this domain".into(),
            )
        })?;

    let _token = challenge.token.clone();
    let key_auth = order.key_authorization(challenge).as_str().to_owned();
    let challenge_url = challenge.url.clone();

    // ── Spin up a temporary challenge server on port 80 ────────────────────
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let challenge_key_auth = key_auth.clone();

    let challenge_handle = tokio::spawn(async move {
        use axum::routing::get;
        use axum::Router;

        let ka = challenge_key_auth;
        let app = Router::new().route(
            "/.well-known/acme-challenge/{token}",
            get(move || {
                let body = ka.clone();
                async move { body }
            }),
        );

        let listener = match tokio::net::TcpListener::bind("0.0.0.0:80").await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("Cannot bind port 80 for ACME challenge: {e}");
                return;
            }
        };

        tracing::info!("ACME challenge server listening on :80");
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
        tracing::info!("ACME challenge server stopped");
    });

    // Give the challenge server a moment to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ── Tell the CA we are ready ───────────────────────────────────────────
    if let Err(e) = order.set_challenge_ready(&challenge_url).await {
        let _ = shutdown_tx.send(());
        return Err(AppError::Internal(format!("Failed to signal challenge readiness: {e}")));
    }

    // ── Poll until the order is Ready ──────────────────────────────────────
    let mut tries = 0u32;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let state = match order.refresh().await {
            Ok(s) => s,
            Err(e) => {
                let _ = shutdown_tx.send(());
                return Err(AppError::Internal(format!("Failed to refresh order status: {e}")));
            }
        };

        match state.status {
            OrderStatus::Ready | OrderStatus::Valid => break,
            OrderStatus::Invalid => {
                let _ = shutdown_tx.send(());
                return Err(AppError::Internal(
                    "ACME order became invalid — the challenge may have failed. \
                     Make sure the domain points to this server and port 80 is reachable."
                        .into(),
                ));
            }
            _ => {}
        }

        tries += 1;
        if tries > 30 {
            let _ = shutdown_tx.send(());
            return Err(AppError::Internal(
                "Timed out waiting for ACME challenge verification (60 s)".into(),
            ));
        }
    }

    // Shut down the challenge server — no longer needed
    let _ = shutdown_tx.send(());
    let _ = challenge_handle.await;

    // ── Generate key pair & CSR ────────────────────────────────────────────
    let key_pair = rcgen::KeyPair::generate()
        .map_err(|e| AppError::Internal(format!("Key generation failed: {e}")))?;

    let mut cert_params = rcgen::CertificateParams::new(vec![domain.clone()])
        .map_err(|e| AppError::Internal(format!("CSR params error: {e}")))?;
    cert_params.distinguished_name = rcgen::DistinguishedName::new();

    let csr = cert_params
        .serialize_request(&key_pair)
        .map_err(|e| AppError::Internal(format!("CSR serialisation failed: {e}")))?;

    // ── Finalize the order ─────────────────────────────────────────────────
    order
        .finalize(csr.der())
        .await
        .map_err(|e| AppError::Internal(format!("ACME order finalization failed: {e}")))?;

    // Poll until the certificate is issued
    let cert_chain_pem = {
        let mut cert_tries = 0u32;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let state = order.refresh().await.map_err(|e| {
                AppError::Internal(format!("Failed to refresh order after finalize: {e}"))
            })?;

            match state.status {
                OrderStatus::Valid => {
                    break order
                        .certificate()
                        .await
                        .map_err(|e| {
                            AppError::Internal(format!("Failed to download certificate: {e}"))
                        })?
                        .ok_or_else(|| {
                            AppError::Internal("Certificate was empty after order completion".into())
                        })?;
                }
                OrderStatus::Invalid => {
                    return Err(AppError::Internal(
                        "Order became invalid during certificate issuance".into(),
                    ));
                }
                _ => {}
            }

            cert_tries += 1;
            if cert_tries > 30 {
                return Err(AppError::Internal(
                    "Timed out waiting for certificate issuance".into(),
                ));
            }
        }
    };

    // ── Persist cert & key ─────────────────────────────────────────────────
    let key_pem = key_pair.serialize_pem();

    tokio::fs::write(&cert_path, &cert_chain_pem)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write certificate file: {e}")))?;

    tokio::fs::write(&key_path, &key_pem)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write private key file: {e}")))?;

    // ── Update config.toml to enable TLS with the new paths ────────────────
    let cert_str = cert_path.to_string_lossy().to_string();
    let key_str = key_path.to_string_lossy().to_string();
    update_config_toml_ssl(true, Some(&cert_str), Some(&key_str))?;

    // ── Audit ──────────────────────────────────────────────────────────────
    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "letsencrypt_cert_generated",
            "domain": domain,
            "staging": req.staging,
        })),
    )
    .await;

    tracing::info!(
        "Let's Encrypt certificate for {} saved to {:?} / {:?}",
        domain,
        cert_path,
        key_path
    );

    Ok(Json(LetsEncryptResponse {
        success: true,
        cert_path: cert_str,
        key_path: key_str,
        message: format!(
            "Certificate for {domain} generated successfully. \
             TLS has been enabled — restart the server to apply."
        ),
    }))
}
