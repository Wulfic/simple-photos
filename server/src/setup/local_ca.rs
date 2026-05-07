//! Self-signed Local CA — automatic HTTPS for LAN / offline deployments.
//!
//! This module is the 4th TLS option offered to operators. It generates:
//!
//! 1. A long-lived (10-year) self-signed **root CA** (`ca.pem` / `ca.key`).
//! 2. A short-lived (1-year) **server leaf certificate** (`server.pem` /
//!    `server.key`) signed by the root CA, with Subject Alternative Names
//!    covering `localhost`, `127.0.0.1`, `::1`, the host's machine name,
//!    and every local IPv4/IPv6 address discovered at generation time.
//!
//! Operators install the **root CA** on each device that talks to the
//! server (Linux, Windows, Android). After install the device sees a real,
//! trusted HTTPS connection — no warnings, no third-party CAs, no public
//! DNS, no inbound firewall rules.
//!
//! All artifacts live under `data/local_ca/`:
//!
//! ```text
//! data/local_ca/
//!   ca.pem            Root certificate (PEM)        — distribute to clients
//!   ca.key            Root private key (PEM)        — keep on server only
//!   server.pem        Leaf certificate (PEM)        — used by HTTPS listener
//!   server.key        Leaf private key (PEM)        — used by HTTPS listener
//!   meta.json         {generated_at, hosts, fingerprint}
//! ```
//!
//! The download endpoint serves a zip containing **only** `ca.pem`, the
//! per-platform install scripts (`install-linux.sh`, `install-windows.ps1`,
//! `install-android.txt`), and a `README.md`. It never includes the
//! private keys.
//!
//! Security boundary:
//!   * Private keys never leave the server filesystem.
//!   * The root CA is a **name-constrained** intermediate would be ideal,
//!     but rcgen does not yet support name constraints; instead we make
//!     the CA's lifetime short-ish (10 y) and clearly label it in the
//!     subject so it is easy to revoke from the OS trust store.
//!   * Files are written with `0o600` permissions on Unix.

use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Lifetime of the root CA certificate (10 years).
const CA_VALIDITY_DAYS: i64 = 365 * 10;
/// Lifetime of the server leaf certificate (~13 months — chrome rejects > 398 d).
const LEAF_VALIDITY_DAYS: i64 = 397;

/// Inputs accepted by [`generate_local_ca`].  All fields are optional —
/// sensible defaults are derived from the host machine.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GenerateRequest {
    /// Optional friendly name embedded in the CA's CommonName, e.g.
    /// "Simple Photos Local CA — kitchen-NAS".  Defaults to
    /// `"Simple Photos Local CA"` plus the machine hostname.
    #[serde(default)]
    pub label: Option<String>,
    /// Extra DNS names / IPs to embed as SANs in the leaf certificate.
    /// Useful when the server is reachable via a friendly mDNS name
    /// (`photos.local`) or a custom hosts file entry.
    #[serde(default)]
    pub extra_hosts: Vec<String>,
}

/// Result of a successful run.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateOutcome {
    pub ca_path: String,
    pub cert_path: String,
    pub key_path: String,
    pub bundle_path: String,
    pub fingerprint_sha256: String,
    pub hosts: Vec<String>,
    pub generated_at: String,
    pub ca_expires_at: String,
    pub cert_expires_at: String,
}

/// Persisted under `[tls.local_ca]` in `config.toml` so the SSL status
/// endpoint can surface "Active local CA" details without re-reading the
/// PEM file on every request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCaMeta {
    pub generated_at: String,
    pub ca_expires_at: String,
    pub cert_expires_at: String,
    pub hosts: Vec<String>,
    pub fingerprint_sha256: String,
}

// ── Generation ──────────────────────────────────────────────────────

