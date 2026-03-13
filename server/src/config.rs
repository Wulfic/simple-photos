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
}

/// SQLite database connection settings.
#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    /// Path to the SQLite database file (created if missing).
    pub path: PathBuf,
    /// Maximum number of connections in the pool (default: 5).
    pub max_connections: u32,
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
/// `api_key` is the key that OTHER servers must provide (via X-API-Key header)
/// to access this server's backup list/download endpoints.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct BackupConfig {
    /// API key that remote servers use to pull data from this instance.
    /// If empty/unset, backup serving endpoints are disabled.
    #[serde(default)]
    pub api_key: Option<String>,
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
        let path = std::env::var("SIMPLE_PHOTOS_CONFIG")
            .unwrap_or_else(|_| "config.toml".into());

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

        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_DATABASE_PATH") {
            config.database.path = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_DATABASE_MAX_CONNECTIONS") {
            config.database.max_connections = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_DATABASE_MAX_CONNECTIONS"))?;
        }

        // Storage — accepts local paths AND mounted network shares (SMB/NFS/SSHFS)
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_STORAGE_ROOT") {
            config.storage.root = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_STORAGE_DEFAULT_QUOTA_BYTES") {
            config.storage.default_quota_bytes = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_STORAGE_DEFAULT_QUOTA_BYTES"))?;
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_STORAGE_MAX_BLOB_SIZE_BYTES") {
            config.storage.max_blob_size_bytes = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_STORAGE_MAX_BLOB_SIZE_BYTES"))?;
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
            config.auth.refresh_token_ttl_days = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_AUTH_REFRESH_TOKEN_TTL_DAYS"))?;
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
            config.scan.auto_scan_interval_secs = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_SCAN_AUTO_SCAN_INTERVAL_SECS"))?;
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

        Ok(config)
    }
}
