# Simple Photos — Implementation Plan

## Context

Building a self-hosted, end-to-end encrypted photos **and video** application from scratch. The server is written in **Rust (Axum)**, the mobile client is an **Android app (Kotlin + Jetpack Compose)**, and there is a **web frontend (React + TypeScript + Vite)**. All content and metadata is encrypted **client-side** before upload — the server stores only opaque encrypted blobs and cannot read any user data without the client-side key.

**Key decisions:**
- Server: Rust + Axum + SQLite (via sqlx)
- Multi-user (isolated storage per user)
- Full E2E encryption: photos, videos, GIFs, filenames, timestamps, album names — all encrypted before upload
- **All media formats supported**: every image format (JPEG, PNG, WebP, HEIC, AVIF, TIFF, BMP, RAW…), animated GIFs, and all video formats (MP4, MOV, MKV, WebM, AVI, etc.)
- Android UI: Jetpack Compose
- Web UI: React + TypeScript + Vite (served by Axum)
- 2FA: TOTP (RFC 6238) — compatible with Authy, Google Authenticator, Microsoft Authenticator, 1Password
- **Network drive support**: storage root can be any POSIX-mounted path — local disk, SMB/CIFS, NFS, SSHFS, etc. Production target: `smb://vault.local/vault/Files/Simple-Photos`
- **Environment variables**: all sensitive config overridable via `SIMPLE_PHOTOS_*` env vars; `.env` files gitignored for public release

---

## Network Drive / SMB Setup

The server writes only standard POSIX file I/O, so **any mounted filesystem** works as the storage root.

### Mount the Samba share (once, on the host or in Docker)

```bash
# Install cifs-utils
sudo apt install cifs-utils

# Create mount point
sudo mkdir -p /mnt/simple-photos

# Mount (add to /etc/fstab for persistence)
sudo mount -t cifs //vault.local/vault/Files/Simple-Photos /mnt/simple-photos \
  -o username=YOUR_USER,password=YOUR_PASS,uid=$(id -u),gid=$(id -g),vers=3.0,iocharset=utf8
```

### /etc/fstab entry (persistent)

```
//vault.local/vault/Files/Simple-Photos  /mnt/simple-photos  cifs \
  credentials=/etc/samba/simple-photos.creds,uid=1000,gid=1000,vers=3.0,iocharset=utf8  0 0
```

### Server config

```toml
[storage]
root = "/mnt/simple-photos"
```

Or via environment variable (no config file change needed):

```bash
SIMPLE_PHOTOS_STORAGE_ROOT=/mnt/simple-photos
```

---

## Environment Variables & Secrets

All secrets live in `.env` files which are **gitignored** and never committed.

| File | Purpose |
|---|---|
| `server/.env` | Server runtime secrets (JWT secret, storage path, etc.) |
| `server/.env.example` | Template committed to git — copy to `.env` and fill in |
| `web/.env` | Vite build-time env vars (`VITE_API_BASE_URL`, etc.) |
| `web/.env.example` | Template committed to git |
| `.gitignore` | Root gitignore covers `.env`, `target/`, `node_modules/`, build output, IDE files, OS files, and network drive artifacts |

### Server env var format

```
SIMPLE_PHOTOS_<SECTION>_<KEY>=value
# e.g.
SIMPLE_PHOTOS_AUTH_JWT_SECRET=...
SIMPLE_PHOTOS_STORAGE_ROOT=/mnt/simple-photos
SIMPLE_PHOTOS_SERVER_PORT=8080
```

---

## Directory Structure