/// Generate a fresh CA + leaf cert pair under `data_root/local_ca/`.
///
/// Idempotent: re-running rotates both the CA and the leaf.  The HTTPS
/// listener does **not** hot-reload PEM files, so the caller must restart
/// the server for the new cert to take effect — the same constraint that
/// applies to manual and Let's Encrypt certs.
pub fn generate_local_ca(
    req: &GenerateRequest,
    data_root: &Path,
) -> Result<GenerateOutcome, AppError> {
    let dir = data_root.join("local_ca");
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Internal(format!("Failed to create local_ca directory: {}", e))
    })?;

    let hostname = detect_hostname();
    let label = req
        .label
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Simple Photos Local CA ({})", hostname));

    // ── 1. Root CA ────────────────────────────────────────────────
    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| AppError::Internal(format!("rcgen ca params: {}", e)))?;
    let mut ca_dn = DistinguishedName::new();
    ca_dn.push(DnType::CommonName, label.clone());
    ca_dn.push(DnType::OrganizationName, "Simple Photos");
    ca_params.distinguished_name = ca_dn;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    ca_params.not_before = now_offset(0);
    ca_params.not_after = now_offset(CA_VALIDITY_DAYS);

    let ca_key = KeyPair::generate()
        .map_err(|e| AppError::Internal(format!("rcgen ca key: {}", e)))?;
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| AppError::Internal(format!("rcgen ca self_signed: {}", e)))?;
    let ca_pem = ca_cert.pem();
    let ca_der = ca_cert.der().to_vec();
    let fingerprint = sha256_hex(&ca_der);

    // ── 2. Leaf certificate ────────────────────────────────────────
    let hosts = collect_hosts(&hostname, &req.extra_hosts);
    let mut leaf_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| AppError::Internal(format!("rcgen leaf params: {}", e)))?;
    let mut leaf_dn = DistinguishedName::new();
    leaf_dn.push(DnType::CommonName, hostname.clone());
    leaf_dn.push(DnType::OrganizationName, "Simple Photos");
    leaf_params.distinguished_name = leaf_dn;
    leaf_params.is_ca = IsCa::NoCa;
    leaf_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    leaf_params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    leaf_params.subject_alt_names = hosts
        .iter()
        .map(|h| -> Result<SanType, AppError> {
            match h.parse::<IpAddr>() {
                Ok(ip) => Ok(SanType::IpAddress(ip)),
                Err(_) => Ok(SanType::DnsName(
                    h.as_str()
                        .try_into()
                        .map_err(|e| AppError::Internal(format!("invalid SAN dns name {h:?}: {e}")))?,
                )),
            }
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    leaf_params.not_before = now_offset(0);
    leaf_params.not_after = now_offset(LEAF_VALIDITY_DAYS);

    let leaf_key = KeyPair::generate()
        .map_err(|e| AppError::Internal(format!("rcgen leaf key: {}", e)))?;
    let leaf_cert = leaf_params
        .signed_by(&leaf_key, &ca_cert, &ca_key)
        .map_err(|e| AppError::Internal(format!("rcgen leaf signed_by: {}", e)))?;
    let leaf_pem = leaf_cert.pem();

    // ── 3. Persist atomically (write+rename) ───────────────────────
    let ca_path = dir.join("ca.pem");
    let ca_key_path = dir.join("ca.key");
    let cert_path = dir.join("server.pem");
    let key_path = dir.join("server.key");
    let meta_path = dir.join("meta.json");

    write_secret(&ca_path, ca_pem.as_bytes())?;
    write_secret(&ca_key_path, ca_key.serialize_pem().as_bytes())?;
    // The leaf PEM is the chain expected by axum-server: leaf followed by
    // the issuing CA so clients without the CA pre-installed still see
    // the full chain.
    let mut chain = leaf_pem.clone();
    chain.push_str(&ca_pem);
    write_secret(&cert_path, chain.as_bytes())?;
    write_secret(&key_path, leaf_key.serialize_pem().as_bytes())?;

    let generated_at = now_iso8601(0);
    let ca_expires_at = now_iso8601(CA_VALIDITY_DAYS);
    let cert_expires_at = now_iso8601(LEAF_VALIDITY_DAYS);

    let meta = LocalCaMeta {
        generated_at: generated_at.clone(),
        ca_expires_at: ca_expires_at.clone(),
        cert_expires_at: cert_expires_at.clone(),
        hosts: hosts.clone(),
        fingerprint_sha256: fingerprint.clone(),
    };
    let meta_json = serde_json::to_vec_pretty(&meta)
        .map_err(|e| AppError::Internal(format!("meta json: {}", e)))?;
    write_secret(&meta_path, &meta_json)?;

    // ── 4. Build the client install bundle ─────────────────────────
    let bundle_path = dir.join("simple-photos-ca-bundle.zip");
    write_bundle(&bundle_path, &ca_pem, &fingerprint, &hosts)?;

    Ok(GenerateOutcome {
        ca_path: ca_path.to_string_lossy().into_owned(),
        cert_path: cert_path.to_string_lossy().into_owned(),
        key_path: key_path.to_string_lossy().into_owned(),
        bundle_path: bundle_path.to_string_lossy().into_owned(),
        fingerprint_sha256: fingerprint,
        hosts,
        generated_at,
        ca_expires_at,
        cert_expires_at,
    })
}

/// Read existing meta.json (if present).  Used by the SSL status endpoint
/// so the UI can show "Active local CA" without re-reading the PEM file.
#[allow(dead_code)]
pub fn load_meta(data_root: &Path) -> Option<LocalCaMeta> {
    let path = data_root.join("local_ca").join("meta.json");
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Read the install bundle as raw bytes for the download endpoint.
pub fn read_bundle(data_root: &Path) -> Result<(PathBuf, Vec<u8>), AppError> {
    let path = data_root.join("local_ca").join("simple-photos-ca-bundle.zip");
    let bytes = std::fs::read(&path).map_err(|e| {
        AppError::BadRequest(format!(
            "Local CA bundle not found at {} ({}). Generate a local CA first.",
            path.display(),
            e
        ))
    })?;
    Ok((path, bytes))
}

// ── Helpers ─────────────────────────────────────────────────────────

fn write_secret(path: &Path, data: &[u8]) -> Result<(), AppError> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| AppError::Internal(format!("create {}: {}", tmp.display(), e)))?;
        f.write_all(data)
            .map_err(|e| AppError::Internal(format!("write {}: {}", tmp.display(), e)))?;
        f.sync_all().ok();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| AppError::Internal(format!("rename {}: {}", path.display(), e)))?;
    Ok(())
}

