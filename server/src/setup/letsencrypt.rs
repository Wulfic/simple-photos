//! Let's Encrypt / ACME-v2 (RFC 8555) certificate provisioning.
//!
//! The setup wizard lets the admin supply a domain + email and tick "agree
//! to the Let's Encrypt Terms of Service".  This module then:
//!
//! 1. Creates an ACME account against either the production or the staging
//!    Let's Encrypt directory (staging issues untrusted certs but has far
//!    looser rate limits — useful for E2E tests).
//! 2. Submits a `new-order` for the domain.
//! 3. Spawns a transient HTTP listener on the configured challenge port
//!    (default 80) that serves the ACME `http-01` token at
//!    `GET /.well-known/acme-challenge/{token}`.
//! 4. Tells the CA the challenge is ready, then polls the order until it
//!    becomes `Ready`, finalises with a freshly-generated CSR, and downloads
//!    the issued certificate chain.
//! 5. Atomically writes the chain + private key to
//!    `data/letsencrypt/{domain}/fullchain.pem` and `privkey.pem`.
//!
//! The HTTPS listener picks up the new files on the next server restart —
//! axum-server does not hot-reload PEM files.  The renewal task therefore
//! logs a clear "restart required" warning whenever a cert is renewed.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path as AxumPath, State as AxumState};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use dashmap::DashMap;
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder,
    OrderStatus,
};
use rcgen::{CertificateParams, DistinguishedName, KeyPair};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Maximum number of attempts when polling order / authorization state.
const MAX_POLL_ATTEMPTS: u32 = 60;
/// Delay between polls (seconds). 60 × 2 s = 2 minutes max.
const POLL_INTERVAL_SECS: u64 = 2;

/// Inputs required to provision a Let's Encrypt certificate.
#[derive(Debug, Clone, Deserialize)]
pub struct ProvisionRequest {
    /// Fully-qualified domain the certificate will be issued for.
    pub domain: String,
    /// Contact email used to register the ACME account.
    pub email: String,
    /// Whether the user has agreed to the Let's Encrypt Subscriber Agreement.
    /// **Must** be `true` — the CA refuses to issue otherwise.
    pub agree_tos: bool,
    /// `true` to use the staging directory (test certs, looser rate limits).
    /// Defaults to `false` (production).
    #[serde(default)]
    pub staging: bool,
    /// Port on which to serve the ACME `http-01` challenge.
    /// Defaults to `80`. Useful when port 80 is unreachable but a
    /// reverse proxy / port-forward maps it to a different local port.
    #[serde(default = "default_challenge_port")]
    pub challenge_port: u16,
}

fn default_challenge_port() -> u16 {
    80
}

/// Outcome of a successful provisioning run.
#[derive(Debug, Clone, Serialize)]
pub struct ProvisionResult {
    /// Absolute path to the saved fullchain PEM.
    pub cert_path: String,
    /// Absolute path to the saved private-key PEM.
    pub key_path: String,
    /// Echoes the domain for clarity in the response.
    pub domain: String,
    /// Echoes whether the staging directory was used.
    pub staging: bool,
    /// RFC-3339 timestamp of issuance.
    pub issued_at: String,
}