```
simple-photos/
├── .gitignore                   # Root gitignore (all platforms + secrets)
├── plan.md
├── web/
│   ├── .env                     # ← gitignored, copied from .env.example
│   ├── .env.example             # ← committed template
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   └── src/
│       ├── main.tsx
│       ├── App.tsx
│       ├── api/                 # Fetch wrappers for REST API
│       ├── crypto/              # Web Crypto API + Argon2id (WASM)
│       ├── db/                  # IndexedDB (Dexie.js) — v2 schema with mediaType
│       ├── store/               # Zustand auth state
│       └── pages/
│           ├── Setup.tsx        # Server URL + passphrase setup
│           ├── Login.tsx        # Login + TOTP code prompt
│           ├── Gallery.tsx      # Media grid (photos, GIFs, videos)
│           ├── Albums.tsx       # Album list + detail
│           ├── Viewer.tsx       # Full-res viewer + video player + live preview
│           └── Settings.tsx     # Account settings, 2FA setup
│
├── server/
│   ├── .env                     # ← gitignored, copied from .env.example
│   ├── .env.example             # ← committed template
│   ├── Cargo.toml
│   ├── Dockerfile
│   ├── docker-compose.yml
│   ├── .dockerignore
│   ├── config.example.toml
│   ├── config.toml              # ← gitignored local dev config
│   ├── migrations/
│   │   └── 001_initial.sql
│   └── src/
│       ├── main.rs            # Entry: load config, init DB, start Axum
│       ├── config.rs          # Config struct + TOML + env var overrides
│       ├── db.rs              # SQLite pool init + sqlx::migrate!
│       ├── error.rs           # AppError enum + IntoResponse impl
│       ├── state.rs           # AppState: pool, config
│       ├── auth/
│       │   ├── handlers.rs    # register, login, refresh, logout, 2FA endpoints
│       │   ├── middleware.rs  # JWT extractor, require_auth
│       │   └── models.rs      # User, LoginRequest, RegisterRequest
│       ├── blobs/
│       │   ├── handlers.rs    # upload, streaming download, list, delete
│       │   ├── storage.rs     # Local filesystem (works on any POSIX mount)
│       │   └── models.rs      # BlobRecord, BlobListResponse
│       └── health/
│           └── handlers.rs    # GET /health
│
└── android/
    ├── settings.gradle.kts
    ├── build.gradle.kts
    ├── gradle/libs.versions.toml
    └── app/
        ├── build.gradle.kts
        └── src/main/
            ├── AndroidManifest.xml
            └── kotlin/com/simplephotos/
                ├── SimplePhotosApplication.kt
                ├── MainActivity.kt
                ├── di/
                │   ├── AppModule.kt
                │   └── NetworkModule.kt
                ├── data/
                │   ├── local/
                │   │   ├── AppDatabase.kt
                │   │   ├── dao/ (PhotoDao, AlbumDao, BlobQueueDao)
                │   │   └── entities/ (PhotoEntity, AlbumEntity, BlobQueueEntity)
                │   ├── remote/
                │   │   ├── ApiService.kt   # Retrofit interface
                │   │   └── dto/ (AuthDto, BlobDto)
                │   └── repository/ (AuthRepository, PhotoRepository, AlbumRepository, SyncRepository)
                ├── crypto/
                │   ├── CryptoManager.kt   # AES-256-GCM encrypt/decrypt
                │   ├── KeyManager.kt      # Argon2id derivation + Keystore wrapping
                │   └── EncryptedPayload.kt
                ├── sync/
                │   ├── BackupWorker.kt    # WorkManager worker
                │   └── SyncScheduler.kt
                └── ui/
                    ├── theme/
                    ├── navigation/ (NavGraph, Screen)
                    └── screens/
                        ├── setup/ (ServerSetupScreen, PassphraseSetupScreen)
                        ├── auth/ (LoginScreen, RegisterScreen)
                        ├── gallery/ (GalleryScreen, GalleryViewModel)
                        ├── album/ (AlbumListScreen, AlbumDetailScreen, AlbumViewModel)
                        ├── viewer/ (PhotoViewerScreen, PhotoViewerViewModel)
                        ├── settings/ (SettingsScreen, SettingsViewModel)
                        └── twofactor/ (TwoFactorSetupScreen)
```

---

## Server API