fn detect_hostname() -> String {
    // We avoid pulling in an extra crate just for hostname detection —
    // the `HOSTNAME` env var works on Linux/macOS most of the time, and
    // `COMPUTERNAME` covers Windows.  Fall back to "simple-photos".
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "simple-photos".into())
}

/// Validate a single user-supplied host entry — must be a syntactically
/// reasonable DNS name or IP.  Rejects empty strings, control chars,
/// wildcards and absurdly long entries.
fn sanitize_host(raw: &str) -> Option<String> {
    let h = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() || h.len() > 253 {
        return None;
    }
    // No control characters / spaces / wildcards / null bytes.
    if h
        .chars()
        .any(|c| c.is_control() || c == ' ' || c == '*' || c == '\0')
    {
        return None;
    }
    // IPs always allowed; otherwise must look like a DNS label set.
    if h.parse::<IpAddr>().is_ok() {
        return Some(h);
    }
    // DNS: each label 1–63 chars, [a-z0-9-], not starting/ending with `-`.
    for label in h.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return None;
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return None;
        }
    }
    Some(h)
}

fn collect_hosts(hostname: &str, extras: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        if let Some(h) = sanitize_host(s) {
            if !out.iter().any(|e| e == &h) {
                out.push(h);
            }
        }
    };

    push("localhost");
    push("127.0.0.1");
    push("::1");
    push(hostname);
    // Also add the `<hostname>.local` mDNS name — common on Windows /
    // macOS networks even without Avahi.
    if !hostname.contains('.') {
        push(&format!("{}.local", hostname));
    }

    // Local interface IPs (best-effort; ignored if the syscall is not
    // available on this platform).
    for ip in detect_local_ips() {
        push(&ip.to_string());
    }

    for h in extras {
        push(h);
    }
    out
}