/// Validate the user-supplied inputs.  Returns a `BadRequest` describing the
/// first failed rule, or `Ok(())` when every field is acceptable.  Performs
/// **only** local validation — never reaches the Internet — so the setup
/// wizard can call this in a "dry-run" mode without rate-limiting Let's
/// Encrypt accounts.
pub fn validate_inputs(req: &ProvisionRequest) -> Result<(), AppError> {
    let domain = req.domain.trim();
    let email = req.email.trim();

    if !req.agree_tos {
        return Err(AppError::BadRequest(
            "You must agree to the Let's Encrypt Subscriber Agreement".into(),
        ));
    }

    if domain.is_empty() {
        return Err(AppError::BadRequest("Domain is required".into()));
    }
    if domain.len() > 253 {
        return Err(AppError::BadRequest(
            "Domain is too long (max 253 characters)".into(),
        ));
    }
    // Reject obvious non-FQDNs.  Let's Encrypt does not issue for raw IPs,
    // single-label hosts, wildcards via HTTP-01, or IDN-sandboxed hosts.
    if !domain.contains('.') {
        return Err(AppError::BadRequest(
            "Domain must be a fully-qualified hostname (e.g. photos.example.com)".into(),
        ));
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return Err(AppError::BadRequest("Domain has empty label".into()));
    }
    if domain.starts_with('*') {
        return Err(AppError::BadRequest(
            "Wildcard domains require DNS-01 challenges, which are not yet supported".into(),
        ));
    }
    if domain.parse::<std::net::IpAddr>().is_ok() {
        return Err(AppError::BadRequest(
            "Let's Encrypt cannot issue certificates for raw IP addresses".into(),
        ));
    }
    // Each DNS label: 1-63 chars, alphanumeric or hyphen, no leading/trailing hyphen.
    for label in domain.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(AppError::BadRequest(
                "Domain label must be 1-63 characters".into(),
            ));
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(AppError::BadRequest(
                "Domain may only contain ASCII letters, digits, hyphens, and dots".into(),
            ));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(AppError::BadRequest(
                "Domain label may not start or end with a hyphen".into(),
            ));
        }
    }

    if email.is_empty() {
        return Err(AppError::BadRequest("Contact email is required".into()));
    }
    if email.len() > 320 {
        return Err(AppError::BadRequest("Email is too long".into()));
    }
    // RFC-5321 mandates exactly one '@' separating local + domain parts;
    // a dot-bearing domain part is required for Let's Encrypt accounts.
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 || parts[0].is_empty() || !parts[1].contains('.') {
        return Err(AppError::BadRequest(
            "Email must be of the form user@domain.tld".into(),
        ));
    }
    // Reject control characters and whitespace anywhere in the address.
    if email
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(AppError::BadRequest(
            "Email must not contain whitespace or control characters".into(),
        ));
    }

    if req.challenge_port == 0 {
        return Err(AppError::BadRequest(
            "challenge_port must be between 1 and 65535".into(),
        ));
    }

    Ok(())
}

/// Resolve where on disk to write the issued certificate and key.
///
/// `data_root` is normally `data/letsencrypt`; the caller supplies it so
/// tests can write into a tempdir.
pub fn cert_paths(data_root: &Path, domain: &str) -> (PathBuf, PathBuf) {
    let dir = data_root.join(domain);
    (dir.join("fullchain.pem"), dir.join("privkey.pem"))
}

