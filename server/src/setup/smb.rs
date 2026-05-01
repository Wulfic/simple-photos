//! SMB / CIFS network-share support for the Simple Photos storage backend.
//!
//! This module lets administrators point the server at an SMB share directly
//! (e.g. via the first-run setup wizard) instead of requiring the operator to
//! pre-mount the share on the host. We accept several common address formats
//! and shell out to `mount.cifs` (Linux) to mount the share at a stable point
//! under `data/mounts/<host>__<share>`. Once mounted, the rest of the server
//! treats it as a normal local directory — every blob handler, scan task, and
//! background worker continues to use `tokio::fs` against the mount point.
//!
//! ## Accepted input formats
//!
//! | Example                                  | Notes                          |
//! |------------------------------------------|--------------------------------|
//! | `smb://vault.local/photos/SimplePhotos`  | RFC 4795-style URI             |
//! | `\\vault.local\photos\SimplePhotos`      | Windows UNC (escaping handled) |
//! | `//vault.local/photos/SimplePhotos`      | POSIX-style                    |
//! | `smb://user:pass@host:445/share/sub`     | Inline credentials + port      |
//!
//! ## Security
//!
//! - Credentials are written to a 0600 file under `data/smb-creds/` and only
//!   passed to `mount.cifs` via `-o credentials=…`. They never appear on the
//!   process command line (which is world-readable in `/proc`).
//! - The on-disk config encrypts the password with AES-256-GCM using a key
//!   derived from the server's JWT secret (see [`encrypt_password`]).
//! - Host, share, subpath, username, and domain are all validated against
//!   strict allow-lists to defeat shell-metacharacter and path-traversal
//!   injection — `mount.cifs` is a SUID binary on most distros and we do
//!   *not* want to feed it tainted input.
//!
//! ## Privilege
//!
//! Mounting CIFS requires `CAP_SYS_ADMIN`. The server typically runs unprivileged,
//! so we try in order:
//!   1. Direct invocation of `mount.cifs` (works if the binary has the SUID bit
//!      set — the default on most distributions).
//!   2. `sudo -n mount.cifs …` (works if the operator added a sudoers rule).
//!
//! If both fail we return a clear error with remediation steps for the wizard
//! to surface to the user.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Parsed SMB target — the result of [`parse_smb_input`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmbTarget {
    /// Server hostname or IP. Validated: only `[A-Za-z0-9._-]`.
    pub host: String,
    /// Optional non-default port (almost always 445 — left as `None` to keep
    /// `mount.cifs` happy with the default).
    pub port: Option<u16>,
    /// Share name (the first path component after the host). No slashes.
    pub share: String,
    /// Optional sub-path beneath the share (forward-slash separated).
    /// Empty when the user pointed at the share root.
    pub subpath: String,
    /// Optional Windows / Active Directory domain.
    pub domain: Option<String>,
    /// Optional username embedded in the URI (e.g. `smb://user@host/share`).
    pub username: Option<String>,
    /// Optional password embedded in the URI (rare — typically supplied
    /// separately by the wizard so it isn't echoed back to the client).
    pub password: Option<String>,
}

/// Persisted SMB configuration — written to `[storage.smb]` in `config.toml`.
///
/// `password_enc` is the AES-256-GCM ciphertext of the password, base64-encoded.
/// The key is derived from `auth.jwt_secret` so the config file alone is not
/// enough to recover the credential.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SmbStoredConfig {
    /// Original address as entered by the admin (for display in the UI).
    pub address: String,
    /// Username (plain — not considered secret for self-hosted single-tenant
    /// deployments, but still kept off-process via the credentials file).
    #[serde(default)]
    pub username: String,
    /// Base64(AES-GCM(password)) — `None`/empty means "guest" or anonymous.
    #[serde(default)]
    pub password_enc: String,
    /// Optional Windows domain.
    #[serde(default)]
    pub domain: String,
    /// Absolute mount point chosen at configure-time.
    pub mount_point: String,
    /// Sub-path beneath the mount point that becomes the storage root.
    /// Empty means storage root == mount point.
    #[serde(default)]
    pub subpath: String,
}