All endpoints except `/health`, `/api/auth/register`, `/api/auth/login` require `Authorization: Bearer <jwt>`.

### Auth
```
POST /api/auth/register          { username, password } → { user_id, username }
POST /api/auth/login             { username, password }
                                   → { access_token, refresh_token, expires_in }   (if no 2FA)
                                   → { requires_totp: true, totp_session_token }   (if 2FA enabled)
POST /api/auth/login/totp        { totp_session_token, totp_code }
                                   OR { totp_session_token, backup_code }
                                   → { access_token, refresh_token, expires_in }
POST /api/auth/refresh           { refresh_token } → { access_token, expires_in }
POST /api/auth/logout            { refresh_token } → 204
```

### 2FA (TOTP)
```
POST /api/auth/2fa/setup         → { otpauth_uri: "otpauth://totp/...", backup_codes: [...] }
POST /api/auth/2fa/confirm       { totp_code: "123456" } → 200
POST /api/auth/2fa/disable       { totp_code: "123456" } → 204
```

### Blobs (all content is opaque encrypted bytes)
```
POST   /api/blobs                Upload blob (raw bytes body)
                                   Headers: X-Blob-Type, X-Blob-Size, X-Client-Hash
                                   X-Blob-Type values: photo | gif | video | thumbnail | video_thumbnail | album_manifest
                                   → 201 { blob_id, upload_time, size }
GET    /api/blobs                List blobs (query: blob_type, after, limit)
                                   → { blobs: [{blob_id, blob_type, size, upload_time, client_hash}], next_cursor }
GET    /api/blobs/:id            Stream encrypted blob → raw bytes (chunked, Accept-Ranges supported)
DELETE /api/blobs/:id            → 204
```

### Other
```
GET  /health                     → { status: "ok", version: "0.1.0" }
GET  /*                          Serve React SPA static files (API routes take precedence)
```

---

## Server Database Schema (`server/migrations/001_initial.sql`)

```sql
CREATE TABLE users (
    id                    TEXT PRIMARY KEY,
    username              TEXT NOT NULL UNIQUE,
    password_hash         TEXT NOT NULL,
    created_at            TEXT NOT NULL,
    storage_quota_bytes   INTEGER NOT NULL DEFAULT 10737418240,
    totp_secret           TEXT,
    totp_enabled          INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE totp_backup_codes (
    id        TEXT PRIMARY KEY,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_totp_backup_user ON totp_backup_codes(user_id);

CREATE TABLE refresh_tokens (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL UNIQUE,
    expires_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_refresh_tokens_user ON refresh_tokens(user_id);

CREATE TABLE blobs (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blob_type    TEXT NOT NULL,
    -- photo | gif | video | thumbnail | video_thumbnail | album_manifest
    size_bytes   INTEGER NOT NULL,
    client_hash  TEXT,
    upload_time  TEXT NOT NULL,
    storage_path TEXT NOT NULL
);
CREATE INDEX idx_blobs_user_type_time ON blobs(user_id, blob_type, upload_time);
CREATE INDEX idx_blobs_client_hash ON blobs(user_id, client_hash) WHERE client_hash IS NOT NULL;
```

---

## Encrypted Blob Format

Every uploaded blob: `[12-byte nonce][AES-256-GCM ciphertext + 16-byte auth tag]`

The plaintext of each blob is a versioned JSON envelope:

**Photo / GIF blob** (`blob_type = "photo"` or `"gif"`):
```json
{
  "v": 1,
  "filename": "IMG_20260222.jpg",
  "taken_at": "2026-02-22T15:38:00Z",
  "mime_type": "image/jpeg",
  "media_type": "photo",
  "width": 4000,
  "height": 3000,
  "latitude": 48.8566,
  "longitude": 2.3522,
  "album_ids": ["local-uuid-1"],
  "thumbnail_blob_id": "server-uuid",
  "data": "<base64-encoded file bytes>"
}
```

