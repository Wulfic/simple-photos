//! Storage and directory browsing admin endpoints.
//!
//! - `GET  /api/admin/storage`        — current storage root, quota, and SMB config (if any).
//! - `PUT  /api/admin/storage`        — update storage root. Accepts either a plain
//!   filesystem path or an SMB descriptor (host/share/credentials); if an SMB
//!   descriptor is provided we mount the share first and point the root at the
//!   resulting mount point.
//! - `POST /api/admin/storage/test-smb` — dry-run an SMB connection (probes
//!   host + auth via `smbclient -L`) without mounting.
//! - `GET  /api/admin/browse`         — list directories on the server filesystem
//!   so the admin can pick a storage root from the web UI.
//!
//! Path traversal attacks are blocked by `sanitize::validate_relative_path()`.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::admin::require_admin;
use super::smb::{self, SmbStoredConfig};

#[derive(Debug, Serialize)]
pub struct StorageResponse {
    pub storage_path: String,
    pub message: String,
    /// Currently configured SMB share, if any. Password is never returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smb: Option<SmbInfo>,
}

#[derive(Debug, Serialize)]
pub struct SmbInfo {
    pub address: String,
    pub username: String,
    pub domain: String,
    pub mount_point: String,
    pub subpath: String,
    /// Whether the mount is currently live (kernel reports it mounted).
    pub mounted: bool,
}

fn smb_info(cfg: &SmbStoredConfig, mounted: bool) -> SmbInfo {
    SmbInfo {
        address: cfg.address.clone(),
        username: cfg.username.clone(),
        domain: cfg.domain.clone(),
        mount_point: cfg.mount_point.clone(),
        subpath: cfg.subpath.clone(),
        mounted,
    }
}