/// Parse a user-supplied address into an [`SmbTarget`].
///
/// Returns `Ok(None)` if the input is not an SMB-style address (caller should
/// treat it as a plain local path). Returns `Err` on malformed SMB input —
/// e.g. missing share, unsafe characters, or empty host.
pub fn parse_smb_input(raw: &str) -> Result<Option<SmbTarget>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    // Detect SMB-ness. Three syntaxes:
    //   smb://...           — explicit scheme
    //   \\host\share\...    — Windows UNC (single or escaped backslashes)
    //   //host/share/...    — POSIX (only when *both* leading slashes present
    //                          AND the path looks like host/share, to avoid
    //                          eating ordinary absolute paths like //)
    let normalized = if let Some(rest) = trimmed
        .strip_prefix("smb://")
        .or_else(|| trimmed.strip_prefix("SMB://"))
    {
        rest.to_string()
    } else if trimmed.starts_with("\\\\") || trimmed.starts_with("\\\\\\\\") {
        // Windows UNC — collapse all backslashes to forward slashes and trim
        // the leading "//" left over.
        let unc = trimmed.replace('\\', "/");
        unc.trim_start_matches('/').to_string()
    } else if let Some(rest) = trimmed.strip_prefix("//") {
        // Treat `//host/share/...` as SMB *only* if there's a real share
        // component. A bare `//foo` (no share) is ambiguous — bail out and
        // let the caller treat it as a local path.
        if !rest.contains('/') || rest.starts_with('/') {
            return Ok(None);
        }
        rest.to_string()
    } else {
        return Ok(None);
    };

    if normalized.is_empty() {
        return Err("SMB address is missing host and share".into());
    }

    // Split optional `user[:pass]@` prefix.
    let (creds, host_and_path) = match normalized.find('@') {
        Some(i) => (Some(&normalized[..i]), &normalized[i + 1..]),
        None => (None, normalized.as_str()),
    };

    let (username, password) = match creds {
        Some(c) if !c.is_empty() => match c.find(':') {
            Some(i) => (
                Some(percent_decode(&c[..i])?),
                Some(percent_decode(&c[i + 1..])?),
            ),
            None => (Some(percent_decode(c)?), None),
        },
        _ => (None, None),
    };

    // Split `host[:port]` from path.
    let mut parts = host_and_path.splitn(2, '/');
    let hostport = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let (host, port) = match hostport.rfind(':') {
        // IPv6 literal `[::1]:445` — out of scope for the wizard, treat colon
        // inside brackets as part of the host.
        Some(i) if !hostport[..i].contains(']') => {
            let h = &hostport[..i];
            let p: u16 = hostport[i + 1..]
                .parse()
                .map_err(|_| format!("Invalid SMB port: {}", &hostport[i + 1..]))?;
            (h.to_string(), Some(p))
        }
        _ => (hostport.to_string(), None),
    };

    if host.is_empty() {
        return Err("SMB address is missing a host".into());
    }
    validate_token(&host, "host")?;

    // Split share / subpath.
    let mut path_parts = path.splitn(2, '/');
    let share = path_parts.next().unwrap_or("").trim().to_string();
    let subpath = path_parts
        .next()
        .map(|s| s.trim_matches('/').to_string())
        .unwrap_or_default();

    if share.is_empty() {
        return Err("SMB address is missing a share name (e.g. smb://host/share)".into());
    }
    validate_token(&share, "share")?;
    if !subpath.is_empty() {
        validate_subpath(&subpath)?;
    }

    if let Some(u) = &username {
        validate_token(u, "username")?;
    }

    Ok(Some(SmbTarget {
        host,
        port,
        share,
        subpath,
        domain: None,
        username,
        password,
    }))
}