**Video blob** (`blob_type = "video"`):
```json
{
  "v": 1,
  "filename": "VID_20260222.mp4",
  "taken_at": "2026-02-22T15:38:00Z",
  "mime_type": "video/mp4",
  "media_type": "video",
  "width": 1920,
  "height": 1080,
  "duration": 127.4,
  "album_ids": ["local-uuid-1"],
  "thumbnail_blob_id": "server-uuid",
  "data": "<base64-encoded video bytes>"
}
```

**Thumbnail blob** (`blob_type = "thumbnail"` or `"video_thumbnail"`):
```json
{ "v": 1, "photo_blob_id": "server-uuid", "width": 256, "height": 256, "data": "<base64 JPEG>" }
```

**Album manifest blob** (`blob_type = "album_manifest"`):
```json
{
  "v": 1,
  "album_id": "local-uuid",
  "name": "Vacation 2026",
  "created_at": "2026-02-22T00:00:00Z",
  "cover_photo_blob_id": "server-uuid",
  "photo_blob_ids": ["uuid1", "uuid2", "uuid3"]
}
```

---

## Android Room Schema

```kotlin
@Entity("photos")
data class PhotoEntity(
    @PrimaryKey val localId: String,
    val serverBlobId: String?,
    val thumbnailBlobId: String?,
    val filename: String,
    val takenAt: Long,
    val mimeType: String,
    /** "photo" | "gif" | "video" */
    val mediaType: String,
    val width: Int, val height: Int,
    /** Duration in seconds for videos, null for photos/GIFs */
    val durationSecs: Float?,
    val latitude: Double?, val longitude: Double?,
    val localPath: String?,
    val syncStatus: SyncStatus,
    val encryptedBlobSize: Long?,
    val createdAt: Long
)

@Entity("albums")
data class AlbumEntity(
    @PrimaryKey val localId: String,
    val serverManifestBlobId: String?,
    val name: String,
    val coverPhotoLocalId: String?,
    val syncStatus: SyncStatus,
    val createdAt: Long
)

@Entity("photo_album_xref", primaryKeys = ["photoLocalId", "albumLocalId"])
data class PhotoAlbumXRef(val photoLocalId: String, val albumLocalId: String)

@Entity("blob_queue")
data class BlobQueueEntity(
    @PrimaryKey val id: String,
    val photoLocalId: String?,
    val albumLocalId: String?,
    val blobType: String,
    // "photo" | "gif" | "video" | "thumbnail" | "video_thumbnail" | "album_manifest"
    val priority: Int,
    val attempts: Int,
    val lastAttemptAt: Long?,
    val status: String
)
```

---

## Media Format Support

### Supported formats (all stored as opaque encrypted blobs)

| Category | Formats |
|---|---|
| **Images** | JPEG, PNG, WebP, HEIC/HEIF, AVIF, TIFF, BMP, GIF (static), RAW (DNG, CR2, NEF, ARW…) |
| **Animated GIF** | GIF (animated) — stored with `blob_type = "gif"`, played back as `<img>` |
| **Video** | MP4 (H.264/H.265), MOV, MKV, WebM (VP8/VP9/AV1), AVI, 3GP, M4V, TS |

### Thumbnail generation
- **Images**: Canvas 2D resize → 256×256 JPEG cover-crop
- **GIFs**: First frame rendered via `<img>` → Canvas → JPEG
- **Videos**: `<video>` element seeked to 10 % of duration → Canvas frame capture → JPEG

### Video upload path (web)
1. User selects `.mp4`, `.mov`, `.mkv`, etc. via file picker or drag-drop
2. Client generates poster-frame thumbnail (seeks to 10 % of duration)
3. Encrypts thumbnail → uploads as `blob_type = video_thumbnail`
4. Encrypts full video bytes → uploads as `blob_type = video`
5. Video blob payload includes `duration` field (seconds)

