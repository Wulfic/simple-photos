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
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub geo: GeoConfig,
    #[serde(default)]
    pub transcode: TranscodeConfig,
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

    /// Optional SMB / CIFS network share configuration.
    ///
    /// When present, the server will mount this share at `mount_point` on
    /// startup and use `mount_point/subpath` as the storage root. Configured
    /// via the first-run wizard or `PUT /api/admin/storage`. See
    /// [`crate::setup::smb`] for the lifecycle.
    #[serde(default)]
    pub smb: Option<crate::setup::smb::SmbStoredConfig>,
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
#[derive(Debug, Deserialize, Clone)]
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
    /// When TLS is enabled, also bind a plain-HTTP listener that 301-
    /// redirects every request to its HTTPS equivalent.
    /// Default: `true`. Set to `false` to disable the redirect entirely
    /// (e.g. when a reverse proxy already handles HTTP→HTTPS upgrade).
    #[serde(default = "TlsConfig::default_redirect_http")]
    pub redirect_http: bool,
    /// Port the HTTP→HTTPS redirect listener binds to.
    /// Default: `80` (the well-known HTTP port).  Bind failures are
    /// non-fatal — the server logs a warning and continues to serve
    /// HTTPS on the configured TLS port.
    #[serde(default = "TlsConfig::default_http_redirect_port")]
    pub http_redirect_port: u16,
    /// Optional Let's Encrypt account / certificate state.  Populated by
    /// [`crate::setup::letsencrypt::provision_certificate`] when the
    /// admin opts in via the setup wizard or settings panel.  Used by
    /// the daily renewal background task to know which domain to renew
    /// and surfaced in `GET /api/admin/ssl` so the UI can display
    /// renewal status.
    #[serde(default)]
    pub letsencrypt: Option<LetsEncryptConfig>,
    /// Optional self-signed local CA state.  Populated by
    /// [`crate::setup::local_ca::generate_local_ca`] when the admin picks
    /// the "Self-signed local CA" TLS option (4th choice in the wizard /
    /// settings panel).  Surfaced in `GET /api/admin/ssl` so the UI can
    /// expose the "Download CA bundle" button only after one has been
    /// generated.
    #[serde(default)]
    pub local_ca: Option<LocalCaConfig>,
}

impl TlsConfig {
    fn default_redirect_http() -> bool {
        true
    }
    fn default_http_redirect_port() -> u16 {
        80
    }
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_path: None,
            key_path: None,
            redirect_http: true,
            http_redirect_port: 80,
            letsencrypt: None,
            local_ca: None,
        }
    }
}

/// Persisted state for a self-signed local CA.
///
/// Stored under `[tls.local_ca]` in `config.toml` after a successful
/// generation run.  Contains only public/non-sensitive metadata — the
/// private keys live exclusively in `data/local_ca/*.key` with `0o600`
/// permissions.
#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct LocalCaConfig {
    /// RFC-3339 timestamp when the CA was generated.
    pub generated_at: String,
    /// RFC-3339 timestamp when the root CA expires.
    pub ca_expires_at: String,
    /// RFC-3339 timestamp when the leaf cert expires.
    pub cert_expires_at: String,
    /// SANs embedded in the leaf certificate at generation time.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// SHA-256 fingerprint of the root CA, colon-separated hex.  The
    /// install scripts re-verify this before adding the cert to the OS
    /// trust store.
    pub fingerprint_sha256: String,
}

/// Persisted state for an ACME (Let's Encrypt) certificate.
///
/// Stored under `[tls.letsencrypt]` in `config.toml` after a successful
/// provisioning run.  Used by the daily renewal task to renew within 30
/// days of expiry without operator intervention.
#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct LetsEncryptConfig {
    /// FQDN the certificate was issued for (e.g. "photos.example.com").
    pub domain: String,
    /// Contact email registered with the ACME account.
    pub email: String,
    /// `true` when the staging directory was used (test-only certificates
    /// not trusted by browsers).  Defaults to `false` (production).
    #[serde(default)]
    pub staging: bool,
    /// Port used for the ACME HTTP-01 challenge.  Default: `80`.
    #[serde(default = "LetsEncryptConfig::default_challenge_port")]
    pub challenge_port: u16,
    /// RFC-3339 timestamp of the most recent successful issuance / renewal.
    /// Used purely for diagnostics; renewal is driven by reading the cert's
    /// `notAfter` field on disk.
    #[serde(default)]
    pub last_issued_at: Option<String>,
}