#[cfg(unix)]
fn detect_local_ips() -> Vec<IpAddr> {
    // Stay dependency-free: probe a handful of UDP "connections" to public
    // sentinels and let the kernel tell us which local interface would be
    // used.  No packets are sent — `connect()` on UDP is a routing hint
    // only.  Falls back to an empty list when the host is air-gapped.
    use std::net::UdpSocket;
    let mut out = Vec::new();
    for target in &["8.8.8.8:80", "1.1.1.1:80"] {
        if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
            if sock.connect(*target).is_ok() {
                if let Ok(addr) = sock.local_addr() {
                    let ip = addr.ip();
                    if !ip.is_unspecified() && !ip.is_loopback() && !out.contains(&ip) {
                        out.push(ip);
                    }
                }
            }
        }
        if let Ok(sock) = UdpSocket::bind("[::]:0") {
            if sock.connect(*target).is_ok() {
                if let Ok(addr) = sock.local_addr() {
                    let ip = addr.ip();
                    if !ip.is_unspecified() && !ip.is_loopback() && !out.contains(&ip) {
                        out.push(ip);
                    }
                }
            }
        }
    }
    out
}

#[cfg(not(unix))]
fn detect_local_ips() -> Vec<IpAddr> {
    use std::net::UdpSocket;
    let mut out = Vec::new();
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                let ip = addr.ip();
                if !ip.is_unspecified() && !ip.is_loopback() {
                    out.push(ip);
                }
            }
        }
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(digest.len() * 3);
    for (i, b) in digest.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.push_str(&format!("{:02X}", b));
    }
    out
}

/// Returns a `time::OffsetDateTime` `days` from now (date-only, UTC).
///
/// rcgen 0.13's `not_before` / `not_after` are `time::OffsetDateTime`,
/// but we avoid a direct dep on `time` by going through
/// `rcgen::date_time_ymd` — the helper rcgen exposes for callers in
/// exactly our position.  The result type is `time::OffsetDateTime`
/// inferred from the helper's return signature.
fn now_offset(days: i64) -> time::OffsetDateTime {
    use chrono::Datelike;
    let target = chrono::Utc::now() + chrono::Duration::days(days);
    rcgen::date_time_ymd(
        target.year(),
        target.month() as u8,
        target.day() as u8,
    )
}

fn now_iso8601(days: i64) -> String {
    let now = chrono::Utc::now() + chrono::Duration::days(days);
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ── Bundle (zip) ────────────────────────────────────────────────────

fn write_bundle(
    path: &Path,
    ca_pem: &str,
    fingerprint: &str,
    hosts: &[String],
) -> Result<(), AppError> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    let tmp = path.with_extension("tmp");
    let file = std::fs::File::create(&tmp)
        .map_err(|e| AppError::Internal(format!("create bundle {}: {}", tmp.display(), e)))?;
    let mut zw = zip::ZipWriter::new(file);

    // Two option presets:
    //   • `opts`      — plain files (0644)            → ca.pem, *.txt, *.md, *.ps1
    //   • `exec_opts` — executable scripts (0755)     → install-linux.sh
    //
    // Without the executable bit on the shell script, `unzip` extracts it
    // as 0644 and the operator gets "Permission denied" trying to run
    // `./install-linux.sh` — even under sudo, because sudo doesn't grant
    // an exec bit, it just changes the EUID.  We mark the shell script
    // as 0755 so it Just Works after `unzip`.
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let exec_opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let mut add = |name: &str,
                   body: &[u8],
                   options: SimpleFileOptions|
     -> Result<(), AppError> {
        zw.start_file(name, options)
            .map_err(|e| AppError::Internal(format!("zip start {}: {}", name, e)))?;
        zw.write_all(body)
            .map_err(|e| AppError::Internal(format!("zip write {}: {}", name, e)))?;
        Ok(())
    };

    add("ca.pem", ca_pem.as_bytes(), opts)?;
    add("install-linux.sh", linux_script(fingerprint).as_bytes(), exec_opts)?;
    add(
        "install-windows.ps1",
        windows_script(fingerprint).as_bytes(),
        opts,
    )?;
    add(
        "install-android.txt",
        android_instructions(fingerprint).as_bytes(),
        opts,
    )?;
    add("README.md", readme(fingerprint, hosts).as_bytes(), opts)?;

    zw.finish()
        .map_err(|e| AppError::Internal(format!("zip finish: {}", e)))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| AppError::Internal(format!("rename bundle: {}", e)))?;
    Ok(())
}