### Video playback (web Viewer)
- Full video bytes decrypted client-side → `Blob` → `Object URL` → `<video controls>`
- **Live preview**: cached thumbnail shown (blurred) immediately while decryption runs
- Browser-native controls for play/pause/seek/volume/fullscreen
- `Accept-Ranges: bytes` header returned by server enables HTTP range requests

---

## Encryption Flow

### Key Setup (one-time, Android)
1. User enters passphrase on `PassphraseSetupScreen`
2. App generates random 16-byte salt (stored in `EncryptedSharedPreferences`)
3. **Argon2id** (via BouncyCastle `Argon2BytesGenerator`) derives 32-byte DEK:
   `memory=65536 KB, time=3, parallelism=4`
4. DEK is wrapped by an **Android Keystore** AES-GCM key and stored encrypted
5. Passphrase is discarded from memory

### Key Setup (browser)
1. User enters passphrase on `/setup` page
2. WASM Argon2id (`hash-wasm`) derives 32-byte key using same parameters + client-side salt
3. Key imported as `CryptoKey` via `crypto.subtle.importKey` for AES-GCM
4. Stored in `sessionStorage` (cleared on tab close)

### Upload Path (per media item)
1. Read file bytes (image/GIF/video)
2. Generate 256×256 JPEG thumbnail (video: seek to 10 % of duration; image: canvas resize)
3. Encrypt thumbnail → upload as `thumbnail` or `video_thumbnail`
4. Build JSON payload with `media_type`, `mime_type`, `duration` (video), base64 data
5. Encrypt payload → upload as `photo`, `gif`, or `video` blob type
6. Update local IndexedDB cache (web) / Room DB (Android)

---

## Backup Sync (BackupWorker — Android)

- `PeriodicWorkRequest` every 1 hour
- Constraints: `NetworkType.UNMETERED` (WiFi-only, user-configurable), `BATTERY_NOT_LOW`
- Backoff: exponential starting at 15 min

**Worker logic:**
1. Query MediaStore for media (photos **and videos**) newer than `last_sync_timestamp`
2. Insert new items as `PENDING` in Room; enqueue `BlobQueueEntity` rows
3. Process queue in priority order (thumbnail=0 → media=1 → album_manifest=2)
4. On success: update Room, mark queue entry `done`
5. On failure: increment `attempts`; mark `failed` after 5 attempts

---

## 2FA Flow

### Enabling 2FA
1. Settings → "Enable Two-Factor Authentication"
2. `POST /api/auth/2fa/setup` → `{ otpauth_uri, backup_codes }`
3. Server generates TOTP secret, stores temporarily (`totp_enabled=0`), returns URI + 10 backup codes
4. Client shows QR code + backup codes
5. User enters 6-digit code → `POST /api/auth/2fa/confirm`
6. Server verifies → `totp_enabled=1`

### Login with 2FA
1. `POST /api/auth/login` → `{ requires_totp: true, totp_session_token }`
2. `POST /api/auth/login/totp` → full auth tokens

---

## Server Config (`server/config.example.toml`)

```toml
[server]
host = "0.0.0.0"
port = 3000
base_url = "http://localhost:3000"

[database]
path = "/data/db/simple-photos.db"
max_connections = 5

[storage]
# Local or network-mounted path.
# SMB: mount //vault.local/vault/Files/Simple-Photos → set root = "/mnt/simple-photos"
root = "/data/storage"
default_quota_bytes = 10737418240   # 10 GB per user
max_blob_size_bytes  = 5368709120   # 5 GB per upload (supports large videos)

[auth]
jwt_secret = "CHANGE_ME_RANDOM_64_CHAR_HEX"   # openssl rand -hex 32
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 12

[web]
static_root = "./web/dist"
```

All values overridable via `SIMPLE_PHOTOS_<SECTION>_<KEY>` env vars.

---

## Blob Storage Layout on Disk

```
{storage.root}/
  <user_id[0..2]>/
    <user_id>/
      <blob_id[0..2]>/
        <blob_id>.bin
```