impl LetsEncryptConfig {
    fn default_challenge_port() -> u16 {
        80
    }
}

/// AI face & object recognition configuration.
///
/// When `enabled = true`, the server spawns a background processor that
/// detects faces and objects in photos, clusters faces into identities,
/// and auto-applies tags. GPU acceleration is used when available.
#[derive(Debug, Deserialize, Clone)]
pub struct AiConfig {
    /// Master toggle. Also controllable at runtime via `POST /api/settings/ai`.
    #[serde(default)]
    pub enabled: bool,
    /// Prefer GPU execution provider (CUDA/TensorRT) if available.
    #[serde(default = "AiConfig::default_gpu_preferred")]
    pub gpu_preferred: bool,
    /// Number of images per inference batch.
    #[serde(default = "AiConfig::default_batch_size")]
    pub batch_size: usize,
    /// Number of threads for CPU inference. 0 = auto-detect.
    #[serde(default)]
    pub threads: usize,
    /// Maximum photos processed per minute (rate limit for background task).
    #[serde(default = "AiConfig::default_photos_per_minute")]
    pub photos_per_minute: u32,
    /// Minimum face detection confidence (0.0–1.0).
    #[serde(default = "AiConfig::default_face_confidence")]
    pub face_confidence: f32,
    /// Minimum object detection confidence (0.0–1.0).
    #[serde(default = "AiConfig::default_object_confidence")]
    pub object_confidence: f32,
    /// Cosine distance threshold for face clustering (0.0–1.0).
    /// Lower = stricter matching, higher = more permissive grouping.
    #[serde(default = "AiConfig::default_face_similarity_threshold")]
    pub face_similarity_threshold: f32,
    /// Cosine similarity threshold for pet individual clustering (0.0–1.0).
    /// MobileNetV2 logit vectors of the same pet score ~0.90–0.98;
    /// different pets of the same species score ~0.70–0.88.
    /// Default 0.85 balances recall vs. cross-pet merge rate.
    #[serde(default = "AiConfig::default_pet_similarity_threshold")]
    pub pet_similarity_threshold: f32,
    /// Directory containing ONNX model files.
    #[serde(default = "AiConfig::default_model_dir")]
    pub model_dir: String,
    /// Detection quality preset: "fast", "balanced", or "high".
    /// Higher quality is slower but more accurate.
    #[serde(default = "AiConfig::default_quality")]
    pub quality: String,
    /// Allow degraded heuristic detectors when ONNX models are unavailable.
    ///
    /// `false` (default) makes ONNX models a hard requirement: face / object
    /// detection produces no results when the model files are missing,
    /// rather than silently emitting low-quality skin-tone / colour-histogram
    /// guesses that look like real AI output.
    ///
    /// Operators who explicitly want the heuristic fallback (offline / air-
    /// gapped installs that can't download models) can opt in by setting this
    /// to `true`.
    #[serde(default)]
    pub allow_heuristic_fallback: bool,
}

impl AiConfig {
    fn default_gpu_preferred() -> bool {
        true
    }
    fn default_batch_size() -> usize {
        8
    }
    fn default_photos_per_minute() -> u32 {
        60
    }
    fn default_face_confidence() -> f32 {
        0.7
    }
    fn default_object_confidence() -> f32 {
        0.5
    }
    /// Cosine similarity threshold for ArcFace 512-d embeddings.
    ///
    /// InsightFace's `w600k_r50` model was trained so that same-identity
    /// pairs score ≥ ~0.42 and different-identity pairs ≤ ~0.30.  The old
    /// default of 0.60 was far too strict and caused the same person to
    /// fragment into many singleton clusters when their pose, lighting,
    /// or expression varied (e.g. group photos, candid shots).
    fn default_face_similarity_threshold() -> f32 {
        0.42
    }
    fn default_pet_similarity_threshold() -> f32 {
        0.70
    }
    fn default_model_dir() -> String {
        "models".into()
    }
    fn default_quality() -> String {
        "high".into()
    }

