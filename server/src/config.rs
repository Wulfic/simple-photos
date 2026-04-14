//! Application configuration, loaded from a TOML file with environment variable overrides.
//!
//! Config file: `$SIMPLE_PHOTOS_CONFIG` or `./config.toml`
//! Override any field: `SIMPLE_PHOTOS_<SECTION>_<KEY>=value`

use serde::Deserialize;
use std::path::PathBuf;

/// Top-level configuration, deserialized from `config.toml`.
/// Each nested struct corresponds to a `[section]` in the TOML file.
#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub auth: AuthConfig,
    pub web: WebConfig,
    #[serde(default)]
    pub backup: BackupConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub scan: ScanConfig,
}

/// HTTP(S) listener settings.
#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Public base URL (e.g. "https://photos.example.com"). Used in backup
    /// broadcast and anywhere an absolute URL is needed.
    pub base_url: String,
    /// Whether to trust `X-Forwarded-For` / `X-Real-IP` headers for
    /// rate-limiting and audit logging. Set to `true` ONLY when behind
    /// a reverse proxy (nginx, Caddy, etc.) that sets these headers.
    /// When `false` (default), the server ignores proxy headers and uses
    /// the TCP peer address for rate-limiting.
    #[serde(default)]
    pub trust_proxy: bool,
    /// Dedicated LAN discovery port. Every Simple Photos server runs a tiny
    /// HTTP listener on this port so clients can discover servers by scanning
    /// a single well-known port instead of probing many ports per IP.
    /// The listener responds with the server's name, version, actual HTTP
    /// port, and mode — allowing instant pairing.
    /// Default: 3301. Set to 0 to disable the discovery listener.
    #[serde(default = "ServerConfig::default_discovery_port")]
    pub discovery_port: u16,
}

impl ServerConfig {
    fn default_discovery_port() -> u16 {
        3301
    }
}

/// SQLite database connection settings.
#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    /// Path to the SQLite database file (created if missing).
    pub path: PathBuf,
    /// Maximum number of connections in the write pool (default: 5).
    /// SQLite allows only 1 concurrent writer, so this controls how many
    /// write connections can pipeline behind the write lock. Clamped to 2..=8.
    pub max_connections: u32,
    /// Maximum number of connections in the read-only pool (default: 32).
    /// Read connections use `PRAGMA query_only = 1` and serve all SELECT
    /// queries in request handlers.  Higher values improve read parallelism
    /// during upload/backup bursts.
    #[serde(default = "default_read_pool_max_connections")]
    pub read_pool_max_connections: u32,
}

fn default_read_pool_max_connections() -> u32 {
    32
}

#[derive(Debug, Deserialize, Clone)]
pub struct StorageConfig {
    /// Root directory for all blob storage.
    ///
    /// Supports any path accessible from the server process:
    ///
    /// **Linux / macOS:**
    ///   - Local:         "./data/storage"  or  "/var/simple-photos/storage"
    ///   - Network (SMB): "/mnt/vault/Files/Simple-Photos"
    ///     Mount first:  sudo mount -t cifs //vault.local/vault/Files/Simple-Photos \
    ///                        /mnt/simple-photos -o username=...,password=...,uid=$(id -u),gid=$(id -g)
    ///     Then set:     root = "/mnt/simple-photos"
    ///   - NFS:           "/mnt/nfs/simple-photos"
    ///
    /// **Windows:**
    ///   - Local:         ".\\data\\storage"  or  "C:\\SimplePhotos\\storage"
    ///   - Network:       "\\\\\\\\server\\share\\SimplePhotos"  or mapped drive  "Z:\\SimplePhotos"
    ///
    /// The server uses only standard cross-platform file operations (create/read/delete),
    /// so any accessible path (local, SMB/CIFS, NFS, SSHFS, mapped drive) works.
    pub root: PathBuf,

    /// Per-user storage quota in bytes (0 = unlimited).
    pub default_quota_bytes: u64,

    /// Maximum size of a single upload in bytes.
    /// Default 5 GiB to accommodate large video files.
    pub max_blob_size_bytes: u64,
}