/// Admin-only: Get the current storage path.
///
/// GET /api/admin/storage
pub async fn get_storage(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<StorageResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Lock-free read via ArcSwap — no async lock needed.
    let root = state.storage_root.load();
    let path = root.display().to_string();

    let smb = match &state.config.storage.smb {
        Some(cfg) => {
            let mounted = smb::is_mounted(std::path::Path::new(&cfg.mount_point)).await;
            Some(smb_info(cfg, mounted))
        }
        None => None,
    };

    Ok(Json(StorageResponse {
        storage_path: path,
        message: "Current storage path".into(),
        smb,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateStorageRequest {
    /// Plain filesystem path. Mutually exclusive with `smb`.
    #[serde(default)]
    pub path: Option<String>,
    /// SMB descriptor — when present, the server mounts the share first and
    /// uses the mount point (joined with the optional subpath) as the new
    /// storage root.
    #[serde(default)]
    pub smb: Option<SmbConfigureRequest>,
}

#[derive(Debug, Deserialize)]
pub struct SmbConfigureRequest {
    /// Address in any of the supported formats — `smb://...`, `\\host\share`,
    /// or `//host/share/sub`.
    pub address: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    /// Optional override for the mount point. Defaults to `data/mounts/<host>__<share>`.
    #[serde(default)]
    pub mount_point: Option<String>,
}

/// Admin-only: Update the storage root directory.
///
/// PUT /api/admin/storage
///
/// Two modes:
///
/// 1. **Local path** (`{ "path": "/some/dir" }`): validates the path exists
///    (or creates it), updates in-memory config, and writes the change back to
///    `config.toml`. Any previously configured SMB share is unmounted and
///    cleared from the config.
/// 2. **SMB share** (`{ "smb": { "address": "smb://...", ... } }`): mounts the
///    share via `mount.cifs` at `data/mounts/<host>__<share>`, sets the
///    storage root to that mount point (joined with the optional subpath),
///    encrypts the password, and persists everything to `[storage.smb]` in
///    `config.toml` so the share is remounted automatically on every restart.
pub async fn update_storage(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<UpdateStorageRequest>,
) -> Result<Json<StorageResponse>, AppError> {
    require_admin(&state, &auth).await?;

    if req.path.is_some() && req.smb.is_some() {
        return Err(AppError::BadRequest(
            "Specify either `path` or `smb`, not both".into(),
        ));
    }

    if let Some(smb_req) = req.smb {
        return configure_smb_storage(&state, &headers, &auth.user_id, smb_req).await;
    }

    let raw_path = req.path.ok_or_else(|| {
        AppError::BadRequest("Request must include either `path` or `smb`".into())
    })?;
    configure_local_storage(&state, &headers, &auth.user_id, raw_path).await
}

async fn configure_local_storage(
    state: &AppState,
    headers: &HeaderMap,
    user_id: &str,
    raw_path: String,
) -> Result<Json<StorageResponse>, AppError> {
    let new_path = PathBuf::from(&raw_path);

    // Security: reject paths that try to escape (e.g. contain "..")
    if raw_path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    // Create directory if it doesn't exist
    if !new_path.exists() {
        tokio::fs::create_dir_all(&new_path).await.map_err(|e| {
            AppError::BadRequest(format!(
                "Cannot create directory '{}': {}",
                new_path.display(),
                e
            ))
        })?;
    }

    // Verify it's actually a directory
    let meta = tokio::fs::metadata(&new_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot access '{}': {}", new_path.display(), e))
    })?;
    if !meta.is_dir() {
        return Err(AppError::BadRequest(format!(
            "'{}' is not a directory",
            new_path.display()
        )));
    }

    // Test write permissions by creating and removing a temp file
    let test_file = new_path.join(".simple-photos-write-test");
    tokio::fs::write(&test_file, b"test").await.map_err(|e| {
        AppError::BadRequest(format!(
            "Directory '{}' is not writable: {}",
            new_path.display(),
            e
        ))
    })?;
    let _ = tokio::fs::remove_file(&test_file).await;

    // If we're switching away from an SMB mount, try to unmount it cleanly.
    let previous_smb = state.config.storage.smb.clone();

    // Atomically swap the storage root (lock-free for readers).
    state
        .storage_root
        .store(std::sync::Arc::new(new_path.clone()));

    // Persist to config.toml — clears any [storage.smb] section.
    let path_clone = raw_path.clone();
    if let Err(e) =
        tokio::task::spawn_blocking(move || update_config_toml_storage(&path_clone, None))
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking join error: {e}")))
    {
        tracing::warn!("Failed to persist storage path to config.toml: {}", e);
    }

    // Best-effort unmount of the previous SMB share (after persisting so a
    // failure here doesn't leave the config in a half-state).
    if let Some(prev) = previous_smb {
        let mp = std::path::PathBuf::from(&prev.mount_point);
        if let Err(e) = smb::unmount_smb(&mp).await {
            tracing::warn!(
                "Could not unmount previous SMB share at {}: {}",
                mp.display(),
                e
            );
        }
    }

    audit::log(
        state,
        AuditEvent::AdminAction,
        Some(user_id),
        headers,
        Some(serde_json::json!({
            "action": "update_storage_path",
            "new_path": raw_path,
        })),
    )
    .await;

    tracing::info!("Storage path updated to: {}", raw_path);

    Ok(Json(StorageResponse {
        storage_path: raw_path,
        message: "Storage path updated successfully".into(),
        smb: None,
    }))
}

async fn configure_smb_storage(
    state: &AppState,
    headers: &HeaderMap,
    user_id: &str,
    req: SmbConfigureRequest,
) -> Result<Json<StorageResponse>, AppError> {
    let target = smb::parse_smb_input(&req.address)
        .map_err(AppError::BadRequest)?
        .ok_or_else(|| {
            AppError::BadRequest(
                "Address is not a recognized SMB URI (try smb://host/share/...)".into(),
            )
        })?;
    let target = smb::apply_credentials(
        target,
        req.username.as_deref(),
        req.password.as_deref(),
        req.domain.as_deref(),
    )
    .map_err(AppError::BadRequest)?;

    // Choose mount point. Custom override is allowed but must be an absolute
    // path under the workspace (no ".." traversal).
    let mount_point = match req.mount_point.as_ref().filter(|s| !s.trim().is_empty()) {
        Some(custom) => {
            if custom.contains("..") {
                return Err(AppError::BadRequest(
                    "Mount point must not contain '..'".into(),
                ));
            }
            PathBuf::from(custom)
        }
        None => smb::default_mount_point(&target, std::path::Path::new("data")),
    };

    let creds_dir = std::path::PathBuf::from("data/smb-creds");

    // mount.cifs is invoked as root (via SUID or sudo) and may resolve
    // relative paths against a different CWD than the server. Pass absolute
    // paths so the credentials file and mount point are unambiguous.
    let cwd = std::env::current_dir()
        .map_err(|e| AppError::Internal(format!("Cannot read server CWD: {e}")))?;
    let mount_point = if mount_point.is_absolute() {
        mount_point
    } else {
        cwd.join(&mount_point)
    };
    let creds_dir = if creds_dir.is_absolute() {
        creds_dir
    } else {
        cwd.join(&creds_dir)
    };

    let storage_root = smb::mount_smb(&target, &mount_point, &creds_dir)
        .await
        .map_err(AppError::BadRequest)?;

    // Verify the resolved storage root is writable.
    tokio::fs::create_dir_all(&storage_root)
        .await
        .map_err(|e| AppError::BadRequest(format!("Cannot create storage subdir on share: {e}")))?;
    let test_file = storage_root.join(".simple-photos-write-test");
    tokio::fs::write(&test_file, b"test").await.map_err(|e| {
        AppError::BadRequest(format!(
            "SMB share mounted but is not writable as the server user: {e}"
        ))
    })?;
    let _ = tokio::fs::remove_file(&test_file).await;

    // Encrypt the password for at-rest storage.
    let password_enc = match target.password.as_deref() {
        Some(pw) if !pw.is_empty() => smb::encrypt_password(pw, &state.config.auth.jwt_secret)
            .map_err(|e| AppError::Internal(format!("Failed to encrypt SMB password: {e}")))?,
        _ => String::new(),
    };

    let stored = SmbStoredConfig {
        address: req.address.clone(),
        username: target.username.clone().unwrap_or_default(),
        password_enc,
        domain: target.domain.clone().unwrap_or_default(),
        mount_point: mount_point.to_string_lossy().into_owned(),
        subpath: target.subpath.clone(),
    };

    // Atomically swap storage root.
    state
        .storage_root
        .store(std::sync::Arc::new(storage_root.clone()));

    let stored_for_persist = stored.clone();
    let new_root_str = storage_root.to_string_lossy().into_owned();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        update_config_toml_storage(&new_root_str, Some(&stored_for_persist))
    })
    .await
    .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking join error: {e}")))
    {
        tracing::warn!("Failed to persist SMB config to config.toml: {}", e);
    }

    audit::log(
        state,
        AuditEvent::AdminAction,
        Some(user_id),
        headers,
        Some(serde_json::json!({
            "action": "configure_smb_storage",
            "address": req.address,
            "mount_point": stored.mount_point,
        })),
    )
    .await;

    tracing::info!(
        "SMB share mounted at {} (root: {})",
        stored.mount_point,
        storage_root.display()
    );

    let mounted = smb::is_mounted(std::path::Path::new(&stored.mount_point)).await;
    let info = smb_info(&stored, mounted);

    Ok(Json(StorageResponse {
        storage_path: storage_root.to_string_lossy().into_owned(),
        message: "SMB share mounted and storage path updated".into(),
        smb: Some(info),
    }))
}

/// Admin-only: dry-run an SMB connection without mounting.
///
/// POST /api/admin/storage/test-smb
///
/// Returns 200 with `{ ok: true }` on success, or 400 with a descriptive
/// error message on failure (auth failure, host unreachable, missing
/// `smbclient`, etc.).
pub async fn test_smb(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SmbConfigureRequest>,
) -> Result<Json<TestSmbResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let target = smb::parse_smb_input(&req.address)
        .map_err(AppError::BadRequest)?
        .ok_or_else(|| {
            AppError::BadRequest(
                "Address is not a recognized SMB URI (try smb://host/share/...)".into(),
            )
        })?;
    let target = smb::apply_credentials(
        target,
        req.username.as_deref(),
        req.password.as_deref(),
        req.domain.as_deref(),
    )
    .map_err(AppError::BadRequest)?;

    smb::probe_smb(&target)
        .await
        .map_err(AppError::BadRequest)?;

    Ok(Json(TestSmbResponse {
        ok: true,
        message: format!("Successfully reached {}", target.host),
    }))
}