Two-level sharding avoids filesystem issues with millions of files. `storage_path` in DB is relative to `storage.root` — root can be moved or remounted without a DB migration.

Works transparently on:
- Local disk (`ext4`, `btrfs`, `xfs`, `zfs`, etc.)
- SMB/CIFS shares (e.g. `smb://vault.local/vault/Files/Simple-Photos`)
- NFS mounts
- SSHFS mounts

---

## Docker

### `server/Dockerfile`
```dockerfile
FROM rust:1.77-slim-bookworm AS builder
WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm src/main.rs
COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libsqlite3-0 cifs-utils curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/simple-photos-server ./
COPY --from=builder /usr/src/app/migrations ./migrations
RUN mkdir -p /data/storage /data/db
ENV SIMPLE_PHOTOS_CONFIG=/app/config.toml
EXPOSE 3000
ENTRYPOINT ["./simple-photos-server"]
```

To use a network drive in Docker, mount it on the host and bind-mount into the container:

```yaml
# docker-compose.yml
services:
  server:
    volumes:
      - /mnt/simple-photos:/data/storage   # host SMB mount → container
```

### `server/docker-compose.yml`
```yaml
services:
  server:
    build: .
    container_name: simple-photos
    restart: unless-stopped
    ports:
      - "3000:3000"
    volumes:
      - ./config.toml:/app/config.toml:ro
      - ./web/dist:/app/web/dist:ro
      - photos_db:/data/db
      - photos_storage:/data/storage   # replace with bind-mount for network drive
    environment:
      - RUST_LOG=info
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3000/health"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s

volumes:
  photos_db:
  photos_storage:
```

---

## Key Rust Crates (`server/Cargo.toml`)

| Crate | Purpose |
|---|---|
| `axum 0.8` | Web framework |
| `sqlx 0.8` (sqlite, migrate) | Async DB + embedded migrations |
| `tokio 1` (full) | Async runtime |
| `tokio-util 0.7` (io) | `ReaderStream` for streaming file downloads |
| `tower-http 0.6` | CORS, tracing, request body size limit, static files |
| `jsonwebtoken 9` | JWT signing/verification (HS256) |
| `bcrypt 0.15` | Password hashing |
| `totp-rs 5` | TOTP (RFC 6238) |
| `uuid 1` (v4) | Blob and user IDs |
| `sha2 0.10` | SHA-256 |
| `serde / serde_json` | Serialization |
| `toml 0.8` | Config file parsing |
| `chrono 0.4` | Timestamps |
| `thiserror / anyhow` | Error handling |
| `tracing / tracing-subscriber` | Structured logging |
| `rand 0.8` | Cryptographic randomness |
| `hex 0.4` | Hex encoding |
| `base32 0.5` | TOTP secret encoding |

---

## Key Web Libraries

| Library | Purpose |
|---|---|
| React 18 + TypeScript | UI framework |
| Vite | Build tool |
| React Router v6 | SPA routing |
| Zustand | Auth global state |
| Dexie.js v4 | IndexedDB (v2 schema: mediaType, duration) |
| Web Crypto API | AES-256-GCM encrypt/decrypt |
| `hash-wasm` | Argon2id WASM |
| `qrcode.react` | QR code for TOTP setup |
| Tailwind CSS | Styling |

---

## Key Android Libraries

| Library | Purpose |
|---|---|
| Jetpack Compose BOM 2024.05 | Declarative UI |
| Hilt 2.51 | Dependency injection |
| Room 2.6 | Local SQLite ORM |
| WorkManager 2.9 | Background backup (photos + videos) |
| Retrofit 2.11 + OkHttp 4.12 | REST client |
| Coil 2.6 | Image/video thumbnail loading; custom `Fetcher` decrypts blobs |
| ExoPlayer | Video playback (decrypted in memory) |
| BouncyCastle `bcpkix-jdk15to18` 1.78 | Argon2id key derivation |
| `androidx.security:security-crypto` | `EncryptedSharedPreferences` |
| DataStore Preferences | Server URL, WiFi-only toggle, last_sync_timestamp |
| Navigation Compose 2.7 | Navigation graph |
| ExifInterface 1.3 | EXIF metadata extraction |
| Accompanist Permissions 0.34 | `READ_MEDIA_IMAGES` + `READ_MEDIA_VIDEO` permission flow |