/// Authentication and token settings.
#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    /// HMAC secret for signing JWTs. Must be at least 32 chars.
    /// Generate with: `openssl rand -hex 32`
    pub jwt_secret: String,
    /// Access token lifetime in seconds (e.g. 900 = 15 min).
    pub access_token_ttl_secs: u64,
    /// Refresh token lifetime in days (e.g. 30).
    pub refresh_token_ttl_days: u64,
    /// Whether new user registration is allowed (disable after initial setup).
    pub allow_registration: bool,
    /// bcrypt hash cost factor. Recommended: 10–12 for production, 4 for tests.
    /// Higher values are more secure but slower — each increment doubles the time.
    pub bcrypt_cost: u32,
}

/// Settings for serving the static web frontend.
#[derive(Debug, Deserialize, Clone)]
pub struct WebConfig {
    /// Path to the built web frontend directory (e.g. "../web/dist").
    /// Empty string disables static file serving.
    pub static_root: String,
}

/// Configuration for backup server features.
///
/// NOTE: There are *two* API key concepts in the backup system:
///
/// 1. `backup.api_key` (this config field / `SIMPLE_PHOTOS_BACKUP_API_KEY` env)
///    — the key that OTHER servers must provide via `X-API-Key` to access this
///    server's backup-serve endpoints (`/api/backup/serve/*`). Validated in
///    `backup::serve::validate_api_key()`. If unset, backup serving is disabled.
///
/// 2. `server_settings.backup_api_key` (DB row) — auto-generated when the admin
///    enables "backup mode" via `/api/admin/backup/mode`. Returned to the UI and
///    broadcast via LAN discovery. Only generated if `backup.api_key` is not
///    already set in config.
///
/// In typical usage, the config file value takes priority. The DB value is a
/// fallback for users who configure backup mode through the UI without editing
/// the config file.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct BackupConfig {
    /// API key that remote servers use to pull data from this instance.
    /// If empty/unset, backup serving endpoints are disabled.
    /// See also: `server_settings.backup_api_key` (DB-stored fallback).
    #[serde(default)]
    pub api_key: Option<String>,

    /// Accept invalid/self-signed TLS certificates when connecting to
    /// other backup servers. Defaults to `true` for backward compatibility
    /// with self-hosted LAN setups using self-signed certs.
    #[serde(default = "BackupConfig::default_accept_invalid_certs")]
    pub accept_invalid_certs: bool,
}

impl BackupConfig {
    fn default_accept_invalid_certs() -> bool {
        true
    }
}

/// Storage auto-scan configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct ScanConfig {
    /// How often (in seconds) to scan the storage directory for new files.
    /// Default: 300 (5 minutes). Set to 0 to disable background scanning.
    #[serde(default = "ScanConfig::default_interval")]
    pub auto_scan_interval_secs: u64,
}

impl ScanConfig {
    fn default_interval() -> u64 {
        300
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            auto_scan_interval_secs: 300,
        }
    }
}

/// TLS/SSL configuration.
/// When enabled, the server will listen on HTTPS instead of plain HTTP.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct TlsConfig {
    /// Whether TLS is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Path to the PEM-encoded TLS certificate file.
    #[serde(default)]
    pub cert_path: Option<String>,
    /// Path to the PEM-encoded TLS private key file.
    #[serde(default)]
    pub key_path: Option<String>,
}