fn linux_script(fingerprint: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
# Install the Simple Photos local CA on Linux.
# Fingerprint (SHA-256): {fp}
#
# Usage:  sudo ./install-linux.sh
set -euo pipefail

if [ "$EUID" -ne 0 ]; then
  echo "Please run with sudo: sudo ./install-linux.sh" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${{BASH_SOURCE[0]}}")" && pwd)"
SRC="$SCRIPT_DIR/ca.pem"
if [ ! -f "$SRC" ]; then
  echo "ca.pem not found next to this script" >&2
  exit 1
fi

# Verify fingerprint before installing — refuses to install a tampered cert.
ACTUAL=$(openssl x509 -in "$SRC" -noout -fingerprint -sha256 | cut -d= -f2)
EXPECTED="{fp}"
if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "Fingerprint mismatch:" >&2
  echo "  expected: $EXPECTED" >&2
  echo "  got     : $ACTUAL"   >&2
  exit 2
fi

if   [ -d /usr/local/share/ca-certificates ]; then
  cp "$SRC" /usr/local/share/ca-certificates/simple-photos-local-ca.crt
  update-ca-certificates
elif [ -d /etc/pki/ca-trust/source/anchors ]; then
  cp "$SRC" /etc/pki/ca-trust/source/anchors/simple-photos-local-ca.pem
  update-ca-trust extract
elif [ -d /etc/ca-certificates/trust-source/anchors ]; then
  cp "$SRC" /etc/ca-certificates/trust-source/anchors/simple-photos-local-ca.crt
  trust extract-compat
else
  echo "Unsupported distribution: please install ca.pem manually." >&2
  exit 3
fi

echo
echo "✓ Simple Photos local CA installed (fingerprint: {fp})."
echo "Browsers using the system store (Chromium, Edge, curl, wget) will trust"
echo "the certificate immediately. Firefox uses its own store — open"
echo "  about:preferences#privacy → View Certificates → Authorities → Import"
echo "and tick \"Trust this CA to identify websites\"."
"#,
        fp = fingerprint,
    )
}

fn windows_script(fingerprint: &str) -> String {
    format!(
        r#"# Install the Simple Photos local CA on Windows (PowerShell, run as admin).
# Fingerprint (SHA-256): {fp}
#
# Usage:  Right-click → Run with PowerShell (as administrator)
#         OR  powershell -ExecutionPolicy Bypass -File .\install-windows.ps1

#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'

$src = Join-Path $PSScriptRoot 'ca.pem'
if (-not (Test-Path $src)) {{
  Write-Error 'ca.pem not found next to this script'
  exit 1
}}

# Verify fingerprint before installing.
$cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 $src
$expected = '{fp_no_colons}'
$actual = $cert.GetCertHashString('SHA256')
if ($actual.ToUpperInvariant() -ne $expected.ToUpperInvariant()) {{
  Write-Error "Fingerprint mismatch: expected $expected, got $actual"
  exit 2
}}

# Import into the LocalMachine "Trusted Root" store.
$store = New-Object System.Security.Cryptography.X509Certificates.X509Store(
  [System.Security.Cryptography.X509Certificates.StoreName]::Root,
  [System.Security.Cryptography.X509Certificates.StoreLocation]::LocalMachine)
$store.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
$store.Add($cert)
$store.Close()

Write-Host ''
Write-Host '✓ Simple Photos local CA installed.' -ForegroundColor Green
Write-Host "Fingerprint: $actual"
Write-Host 'Edge / Chrome / curl will trust the certificate immediately.'
Write-Host 'Firefox uses its own store — Settings → Privacy & Security →'
Write-Host '  View Certificates → Authorities → Import, then tick'
Write-Host '  "Trust this CA to identify websites".'
"#,
        fp = fingerprint,
        fp_no_colons = fingerprint.replace(':', ""),
    )
}