#[derive(Debug, Serialize)]
pub struct TestSmbResponse {
    pub ok: bool,
    pub message: String,
}

/// Read config.toml, update `[storage].root`, and (optionally) the
/// `[storage.smb]` table.  When `smb` is `None`, any existing SMB section is
/// removed so a switch from SMB → local doesn't leave stale credentials behind.
pub fn update_config_toml_storage(
    new_root: &str,
    smb: Option<&SmbStoredConfig>,
) -> anyhow::Result<()> {
    let config_path =
        std::env::var("SIMPLE_PHOTOS_CONFIG").unwrap_or_else(|_| "config.toml".into());
    let contents = std::fs::read_to_string(&config_path)?;
    let mut doc: toml::Table = contents.parse()?;

    if let Some(storage) = doc.get_mut("storage").and_then(|v| v.as_table_mut()) {
        storage.insert(
            "root".to_string(),
            toml::Value::String(new_root.to_string()),
        );

        match smb {
            Some(cfg) => {
                let mut tbl = toml::map::Map::new();
                tbl.insert("address".into(), toml::Value::String(cfg.address.clone()));
                tbl.insert("username".into(), toml::Value::String(cfg.username.clone()));
                tbl.insert(
                    "password_enc".into(),
                    toml::Value::String(cfg.password_enc.clone()),
                );
                tbl.insert("domain".into(), toml::Value::String(cfg.domain.clone()));
                tbl.insert(
                    "mount_point".into(),
                    toml::Value::String(cfg.mount_point.clone()),
                );
                tbl.insert("subpath".into(), toml::Value::String(cfg.subpath.clone()));
                storage.insert("smb".to_string(), toml::Value::Table(tbl));
            }
            None => {
                storage.remove("smb");
            }
        }
    }

    std::fs::write(&config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

// ── Directory Browser ───────────────────────────────────────────────────────

/// Query parameters for browsing server-side directories during storage setup.
#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BrowseResponse {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub directories: Vec<DirEntry>,
    pub writable: bool,
}

#[derive(Debug, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
}