    /// Parse the quality string into a DetectionQuality enum.
    pub fn detection_quality(&self) -> crate::ai::object::DetectionQuality {
        match self.quality.to_lowercase().as_str() {
            "fast" => crate::ai::object::DetectionQuality::Fast,
            "balanced" => crate::ai::object::DetectionQuality::Balanced,
            _ => crate::ai::object::DetectionQuality::High,
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gpu_preferred: true,
            batch_size: 8,
            threads: 0,
            photos_per_minute: 60,
            face_confidence: 0.7,
            object_confidence: 0.5,
            face_similarity_threshold: 0.42,
            pet_similarity_threshold: 0.70,
            model_dir: "models".into(),
            quality: "high".into(),
            allow_heuristic_fallback: false,
        }
    }
}

/// Geolocation & timestamp smart album configuration.
///
/// When `enabled = true`, the server resolves GPS coordinates to city/country
/// using an offline reverse geocoder and generates smart albums by location
/// and timeline. Geo-scrubbing strips GPS data on upload or retroactively.
#[derive(Debug, Deserialize, Clone)]
pub struct GeoConfig {
    /// Master toggle for geolocation features.
    #[serde(default)]
    pub enabled: bool,
    /// Path to the GeoNames cities dataset file (cities500.txt).
    #[serde(default = "GeoConfig::default_dataset_path")]
    pub dataset_path: String,
    /// Maximum photos to geocode per batch.
    #[serde(default = "GeoConfig::default_batch_size")]
    pub batch_size: usize,
    /// Seconds between background backfill cycles.  The first tick fires
    /// immediately at startup; subsequent ticks fire every
    /// `poll_interval_secs`.  Production default is 5 minutes.  E2E tests
    /// override this to a few seconds so newly-uploaded photos can be
    /// asserted within a reasonable window.
    #[serde(default = "GeoConfig::default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Download the offline GeoNames dataset at runtime if it is missing when
    /// geo is enabled (self-heals a failed install — see `geo/dataset.rs`).
    /// Defaults to `true`; set `false` on a fully air-gapped server that must
    /// never make outbound requests for assets.
    #[serde(default = "GeoConfig::default_auto_download_dataset")]
    pub auto_download_dataset: bool,

    // ── Opt-in precise (street-level) reverse geocoding ──────────────────
    // Disabled by default and, crucially, gated *again* per-user: the server
    // never sends a user's coordinates to a third party unless that user has
    // explicitly turned on `geo_precise_enabled`.  These settings only shape
    // *how* enrichment happens once a user opts in.
    /// Provider for street-level lookups: `"auto"` (Nominatim, falling back to
    /// Photon on error), `"nominatim"`, or `"photon"`.  Both are keyless,
    /// no-registration OSM reverse geocoders.  (The US Census geocoder is
    /// intentionally *not* an option: its coordinate endpoint returns census
    /// geographies, not street addresses.)
    #[serde(default = "GeoConfig::default_precise_provider")]
    pub precise_provider: String,
    /// Nominatim reverse endpoint (override to point at a self-hosted mirror).
    #[serde(default = "GeoConfig::default_nominatim_endpoint")]
    pub nominatim_endpoint: String,
    /// Photon reverse endpoint.
    #[serde(default = "GeoConfig::default_photon_endpoint")]
    pub photon_endpoint: String,
    /// `User-Agent` sent with every geocoder request.  Nominatim's usage
    /// policy *requires* an identifying UA; requests without one get banned.
    #[serde(default = "GeoConfig::default_geo_user_agent")]
    pub geo_user_agent: String,
    /// Hard ceiling on outbound geocoder requests per second.  Nominatim's
    /// public instance allows at most 1/s — keep this at 1 unless pointing at
    /// your own mirror.
    #[serde(default = "GeoConfig::default_precise_rate_per_sec")]
    pub precise_rate_per_sec: u32,
    /// Hard ceiling on outbound geocoder requests per UTC day.  Stops a large
    /// first-time backfill from hammering a public instance; the remainder is
    /// picked up on subsequent days.  0 disables the daily cap.
    #[serde(default = "GeoConfig::default_precise_daily_cap")]
    pub precise_daily_cap: u32,
}