/// Drive the full ACME handshake and write the resulting certificate to
/// disk.  On success returns the file paths the caller must persist into
/// `[tls]` of `config.toml`.
///
/// `directory_url` is `None` for the production / staging Let's Encrypt
/// endpoints (selected by `req.staging`); tests may pass an explicit URL
/// (e.g. a Pebble instance) to avoid hitting the real CA.
pub async fn provision_certificate(
    req: &ProvisionRequest,
    data_root: &Path,
    directory_url: Option<&str>,
) -> Result<ProvisionResult, AppError> {
    validate_inputs(req)?;

    let directory = directory_url
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if req.staging {
                LetsEncrypt::Staging.url().to_string()
            } else {
                LetsEncrypt::Production.url().to_string()
            }
        });

    tracing::info!(
        domain = %req.domain,
        email = %req.email,
        staging = req.staging,
        "Starting Let's Encrypt provisioning against {}",
        directory
    );

    // ── 1. Create / fetch ACME account ────────────────────────────────
    let contact_email = format!("mailto:{}", req.email.trim());
    let new_account = NewAccount {
        contact: &[&contact_email],
        terms_of_service_agreed: req.agree_tos,
        only_return_existing: false,
    };
    let (account, _credentials) = Account::create(&new_account, &directory, None)
        .await
        .map_err(|e| AppError::Internal(format!("ACME account creation failed: {}", e)))?;

    // ── 2. Place new order ────────────────────────────────────────────
    let identifier = Identifier::Dns(req.domain.trim().to_string());
    let mut order = account
        .new_order(&NewOrder {
            identifiers: &[identifier],
        })
        .await
        .map_err(|e| AppError::Internal(format!("ACME new-order failed: {}", e)))?;

    // ── 3. Solve the http-01 challenge ────────────────────────────────
    let authorizations = order
        .authorizations()
        .await
        .map_err(|e| AppError::Internal(format!("ACME authorizations fetch failed: {}", e)))?;

    // Map of challenge token → key authorization. Shared with the listener.
    let tokens: Arc<DashMap<String, String>> = Arc::new(DashMap::new());
    let mut challenge_urls: Vec<String> = Vec::new();

    for auth in &authorizations {
        match auth.status {
            AuthorizationStatus::Pending => {}
            AuthorizationStatus::Valid => continue,
            other => {
                return Err(AppError::Internal(format!(
                    "ACME authorization in unexpected state: {:?}",
                    other
                )));
            }
        }

        let challenge = auth
            .challenges
            .iter()
            .find(|c| c.r#type == ChallengeType::Http01)
            .ok_or_else(|| {
                AppError::Internal(
                    "ACME server did not offer an http-01 challenge — DNS-01 is not supported"
                        .into(),
                )
            })?;

        let key_auth = order.key_authorization(challenge);
        tokens.insert(challenge.token.clone(), key_auth.as_str().to_string());
        challenge_urls.push(challenge.url.clone());
    }

    // ── 4. Spin up the temporary HTTP listener ────────────────────────
    let listener_addr: SocketAddr = format!("0.0.0.0:{}", req.challenge_port)
        .parse()
        .map_err(|e| AppError::BadRequest(format!("Invalid challenge_port: {}", e)))?;

    let app = Router::new()
        .route(
            "/.well-known/acme-challenge/{token}",
            get(serve_challenge),
        )
        .with_state(tokens.clone());

    let listener = tokio::net::TcpListener::bind(listener_addr)
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "Failed to bind ACME challenge listener on port {} ({}). \
                 Port 80 must be reachable from the public Internet for HTTP-01 to work; \
                 forward port 80 to {} and retry, or use a reverse proxy.",
                req.challenge_port, e, req.challenge_port,
            ))
        })?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // Notify the CA that each challenge is ready.
    let provision_outcome = async {
        for url in &challenge_urls {
            order
                .set_challenge_ready(url)
                .await
                .map_err(|e| AppError::Internal(format!("ACME set-challenge-ready failed: {}", e)))?;
        }

        // ── 5. Poll order until Ready ─────────────────────────────────
        let mut attempts = 0u32;
        loop {
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
            let state = order
                .refresh()
                .await
                .map_err(|e| AppError::Internal(format!("ACME order refresh failed: {}", e)))?;
            match state.status {
                OrderStatus::Ready => break,
                OrderStatus::Invalid => {
                    return Err(AppError::Internal(
                        "ACME order became invalid \u{2014} the CA could not validate the \
                         http-01 challenge.  Confirm port 80 is open and DNS resolves to this server."
                            .into(),
                    ));
                }
                _ => {}
            }
            attempts += 1;
            if attempts >= MAX_POLL_ATTEMPTS {
                return Err(AppError::Internal(
                    "Timed out waiting for Let's Encrypt to validate the challenge".into(),
                ));
            }
        }

        // ── 6. Generate CSR and finalize ──────────────────────────────
        let mut params = CertificateParams::new(vec![req.domain.trim().to_string()])
            .map_err(|e| AppError::Internal(format!("rcgen params: {}", e)))?;
        params.distinguished_name = DistinguishedName::new();
        let key_pair = KeyPair::generate()
            .map_err(|e| AppError::Internal(format!("rcgen key generation: {}", e)))?;
        let csr = params
            .serialize_request(&key_pair)
            .map_err(|e| AppError::Internal(format!("rcgen CSR serialization: {}", e)))?;

        order
            .finalize(csr.der())
            .await
            .map_err(|e| AppError::Internal(format!("ACME finalize failed: {}", e)))?;

        // ── 7. Poll for the issued certificate ────────────────────────
        let mut attempts = 0u32;
        let cert_chain_pem = loop {
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
            match order
                .certificate()
                .await
                .map_err(|e| AppError::Internal(format!("ACME certificate fetch failed: {}", e)))?
            {
                Some(pem) => break pem,
                None => {
                    attempts += 1;
                    if attempts >= MAX_POLL_ATTEMPTS {
                        return Err(AppError::Internal(
                            "Timed out waiting for Let's Encrypt to issue the certificate".into(),
                        ));
                    }
                }
            }
        };

        Ok::<(String, KeyPair), AppError>((cert_chain_pem, key_pair))
    }
    .await;

    // Always tear down the listener — even on error — before returning.
    let _ = shutdown_tx.send(());
    let _ = server_task.await;

    let (cert_chain_pem, key_pair) = provision_outcome?;

    // ── 8. Atomically write fullchain + private key ───────────────────
    let domain = req.domain.trim();
    let (cert_path, key_path) = cert_paths(data_root, domain);
    if let Some(parent) = cert_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("create cert dir: {}", e)))?;
    }
    write_atomic(&cert_path, cert_chain_pem.as_bytes()).await?;
    write_atomic(&key_path, key_pair.serialize_pem().as_bytes()).await?;

    // Tighten permissions on the private key (best-effort, Unix only).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = tokio::fs::metadata(&key_path).await {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = tokio::fs::set_permissions(&key_path, perms).await;
        }
    }

    let issued_at = chrono::Utc::now().to_rfc3339();
    tracing::info!(
        domain = %domain,
        cert = ?cert_path,
        "Let's Encrypt certificate issued successfully"
    );

    Ok(ProvisionResult {
        cert_path: cert_path.to_string_lossy().into_owned(),
        key_path: key_path.to_string_lossy().into_owned(),
        domain: domain.to_string(),
        staging: req.staging,
        issued_at,
    })
}