/// Admin-only: Browse server filesystem directories.
///
/// GET /api/admin/browse?path=/some/path
///
/// Returns subdirectories only (no files) for the file browser UI.
/// Defaults to the current storage root if no path is given.
///
/// **Security**: even though this endpoint is admin-only, we refuse to browse
/// kernel/process pseudo-filesystems (`/proc`, `/sys`, `/dev`) and audit every
/// access. We also re-check the canonical path after `canonicalize()` to
/// defeat symlinks that escape into otherwise-blocked subtrees.
pub async fn browse_directory(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let browse_path = match &query.path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => (**state.storage_root.load()).clone(),
    };

    // Security: reject ".." traversal
    let path_str = browse_path.display().to_string();
    if path_str.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    if is_blocked_browse_path(&browse_path) {
        audit::log(
            &state,
            AuditEvent::AdminAction,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({
                "action": "browse_directory_blocked",
                "path": browse_path.display().to_string(),
            })),
        )
        .await;
        return Err(AppError::Forbidden(
            "Browsing kernel/process pseudo-filesystems is not allowed".into(),
        ));
    }

    // Canonicalize to get absolute path
    let canonical = tokio::fs::canonicalize(&browse_path).await.map_err(|e| {
        AppError::BadRequest(format!(
            "Cannot resolve path '{}': {}",
            browse_path.display(),
            e
        ))
    })?;

    // Re-check the canonical form too \u2014 defeats symlinks pointing into /proc et al.
    if is_blocked_browse_path(&canonical) {
        return Err(AppError::Forbidden(
            "Resolved path lies inside a blocked filesystem".into(),
        ));
    }

    let meta = tokio::fs::metadata(&canonical).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot access '{}': {}", canonical.display(), e))
    })?;

    if !meta.is_dir() {
        return Err(AppError::BadRequest(format!(
            "'{}' is not a directory",
            canonical.display()
        )));
    }

    // Read directory entries — directories only, skip hidden ones
    let mut directories = Vec::new();
    let mut entries = tokio::fs::read_dir(&canonical).await.map_err(|e| {
        AppError::BadRequest(format!(
            "Cannot read directory '{}': {}",
            canonical.display(),
            e
        ))
    })?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if let Ok(ft) = entry.file_type().await {
            if ft.is_dir() {
                let full_path = entry.path().display().to_string();
                directories.push(DirEntry {
                    name,
                    path: full_path,
                });
            }
        }
    }

    directories.sort_by_key(|a| a.name.to_lowercase());

    // Check if writable
    let test_file = canonical.join(".simple-photos-write-test");
    let writable = tokio::fs::write(&test_file, b"test").await.is_ok();
    if writable {
        let _ = tokio::fs::remove_file(&test_file).await;
    }

    let parent_path = canonical.parent().map(|p| p.display().to_string());

    Ok(Json(BrowseResponse {
        current_path: canonical.display().to_string(),
        parent_path,
        directories,
        writable,
    }))
}