impl GeoConfig {
    // Matches the path written by `scripts/fetch_geo_data.sh` (which is what
    // `install.sh` calls) so the out-of-the-box install actually finds the
    // dataset.  Earlier default `data/geonames/cities500.txt` never matched
    // the fetch script and silently disabled reverse geocoding.
    fn default_dataset_path() -> String {
        "data/cities500.txt".into()
    }
    fn default_batch_size() -> usize {
        100
    }
    fn default_poll_interval_secs() -> u64 {
        300
    }
    fn default_auto_download_dataset() -> bool {
        true
    }
    fn default_precise_provider() -> String {
        "auto".into()
    }
    fn default_nominatim_endpoint() -> String {
        "https://nominatim.openstreetmap.org/reverse".into()
    }
    fn default_photon_endpoint() -> String {
        "https://photon.komoot.io/reverse".into()
    }
    fn default_geo_user_agent() -> String {
        concat!(
            "simple-photos/",
            env!("CARGO_PKG_VERSION"),
            " (self-hosted)"
        )
        .into()
    }
    fn default_precise_rate_per_sec() -> u32 {
        1
    }
    fn default_precise_daily_cap() -> u32 {
        10_000
    }
}

impl Default for GeoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dataset_path: Self::default_dataset_path(),
            batch_size: 100,
            poll_interval_secs: 300,
            auto_download_dataset: true,
            precise_provider: Self::default_precise_provider(),
            nominatim_endpoint: Self::default_nominatim_endpoint(),
            photon_endpoint: Self::default_photon_endpoint(),
            geo_user_agent: Self::default_geo_user_agent(),
            precise_rate_per_sec: Self::default_precise_rate_per_sec(),
            precise_daily_cap: Self::default_precise_daily_cap(),
        }
    }
}

/// GPU-accelerated video transcoding configuration.
///
/// When `gpu_enabled = true` (the default), the server probes FFmpeg for
/// hardware acceleration at startup and uses GPU encoding for video
/// conversions when available.  Falls back to CPU seamlessly.
#[derive(Debug, Deserialize, Clone)]
pub struct TranscodeConfig {
    /// Allow GPU acceleration for video transcoding.
    #[serde(default = "TranscodeConfig::default_gpu_enabled")]
    pub gpu_enabled: bool,
    /// Retry with CPU if GPU transcode fails.
    #[serde(default = "TranscodeConfig::default_gpu_fallback_to_cpu")]
    pub gpu_fallback_to_cpu: bool,
    /// Specific GPU device path (empty = auto-detect).
    #[serde(default)]
    pub gpu_device: String,
}

impl TranscodeConfig {
    fn default_gpu_enabled() -> bool {
        true
    }
    fn default_gpu_fallback_to_cpu() -> bool {
        true
    }
}

impl Default for TranscodeConfig {
    fn default() -> Self {
        Self {
            gpu_enabled: true,
            gpu_fallback_to_cpu: true,
            gpu_device: String::new(),
        }
    }
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
            .map_err(|e| anyhow::anyhow!("Failed to read config file '{path}': {e}"))?;

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
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_TLS_REDIRECT_HTTP") {
            config.tls.redirect_http = v.to_lowercase() == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_TLS_HTTP_REDIRECT_PORT") {
            config.tls.http_redirect_port = v
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid SIMPLE_PHOTOS_TLS_HTTP_REDIRECT_PORT"))?;
        }

        // AI
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_AI_ENABLED") {
            config.ai.enabled = v.to_lowercase() == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("SIMPLE_PHOTOS_AI_MODEL_DIR") {
            config.ai.model_dir = v;
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