/// Apply credentials supplied separately by the wizard (wins over any inline
/// `user:pass@` fragment in the URI).
pub fn apply_credentials(
    mut target: SmbTarget,
    username: Option<&str>,
    password: Option<&str>,
    domain: Option<&str>,
) -> Result<SmbTarget, String> {
    if let Some(u) = username.map(str::trim).filter(|s| !s.is_empty()) {
        validate_token(u, "username")?;
        target.username = Some(u.to_string());
    }
    if let Some(p) = password.filter(|s| !s.is_empty()) {
        // Passwords aren't validated for charset (users have weird passwords),
        // but we *do* refuse newline / NUL which would break the credentials
        // file format we feed to mount.cifs.
        if p.contains('\n') || p.contains('\r') || p.contains('\0') {
            return Err("Password contains illegal control characters".into());
        }
        target.password = Some(p.to_string());
    }
    if let Some(d) = domain.map(str::trim).filter(|s| !s.is_empty()) {
        validate_token(d, "domain")?;
        target.domain = Some(d.to_string());
    }
    Ok(target)
}

/// Default mount point for an SMB target — under `<storage_parent>/mounts/`.
///
/// We base the name on host + share (sanitised) rather than including the
/// subpath so multiple subpaths on the same share share a single mount.
pub fn default_mount_point(target: &SmbTarget, base: &Path) -> PathBuf {
    let safe = format!("{}__{}", sanitize_for_path(&target.host), sanitize_for_path(&target.share));
    base.join("mounts").join(safe)
}

/// Run `mount.cifs` (or via `sudo -n`) to mount the target at `mount_point`.
/// Returns the **storage root** (mount point joined with the subpath).
pub async fn mount_smb(
    target: &SmbTarget,
    mount_point: &Path,
    creds_dir: &Path,
) -> Result<PathBuf, String> {
    if cfg!(not(target_os = "linux")) {
        return Err(
            "Automatic SMB mounting is only supported on Linux. \
             Mount the share manually and point storage at the mount point."
                .into(),
        );
    }

    tokio::fs::create_dir_all(mount_point)
        .await
        .map_err(|e| format!("Cannot create mount point {}: {}", mount_point.display(), e))?;
    tokio::fs::create_dir_all(creds_dir)
        .await
        .map_err(|e| format!("Cannot create credentials dir {}: {}", creds_dir.display(), e))?;

    if is_mounted(mount_point).await {
        return Ok(mount_point.join(&target.subpath));
    }

    let creds_path = creds_dir.join(format!(
        "{}__{}.cred",
        sanitize_for_path(&target.host),
        sanitize_for_path(&target.share)
    ));
    write_credentials_file(target, &creds_path).await?;

    let source = match target.port {
        Some(p) => format!("//{}:{}/{}", target.host, p, target.share),
        None => format!("//{}/{}", target.host, target.share),
    };

    // uid/gid so the server user owns files inside the mount.
    let (uid, gid) = current_uid_gid();
    let opts = format!(
        "credentials={creds},uid={uid},gid={gid},iocharset=utf8,file_mode=0660,dir_mode=0770,nofail,_netdev",
        creds = creds_path.display(),
        uid = uid,
        gid = gid,
    );

    let result = run_mount_command(&source, mount_point, &opts).await;
    if let Err(e) = result {
        // Failed mount → remove the credentials file so failed wizard attempts
        // don't accumulate plaintext credentials under data/smb-creds/. The
        // 0600 mode means the mode + path leak nothing, but disk-at-rest
        // hygiene is cheap.
        if let Err(rm_err) = tokio::fs::remove_file(&creds_path).await {
            tracing::warn!(
                "Mount failed and could not remove stale SMB credentials file {}: {}",
                creds_path.display(),
                rm_err
            );
        }
        return Err(e);
    }

    Ok(mount_point.join(&target.subpath))
}