---

## Android First-Run Navigation

`NavGraph` checks DataStore flags on launch:
1. `server_configured = false` → `ServerSetupScreen`
2. `passphrase_configured = false` → `PassphraseSetupScreen`
3. No valid access token → `LoginScreen`
4. Otherwise → `GalleryScreen`

---

## UI Screens

### Android
| Screen | Features |
|---|---|
| `GalleryScreen` | Grid of photos, GIFs, and video thumbnails (with duration badge) sorted by `takenAt` |
| `AlbumListScreen` | Album grid with cover thumbnails |
| `AlbumDetailScreen` | Media in album; add/remove |
| `PhotoViewerScreen` | Full-res image viewer / video player; swipe navigation; share; delete |
| `SettingsScreen` | Server URL, WiFi-only, backup status, 2FA management, logout |
| `ServerSetupScreen` | Enter server URL, test connection |
| `PassphraseSetupScreen` | Enter + confirm passphrase; Argon2id → wrapped DEK |
| `LoginScreen` | Username + password; inline TOTP field if 2FA enabled |
| `RegisterScreen` | New account creation |
| `TwoFactorSetupScreen` | QR code display, confirm code, show backup codes |

### Web
| Page | Features |
|---|---|
| `/gallery` | Media grid — photos, GIFs (badge), videos (badge + duration) — lazy-loaded thumbnails; drag-and-drop upload |
| `/albums` | Album grid + detail |
| `/photo/:id` | **Viewer**: live preview (blurred thumbnail while decrypting), photo `<img>`, GIF `<img>`, video `<video controls>` player |
| `/login` | Username + password + TOTP |
| `/register` | New account |
| `/setup` | Passphrase setup (Argon2id → sessionStorage) |
| `/settings` | 2FA enable/disable, logout |

---

## Verification / Testing

### Server
```bash
cd server

# Generate a JWT secret for local dev
openssl rand -hex 32

# Copy env template and fill in
cp .env.example .env

cargo build
cargo test
./target/debug/simple-photos-server

# Smoke tests
curl http://localhost:3000/health
curl -X POST http://localhost:3000/api/auth/register \
  -H 'Content-Type: application/json' \
  -d '{"username":"test","password":"password123"}'

# Upload a video blob
TOKEN=$(curl -s -X POST http://localhost:3000/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"test","password":"password123"}' | jq -r .access_token)
curl -X POST http://localhost:3000/api/blobs \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Blob-Type: video" \
  --data-binary @/path/to/test.mp4

# Docker
docker-compose up --build
```

### Web Frontend
```bash
cd web
cp .env.example .env
npm install
npm run dev      # Vite dev server, proxy /api → localhost:3000

# Test flows:
# 1. Register + login
# 2. Passphrase setup
# 3. Upload JPEG, PNG, GIF, MP4, MOV via drag-drop or file picker
# 4. Verify thumbnail badges (GIF badge, video badge + duration)
# 5. Click video → Viewer shows blurred thumbnail, then video player
# 6. Click GIF → animated GIF plays in <img>
# 7. Click photo → full-res image
# 8. Enable 2FA, log out, log back in with TOTP code
# 9. Verify network drive path works by changing SIMPLE_PHOTOS_STORAGE_ROOT

npm run build    # produces web/dist/
```

### Android
- Run on emulator or device
- Grant `READ_MEDIA_IMAGES` + `READ_MEDIA_VIDEO` permissions
- First-run: server URL → passphrase
- Register account
- Verify backup worker uploads photos and videos
- Verify video thumbnail generation and ExoPlayer playback
- Enable 2FA, log out, log back in with TOTP