fn android_instructions(fingerprint: &str) -> String {
    format!(
        r#"Install the Simple Photos local CA on Android
=============================================

Fingerprint (SHA-256): {fp}

Android cannot install a CA from a script — the OS forces a manual
review.  The procedure below works on every Android version from 7
(Nougat) onward.

1. Copy `ca.pem` onto the device (USB transfer, email it to yourself,
   or download it directly from the server's settings page on the
   device's browser).

2. Rename the file from `ca.pem` to `simple-photos-local-ca.crt`.
   Android only opens the install dialog for files with a `.crt`
   extension.

3. Open Settings.  The exact path varies by manufacturer; common ones:

      Pixel / stock:  Security & privacy → More security & privacy
                      → Encryption & credentials → Install a certificate
                      → CA certificate
      Samsung:        Biometrics and security → Other security settings
                      → Install from device storage → CA certificate
      Xiaomi:         Passwords & security → Privacy → Encryption &
                      credentials → Install a certificate → CA certificate

4. Pick the `.crt` file.  Android shows a warning that "your network
   activity may be monitored" — this is the expected dialog for any
   user-installed CA.  Confirm.

5. Verify the fingerprint matches the one above:
      Settings → Security → Encryption & credentials → Trusted credentials
      → User tab → "Simple Photos Local CA"

6. **Important — Simple Photos Android app**: Android 7+ apps do
   **not** trust user-installed CAs by default.  The Simple Photos
   APK ships with a `network_security_config.xml` that opts in to
   user CAs, so the app works after step 4 with no further action.
   Third-party apps (e.g. a different photo viewer) may still refuse
   the connection unless they were built the same way.

To remove the certificate later:
   Settings → Security → Encryption & credentials → User credentials
   → tap → Remove
"#,
        fp = fingerprint,
    )
}

fn readme(fingerprint: &str, hosts: &[String]) -> String {
    let host_list = hosts
        .iter()
        .map(|h| format!("  - {}", h))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"# Simple Photos — Local CA Install Bundle

This zip contains the **public** root certificate (`ca.pem`) for your
Simple Photos server, plus per-platform install helpers.  Install it on
every device you use to browse, sync, or back up photos and the HTTPS
certificate served by the Simple Photos server will be trusted with no
"Your connection is not private" warning.

The private keys never leave the server.

## What this CA covers

The server's leaf certificate is valid for these hosts:

{host_list}

If you reach the server through another name (a custom DNS entry, a
VPN-only hostname, etc.) re-generate the CA from the settings page and
add the extra name in the **Extra hosts** field.

## Fingerprint

SHA-256: `{fp}`

Verify this fingerprint matches the one shown in the server's settings
page **before** installing.  A mismatch means the bundle has been
tampered with on its way to your device.

## Files

| File                     | Platform | Action                                         |
|--------------------------|----------|------------------------------------------------|
| `install-linux.sh`       | Linux    | `sudo ./install-linux.sh`                      |
| `install-windows.ps1`    | Windows  | Run with PowerShell as administrator           |
| `install-android.txt`    | Android  | Read the file — Android requires manual steps  |
| `ca.pem`                 | All      | The certificate itself                         |

### Linux — if `./install-linux.sh` says "Permission denied"

Some unzip tools strip the executable bit even though the bundle ships
it set.  Two equivalent workarounds:

```bash
chmod +x install-linux.sh && sudo ./install-linux.sh
# — or —
sudo bash install-linux.sh
```

If you see "bad interpreter" or stray `^M` in the error, the file was
extracted with Windows-style line endings.  Convert in place:

```bash
sed -i 's/\r$//' install-linux.sh && sudo bash install-linux.sh
```

## Removing the CA

Each platform's standard "Trusted root certificates" UI lets you remove
the CA at any time.  The Simple Photos Android app falls back to
plain HTTP if the certificate is removed and HTTPS is still enforced —
re-install it to restore service.
"#,
        host_list = host_list,
        fp = fingerprint,
    )
}