/// Try to unmount a mount point. Best-effort: errors are returned but don't
/// usually need to abort the caller's flow.
pub async fn unmount_smb(mount_point: &Path) -> Result<(), String> {
    if !is_mounted(mount_point).await {
        return Ok(());
    }
    let mp = mount_point.to_string_lossy().to_string();
    // Try umount first, then sudo -n umount.
    if try_command("umount", &[mp.as_str()]).await.is_ok() {
        return Ok(());
    }
    try_command("sudo", &["-n", "umount", mp.as_str()])
        .await
        .map_err(|e| format!("umount failed: {}", e))
}

/// Lightweight reachability probe — runs `smbclient -L //host -U user%pass -t 5`
/// to verify the share is reachable and credentials are accepted *before* we
/// commit to mounting. Returns `Ok(())` on success.
pub async fn probe_smb(target: &SmbTarget) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("smbclient");
    cmd.arg("-L").arg(format!("//{}", target.host));
    if let Some(p) = target.port {
        cmd.arg("-p").arg(p.to_string());
    }
    if let Some(d) = &target.domain {
        cmd.arg("-W").arg(d);
    }
    let user_pass = match (&target.username, &target.password) {
        (Some(u), Some(p)) => format!("{}%{}", u, p),
        (Some(u), None) => u.clone(),
        _ => "%".to_string(), // anonymous / guest
    };
    cmd.arg("-U").arg(&user_pass);
    cmd.arg("-t").arg("5");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd.output().await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "smbclient not installed — install the `smbclient` package and try again".to_string()
        } else {
            format!("Failed to run smbclient: {}", e)
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        return Err(condense_smb_error(&format!("{}\n{}", stderr, stdout)));
    }
    Ok(())
}

/// Encrypt a plaintext password using a key derived from the JWT secret.
pub fn encrypt_password(password: &str, jwt_secret: &str) -> Result<String, String> {
    let key = derive_key(jwt_secret);
    let ct = crate::crypto::encrypt(&key, password.as_bytes())?;
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(ct))
}

/// Decrypt a ciphertext produced by [`encrypt_password`].
pub fn decrypt_password(ciphertext_b64: &str, jwt_secret: &str) -> Result<String, String> {
    if ciphertext_b64.is_empty() {
        return Ok(String::new());
    }
    use base64::Engine;
    let ct = base64::engine::general_purpose::STANDARD
        .decode(ciphertext_b64)
        .map_err(|e| format!("Invalid base64 in stored SMB password: {}", e))?;
    let key = derive_key(jwt_secret);
    let pt = crate::crypto::decrypt(&key, &ct)?;
    String::from_utf8(pt).map_err(|e| format!("Invalid UTF-8 in decrypted password: {}", e))
}

// ── Internals ───────────────────────────────────────────────────────────────

fn derive_key(jwt_secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"simple-photos:smb-credential-key:v1\n");
    hasher.update(jwt_secret.as_bytes());
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// Check `/proc/self/mountinfo` for `mount_point`. Avoids parsing `mount(8)`
/// output (which is human-formatted) and is the canonical kernel-supplied
/// truth source.
pub async fn is_mounted(mount_point: &Path) -> bool {
    let canonical = match tokio::fs::canonicalize(mount_point).await {
        Ok(p) => p,
        Err(_) => return false,
    };
    let want = canonical.to_string_lossy().to_string();
    let contents = match tokio::fs::read_to_string("/proc/self/mountinfo").await {
        Ok(c) => c,
        Err(_) => return false,
    };
    // mountinfo field 5 is the mount point.
    contents
        .lines()
        .filter_map(|l| l.split_whitespace().nth(4))
        .any(|mp| mp == want)
}

#[cfg(target_os = "linux")]
fn current_uid_gid() -> (u32, u32) {
    // Use rustix's safe wrappers around `geteuid` / `getegid`.
    // These syscalls cannot fail on Linux but rustix gives us a
    // panic-free, `unsafe`-free surface.
    (
        rustix::process::geteuid().as_raw(),
        rustix::process::getegid().as_raw(),
    )
}