/// Refuse to browse kernel and process pseudo-filesystems. These are not
/// useful storage targets and exposing their contents (even read-only, even
/// to admins) leaks sensitive process and hardware information that has no
/// business flowing through an HTTP API.
fn is_blocked_browse_path(p: &std::path::Path) -> bool {
    const BLOCKED_PREFIXES: &[&str] = &["/proc", "/sys", "/dev"];
    let s = p.to_string_lossy();
    BLOCKED_PREFIXES
        .iter()
        .any(|prefix| s == *prefix || s.starts_with(&format!("{prefix}/")))
}

// ── Native folder-picker ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PickDirectoryResponse {
    pub path: String,
}

/// Admin-only: spawn a native OS folder-picker dialog on the server machine
/// and return the selected path.
///
/// GET /api/admin/pick-directory
///
/// On Windows shows the native Win32 folder browser; on Linux tries `zenity`
/// (GNOME/GTK) then `kdialog` (KDE). Returns 400 when no dialog is available
/// (headless host / service session) or the user cancelled. The frontend
/// falls back to the in-browser FolderBrowserModal in that case.
pub async fn pick_directory(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
) -> Result<Json<PickDirectoryResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let path = spawn_native_picker()
        .await
        .map_err(|e| AppError::BadRequest(format!("native_picker_unavailable: {e}")))?;

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "pick_directory",
            "selected": path,
        })),
    )
    .await;

    Ok(Json(PickDirectoryResponse { path }))
}

/// Launch a native OS folder-selection dialog on the server machine.
///
/// On Windows this shows the standard Win32 folder browser; on Linux it tries
/// `zenity` (GTK/GNOME) then `kdialog` (KDE). Returns the selected path, or an
/// error string when the dialog is unavailable (headless / service session) or
/// the user cancelled.
async fn spawn_native_picker() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        spawn_native_picker_windows().await
    }
    #[cfg(not(target_os = "windows"))]
    {
        spawn_native_picker_unix().await
    }
}

/// Windows native folder picker via PowerShell + WinForms `FolderBrowserDialog`.
///
/// The dialog must run in a single-threaded apartment (`-Sta`). We refuse to
/// show it when the process has no interactive desktop (e.g. running as a
/// Windows Service in session 0) — `ShowDialog` there would block forever —
/// returning exit code 2 so the frontend cleanly falls back to its in-browser
/// directory browser.
#[cfg(target_os = "windows")]
async fn spawn_native_picker_windows() -> Result<String, String> {
    const PS: &str = r#"
if (-not [Environment]::UserInteractive) { exit 2 }
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$owner = New-Object System.Windows.Forms.Form
$owner.TopMost = $true
$owner.ShowInTaskbar = $false
$owner.Opacity = 0
$owner.Show()
$dlg = New-Object System.Windows.Forms.FolderBrowserDialog
$dlg.Description = 'Select Photo Storage Folder'
$dlg.ShowNewFolderButton = $true
$result = $dlg.ShowDialog($owner)
$owner.Close()
if ($result -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::Out.Write($dlg.SelectedPath)
    exit 0
}
exit 1
"#;

    let out = tokio::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Sta",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            PS,
        ])
        .output()
        .await
        .map_err(|e| format!("native_picker_unavailable: failed to launch powershell: {e}"))?;

    match out.status.code() {
        Some(0) => {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if p.is_empty() {
                Err("native_picker_unavailable: empty selection".into())
            } else {
                Ok(p)
            }
        }
        Some(1) => Err("cancelled".into()),
        // 2 = no interactive desktop; anything else = unexpected failure.
        other => Err(format!(
            "native_picker_unavailable: dialog exited with code {other:?}"
        )),
    }
}

/// Linux native folder picker. Tries `zenity` (GTK/GNOME) then `kdialog` (KDE).
#[cfg(not(target_os = "windows"))]
async fn spawn_native_picker_unix() -> Result<String, String> {
    // Without a graphical display environment neither zenity nor kdialog can
    // show a window. Both would exit with code 1 — identical to user-cancel —
    // making them undetectable on the frontend. Bail out immediately so the
    // frontend knows to fall back to the in-browser directory browser.
    let has_display = std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
    if !has_display {
        return Err(
            "native_picker_unavailable: no graphical display found (DISPLAY / WAYLAND_DISPLAY not set)".into(),
        );
    }

    // zenity --file-selection --directory
    match tokio::process::Command::new("zenity")
        .args([
            "--file-selection",
            "--directory",
            "--title=Select Photo Storage Folder",
        ])
        .output()
        .await
    {
        Ok(out) if out.status.success() => {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Ok(p);
            }
        }
        Ok(out) if out.status.code() == Some(1) => {
            // User clicked Cancel
            return Err("cancelled".into());
        }
        _ => {} // zenity not installed or no display — try next
    }

    // kdialog --getexistingdirectory
    match tokio::process::Command::new("kdialog")
        .args([
            "--getexistingdirectory",
            "/",
            "--title",
            "Select Photo Storage Folder",
        ])
        .output()
        .await
    {
        Ok(out) if out.status.success() => {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Ok(p);
            }
        }
        Ok(out) if out.status.code() == Some(1) => {
            return Err("cancelled".into());
        }
        _ => {}
    }

    Err(
        "No native dialog tool found. Install zenity (GNOME/GTK) or kdialog (KDE), \
         or ensure the server process has access to a display."
            .into(),
    )
}

