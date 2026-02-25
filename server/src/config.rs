use serde::Deserialize;
use std::path::PathBuf;

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
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub base_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub path: PathBuf,
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

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub access_token_ttl_secs: u64,
    pub refresh_token_ttl_days: u64,
    pub allow_registration: bool,
    pub bcrypt_cost: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WebConfig {
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