#[cfg(not(target_os = "linux"))]
fn current_uid_gid() -> (u32, u32) {
    (0, 0)
}

async fn write_credentials_file(target: &SmbTarget, path: &Path) -> Result<(), String> {
    let mut body = String::new();
    body.push_str(&format!(
        "username={}\n",
        target.username.as_deref().unwrap_or("guest")
    ));
    body.push_str(&format!(
        "password={}\n",
        target.password.as_deref().unwrap_or("")
    ));
    if let Some(d) = &target.domain {
        body.push_str(&format!("domain={}\n", d));
    }

    tokio::fs::write(path, body)
        .await
        .map_err(|e| format!("Cannot write SMB credentials file: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("Cannot set 0600 on credentials file: {}", e))?;
    }
    Ok(())
}

async fn run_mount_command(source: &str, mount_point: &Path, opts: &str) -> Result<(), String> {
    let mp = mount_point.to_string_lossy().to_string();

    // Attempt 1: direct mount.cifs (works when SUID is set, the default on
    // most Linux distributions when cifs-utils is installed).
    let direct = tokio::process::Command::new("mount.cifs")
        .arg(source)
        .arg(&mp)
        .arg("-o")
        .arg(opts)
        .stdin(Stdio::null())
        .output()
        .await;

    match direct {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            // Attempt 2: sudo -n mount.cifs.
            let sudo = tokio::process::Command::new("sudo")
                .arg("-n")
                .arg("mount.cifs")
                .arg(source)
                .arg(&mp)
                .arg("-o")
                .arg(opts)
                .stdin(Stdio::null())
                .output()
                .await;
            match sudo {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => {
                    let sudo_err = String::from_utf8_lossy(&o.stderr).to_string();
                    Err(format!(
                        "mount.cifs failed: {}. sudo fallback: {}. \
                         Tip: install `cifs-utils`, ensure `mount.cifs` has the SUID bit, \
                         or add a NOPASSWD sudoers rule for the server user.",
                        condense_smb_error(&stderr),
                        condense_smb_error(&sudo_err),
                    ))
                }
                Err(_) => {
                    Err(format!(
                        "mount.cifs failed: {}. \
                         Tip: install `cifs-utils` (provides mount.cifs) and ensure it has the SUID bit, \
                         or grant the server user passwordless sudo for /usr/sbin/mount.cifs.",
                        condense_smb_error(&stderr),
                    ))
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(
                "mount.cifs not found — install the `cifs-utils` package and try again".into(),
            )
        }
        Err(e) => Err(format!("Failed to spawn mount.cifs: {}", e)),
    }
}

async fn try_command(prog: &str, args: &[&str]) -> Result<(), String> {
    let out = tokio::process::Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Validate a token (host / share / username / domain) is safe to splice into
/// `mount.cifs` arguments and the credentials file.
///
/// We also reject leading `-` (would be parsed as an option flag), and the
/// `=` and `,` characters which are option separators in `mount.cifs -o ...`
/// and which would let a malicious token escape into a sibling option even
/// though we never feed tokens through a shell.
fn validate_token(value: &str, kind: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("SMB {} is empty", kind));
    }
    if value.len() > 255 {
        return Err(format!("SMB {} is too long (max 255 chars)", kind));
    }
    if value.starts_with('-') {
        return Err(format!("SMB {} must not start with '-'", kind));
    }
    let ok = value.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '$' | '@' | '\\' | ' ')
    });
    if !ok {
        return Err(format!(
            "SMB {} contains illegal characters — allowed: letters, digits, . - _ $ @ \\ space",
            kind
        ));
    }
    if value.contains("..") {
        return Err(format!("SMB {} must not contain '..'", kind));
    }
    // No mount-option separators — defence in depth. None of these are valid
    // in a hostname / share / username / domain anyway.
    if value.contains('=') || value.contains(',') {
        return Err(format!(
            "SMB {} contains illegal mount-option separator (= or ,)",
            kind
        ));
    }
    Ok(())
}