/// Atomic write: stage at `<path>.tmp` then rename.  Avoids a torn-file
/// scenario where a half-written PEM is read by the rustls loader.
async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| AppError::Internal(format!("write {:?}: {}", tmp, e)))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| AppError::Internal(format!("rename {:?} -> {:?}: {}", tmp, path, e)))?;
    Ok(())
}

/// Axum handler that returns the ACME `key_authorization` for a given
/// challenge token, or `404` if the token is unknown.
async fn serve_challenge(
    AxumState(tokens): AxumState<Arc<DashMap<String, String>>>,
    AxumPath(token): AxumPath<String>,
) -> Result<String, StatusCode> {
    tokens
        .get(&token)
        .map(|v| v.value().clone())
        .ok_or(StatusCode::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(domain: &str, email: &str, agree: bool) -> ProvisionRequest {
        ProvisionRequest {
            domain: domain.into(),
            email: email.into(),
            agree_tos: agree,
            staging: true,
            challenge_port: 80,
        }
    }

    #[test]
    fn rejects_unagreed_tos() {
        let r = req("photos.example.com", "a@b.co", false);
        assert!(matches!(validate_inputs(&r), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn rejects_raw_ip() {
        let r = req("203.0.113.5", "a@b.co", true);
        assert!(matches!(validate_inputs(&r), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn rejects_wildcard() {
        let r = req("*.example.com", "a@b.co", true);
        assert!(matches!(validate_inputs(&r), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn rejects_single_label() {
        let r = req("localhost", "a@b.co", true);
        assert!(matches!(validate_inputs(&r), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn rejects_bad_email() {
        let r = req("photos.example.com", "not-an-email", true);
        assert!(matches!(validate_inputs(&r), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn rejects_zero_port() {
        let mut r = req("photos.example.com", "a@b.co", true);
        r.challenge_port = 0;
        assert!(matches!(validate_inputs(&r), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn accepts_valid_input() {
        let r = req("photos.example.com", "admin@example.com", true);
        assert!(validate_inputs(&r).is_ok());
    }

    #[test]
    fn cert_paths_under_data_root() {
        let (c, k) = cert_paths(Path::new("/tmp/le"), "photos.example.com");
        assert!(c.ends_with("photos.example.com/fullchain.pem"));
        assert!(k.ends_with("photos.example.com/privkey.pem"));
    }
}