// ── Sentinel resolver ────────────────────────────────────────────────────────
//
// The browser's `showDirectoryPicker()` shows the native OS folder-picker (the
// same dialog as file uploads) but, for security, never exposes the absolute
// filesystem path to JavaScript. To bridge this gap the frontend:
//   1. Calls `showDirectoryPicker()` → gets a FileSystemDirectoryHandle.
//   2. Writes a tiny sentinel file into that directory via the handle.
//   3. Calls this endpoint with the sentinel's filename.
//   4. The server locates the file on disk and returns its parent directory.
//   5. The frontend deletes the sentinel.
//
// Sentinel names are validated to the form `sp-picker-<uuid>.tmp` to prevent
// the endpoint from being used as an arbitrary filesystem search tool.

#[derive(Debug, Deserialize)]
pub struct SentinelQuery {
    pub filename: String,
}

#[derive(Debug, Serialize)]
pub struct SentinelResponse {
    pub path: String,
}

/// Validate that a sentinel filename matches `sp-picker-<uuid>.tmp`.
fn is_valid_sentinel_name(name: &str) -> bool {
    let Some(mid) = name
        .strip_prefix("sp-picker-")
        .and_then(|s| s.strip_suffix(".tmp"))
    else {
        return false;
    };
    // UUID: 32-36 chars of lowercase hex + hyphens, no slashes or dots
    mid.len() >= 32
        && mid.len() <= 36
        && mid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && !mid.contains('/')
        && !mid.contains('\\')
        && !mid.contains('.')
}

/// Admin-only: locate a sentinel file and return its parent directory path.
///
/// GET /api/admin/resolve-sentinel?filename=sp-picker-UUID.tmp
///
/// Used by the frontend's `showDirectoryPicker()` flow to recover the absolute
/// server-side path of a folder the user chose via the native OS dialog.
pub async fn resolve_storage_sentinel(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<SentinelQuery>,
) -> Result<Json<SentinelResponse>, AppError> {
    require_admin(&state, &auth).await?;

    if !is_valid_sentinel_name(&query.filename) {
        return Err(AppError::BadRequest(
            "Invalid sentinel filename — expected sp-picker-<uuid>.tmp".into(),
        ));
    }

    // Search common user-accessible locations (non-system roots only).
    // Non-existent roots are silently skipped by `find`.
    const SEARCH_ROOTS: &[&str] = &[
        "/home",
        "/mnt",
        "/media",
        "/run/media",
        "/data",
        "/storage",
        "/srv",
    ];

    let filename = query.filename.clone();
    let mut cmd = tokio::process::Command::new("find");
    for root in SEARCH_ROOTS {
        cmd.arg(root);
    }
    cmd.args(["-maxdepth", "8", "-name", &filename, "-type", "f"]);
    // Suppress "Permission denied" noise from inaccessible subdirectories.
    cmd.stderr(std::process::Stdio::null());

    let output = tokio::time::timeout(std::time::Duration::from_secs(15), cmd.output())
        .await
        .map_err(|_| {
            AppError::BadRequest(
                "Directory search timed out — try entering the path manually.".into(),
            )
        })?
        .map_err(|e| AppError::Internal(format!("find command error: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = stdout.lines().next().ok_or_else(|| {
        AppError::BadRequest(
            "Sentinel file not found. The selected folder may not be readable by \
             the server process, or it is outside the searched locations \
             (/home, /mnt, /media, /data, /srv). Enter the path manually instead."
                .into(),
        )
    })?;

    let file_path = std::path::PathBuf::from(first.trim());
    let parent = file_path
        .parent()
        .ok_or_else(|| AppError::Internal("Sentinel has no parent path".into()))?;

    Ok(Json(SentinelResponse {
        path: parent.display().to_string(),
    }))
}