impl AppConfig {
    /// Load configuration from TOML file, then apply env var overrides.
    ///
    /// Config file path: `$SIMPLE_PHOTOS_CONFIG` or `./config.toml`
    ///
    /// Any field can be overridden by an env var:
    ///   `SIMPLE_PHOTOS_<SECTION>_<KEY>=value`
    ///
    /// Examples:
    ///   SIMPLE_PHOTOS_AUTH_JWT_SECRET=mysecret
    ///   SIMPLE_PHOTOS_STORAGE_ROOT=/mnt/vault/Files/Simple-Photos
    ///   SIMPLE_PHOTOS_SERVER_PORT=8080
    pub fn load() -> anyhow::Result<Self> {
        let path = std::env::var("SIMPLE_PHOTOS_CONFIG").unwrap_or_else(|_| "config.toml".into());

        let contents = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path, e))?;

        let mut config: AppConfig = toml::from_str(&contents)?;

        // ── Apply environment variable overrides ─────────────────────────────
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_SERVER_HOST") {
            config.server.host = v;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_SERVER_PORT") {
            config.server.port = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_SERVER_PORT"))?;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_SERVER_BASE_URL") {
            config.server.base_url = v;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_SERVER_DISCOVERY_PORT") {
            config.server.discovery_port = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_SERVER_DISCOVERY_PORT"))?;
        }

        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_DATABASE_PATH") {
            config.database.path = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_DATABASE_MAX_CONNECTIONS") {
            config.database.max_connections = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_DATABASE_MAX_CONNECTIONS"))?;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_DATABASE_READ_POOL_MAX_CONNECTIONS") {
            config.database.read_pool_max_connections = v.parse().map_err(|_| {
                anyhow::anyhow!("Invalid SIMPLE_PHOTOS_DATABASE_READ_POOL_MAX_CONNECTIONS")
            })?;
        }

        // Storage — accepts local paths AND mounted network shares (SMB/NFS/SSHFS)
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_STORAGE_ROOT") {
            config.storage.root = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_STORAGE_DEFAULT_QUOTA_BYTES") {
            config.storage.default_quota_bytes = v.parse().map_err(|_| {
                anyhow::anyhow!("Invalid SIMPLE_PHOTOS_STORAGE_DEFAULT_QUOTA_BYTES")
            })?;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_STORAGE_MAX_BLOB_SIZE_BYTES") {
            config.storage.max_blob_size_bytes = v.parse().map_err(|_| {
                anyhow::anyhow!("Invalid SIMPLE_PHOTOS_STORAGE_MAX_BLOB_SIZE_BYTES")
            })?;
        }

        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_AUTH_JWT_SECRET") {
            config.auth.jwt_secret = v;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_AUTH_ACCESS_TOKEN_TTL_SECS") {
            config.auth.access_token_ttl_secs = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_AUTH_ACCESS_TOKEN_TTL_SECS"))?;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_AUTH_REFRESH_TOKEN_TTL_DAYS") {
            config.auth.refresh_token_ttl_days = v.parse().map_err(|_| {
                anyhow::anyhow!("Invalid SIMPLE_PHOTOS_AUTH_REFRESH_TOKEN_TTL_DAYS")
            })?;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_AUTH_ALLOW_REGISTRATION") {
            config.auth.allow_registration = v.to_lowercase() == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_AUTH_BCRYPT_COST") {
            config.auth.bcrypt_cost = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_AUTH_BCRYPT_COST"))?;
        }

        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_WEB_STATIC_ROOT") {
            config.web.static_root = v;
        }

        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_BACKUP_API_KEY") {
            config.backup.api_key = Some(v);
        }

        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_SCAN_AUTO_SCAN_INTERVAL_SECS") {
            config.scan.auto_scan_interval_secs = v.parse().map_err(|_| {
                anyhow::anyhow!("Invalid SIMPLE_PHOTOS_SCAN_AUTO_SCAN_INTERVAL_SECS")
            })?;
        }

        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_TLS_ENABLED") {
            config.tls.enabled = v.to_lowercase() == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_TLS_CERT_PATH") {
            config.tls.cert_path = Some(v);
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_TLS_KEY_PATH") {
            config.tls.key_path = Some(v);
        }

        // ── Startup validation ───────────────────────────────────────────
        // Reject obviously insecure JWT secrets early rather than waiting
        // for an auth failure at runtime.
        if config.auth.jwt_secret.len() < 32 {
            anyhow::bail!(
                "auth.jwt_secret must be at least 32 characters (got {}). \
                 Generate one with: openssl rand -hex 32",
                config.auth.jwt_secret.len()
            );
        }

        Ok(config)
    }
}