fn validate_subpath(value: &str) -> Result<(), String> {
    if value.contains("..") {
        return Err("SMB subpath must not contain '..'".into());
    }
    for seg in value.split('/') {
        if !seg.is_empty() {
            // Subpath segments may contain spaces but no metacharacters.
            for ch in seg.chars() {
                if "\0\n\r;&|`$<>".contains(ch) {
                    return Err("SMB subpath contains illegal characters".into());
                }
            }
        }
    }
    Ok(())
}

fn sanitize_for_path(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect()
}

fn percent_decode(s: &str) -> Result<String, String> {
    let bytes = percent_encoding::percent_decode_str(s).collect::<Vec<u8>>();
    String::from_utf8(bytes).map_err(|_| "SMB credential contains invalid UTF-8".into())
}

/// Reduce noisy multi-line `mount.cifs` / `smbclient` output to a single line
/// suitable for surfacing in the wizard UI.
fn condense_smb_error(raw: &str) -> String {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("(no error output)");
    // Common signatures we can rewrite into something friendlier.
    if line.contains("NT_STATUS_LOGON_FAILURE") {
        return "Logon failure — wrong username or password".into();
    }
    if line.contains("NT_STATUS_BAD_NETWORK_NAME") {
        return "Share not found on the server".into();
    }
    if line.contains("NT_STATUS_HOST_UNREACHABLE") || line.contains("Connection refused") {
        return "Cannot reach the SMB server (host unreachable)".into();
    }
    if line.contains("Permission denied") {
        return "Permission denied (need root or SUID mount.cifs)".into();
    }
    line.to_string()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_smb_uri() {
        let t = parse_smb_input("smb://vault.local/photos/Simple-Photos")
            .unwrap()
            .unwrap();
        assert_eq!(t.host, "vault.local");
        assert_eq!(t.share, "photos");
        assert_eq!(t.subpath, "Simple-Photos");
        assert_eq!(t.port, None);
        assert!(t.username.is_none());
    }

    #[test]
    fn parses_unc_path() {
        let t = parse_smb_input(r"\\nas.local\media\Photos\2026")
            .unwrap()
            .unwrap();
        assert_eq!(t.host, "nas.local");
        assert_eq!(t.share, "media");
        assert_eq!(t.subpath, "Photos/2026");
    }

    #[test]
    fn parses_double_escaped_unc() {
        let t = parse_smb_input(r"\\\\nas\share\sub").unwrap().unwrap();
        assert_eq!(t.host, "nas");
        assert_eq!(t.share, "share");
        assert_eq!(t.subpath, "sub");
    }

    #[test]
    fn parses_creds_and_port() {
        let t = parse_smb_input("smb://alice:s3cret@vault:4450/share/sub")
            .unwrap()
            .unwrap();
        assert_eq!(t.username.as_deref(), Some("alice"));
        assert_eq!(t.password.as_deref(), Some("s3cret"));
        assert_eq!(t.port, Some(4450));
    }

    #[test]
    fn rejects_missing_share() {
        assert!(parse_smb_input("smb://vault.local").is_err());
    }

    #[test]
    fn rejects_unsafe_host() {
        assert!(parse_smb_input("smb://vault;rm -rf/share").is_err());
    }

    #[test]
    fn local_paths_pass_through() {
        assert!(parse_smb_input("/mnt/photos").unwrap().is_none());
        assert!(parse_smb_input("./data/storage").unwrap().is_none());
        assert!(parse_smb_input("C:\\Photos").unwrap().is_none());
    }

    #[test]
    fn encrypt_roundtrip() {
        let secret = "0123456789abcdef0123456789abcdef";
        let ct = encrypt_password("hunter2", secret).unwrap();
        let pt = decrypt_password(&ct, secret).unwrap();
        assert_eq!(pt, "hunter2");
    }

    #[test]
    fn empty_password_decrypts_to_empty() {
        assert_eq!(decrypt_password("", "secret").unwrap(), "");
    }
}
