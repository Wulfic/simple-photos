# Simple Photos — Implementation Plan

## Context

Building a self-hosted, end-to-end encrypted photos application from scratch. The server is written in **Rust (Axum)**, the mobile client is an **Android app (Kotlin + Jetpack Compose)**, and there is a **web frontend (React + TypeScript + Vite)**. All photo content and metadata is encrypted **client-side** before upload — the server stores only opaque encrypted blobs and cannot read any user data without the client-side key.

**Key decisions:**
- Server: Rust + Axum + SQLite (via sqlx)
- Multi-user (isolated storage per user)
- Full E2E encryption: photos, filenames, timestamps, album names — all encrypted before upload
- Android UI: Jetpack Compose
- Web UI: React + TypeScript + Vite (served by Axum)
- 2FA: TOTP (RFC 6238) — compatible with Authy, Google Authenticator, Microsoft Authenticator, 1Password

---

## Directory Structure

```
simple-photos/
├── plan.md
├── web/                         # React + TypeScript + Vite frontend
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   └── src/
│       ├── main.tsx
│       ├── App.tsx
│       ├── api/                 # Fetch wrappers for REST API
│       ├── crypto/              # Web Crypto API + Argon2id (WASM)
│       ├── db/                  # IndexedDB local cache (Dexie.js)
│       ├── store/               # Zustand state management
│       └── pages/
│           ├── Setup.tsx        # Server URL + passphrase setup
│           ├── Login.tsx        # Login + TOTP code prompt
│           ├── Gallery.tsx      # Photo grid
│           ├── Albums.tsx       # Album list + detail
│           ├── Viewer.tsx       # Full-res photo viewer
│           └── Settings.tsx     # Account settings, 2FA setup
│
├── server/
│   ├── Cargo.toml
│   ├── Dockerfile
│   ├── docker-compose.yml
│   ├── .dockerignore
│   ├── config.example.toml
│   ├── migrations/
│   │   └── 001_initial.sql
│   └── src/
│       ├── main.rs            # Entry: load config, init DB, start Axum
│       ├── config.rs          # Config struct + TOML deserialization
│       ├── db.rs              # SQLite pool init + sqlx::migrate!
│       ├── error.rs           # AppError enum + IntoResponse impl
│       ├── state.rs           # AppState: pool, config, storage
│       ├── auth/
│       │   ├── handlers.rs    # register, login, refresh, logout, 2FA endpoints
│       │   ├── middleware.rs  # JWT extractor, require_auth
│       │   └── models.rs      # User, LoginRequest, RegisterRequest
│       ├── blobs/
│       │   ├── handlers.rs    # upload, download, list, delete
│       │   ├── storage.rs     # Local filesystem trait impl
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

`totp_session_token` is a short-lived (5 min) JWT encoding only `user_id + totp_required=true` — cannot access any other authenticated endpoints.

### 2FA (TOTP)
```
POST /api/auth/2fa/setup         → { otpauth_uri: "otpauth://totp/...", backup_codes: [...] }
POST /api/auth/2fa/confirm       { totp_code: "123456" } → 200  (finalizes 2FA enablement)
POST /api/auth/2fa/disable       { totp_code: "123456" } → 204
```

### Blobs (all content is opaque encrypted bytes)
```
POST   /api/blobs                Upload blob (raw bytes body)
                                   Headers: X-Blob-Type, X-Blob-Size, X-Client-Hash
                                   → 201 { blob_id, upload_time, size }
GET    /api/blobs                List blobs (query: blob_type, after, limit)
                                   → { blobs: [{blob_id, blob_type, size, upload_time, client_hash}], next_cursor }
GET    /api/blobs/:id            Download encrypted blob → raw bytes
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
    id                    TEXT PRIMARY KEY,  -- UUID v4
    username              TEXT NOT NULL UNIQUE,
    password_hash         TEXT NOT NULL,     -- bcrypt
    created_at            TEXT NOT NULL,
    storage_quota_bytes   INTEGER NOT NULL DEFAULT 10737418240,
    totp_secret           TEXT,              -- base32-encoded TOTP secret, NULL if 2FA disabled
    totp_enabled          INTEGER NOT NULL DEFAULT 0
);

-- Single-use TOTP backup codes (hashed)
CREATE TABLE totp_backup_codes (
    id        TEXT PRIMARY KEY,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,   -- SHA-256 of raw backup code
    used      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_totp_backup_user ON totp_backup_codes(user_id);

-- Refresh tokens (stored as SHA-256 hash of the raw token)
CREATE TABLE refresh_tokens (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL UNIQUE,
    expires_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_refresh_tokens_user ON refresh_tokens(user_id);

-- Encrypted blobs — server knows nothing about contents
CREATE TABLE blobs (
    id           TEXT PRIMARY KEY,  -- UUID v4 returned as blob_id
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blob_type    TEXT NOT NULL,     -- 'photo' | 'thumbnail' | 'album_manifest'
    size_bytes   INTEGER NOT NULL,
    client_hash  TEXT,              -- opaque SHA-256 hex from client (dedup)
    upload_time  TEXT NOT NULL,
    storage_path TEXT NOT NULL      -- relative path from storage_root
);
CREATE INDEX idx_blobs_user_type_time ON blobs(user_id, blob_type, upload_time);
CREATE INDEX idx_blobs_client_hash ON blobs(user_id, client_hash) WHERE client_hash IS NOT NULL;
```

No photo/album semantic tables on server — all structure lives encrypted on the client.

---

## Encrypted Blob Format

Every uploaded blob: `[12-byte nonce][AES-256-GCM ciphertext + 16-byte auth tag]`

The plaintext of each blob is a versioned JSON envelope:

**Photo blob:**
```json
{
  "v": 1,
  "filename": "IMG_20260222.jpg",
  "taken_at": "2026-02-22T15:38:00Z",
  "mime_type": "image/jpeg",
  "width": 4000,
  "height": 3000,
  "latitude": 48.8566,
  "longitude": 2.3522,
  "album_ids": ["local-uuid-1"],
  "thumbnail_blob_id": "server-uuid",
  "data": "<base64-encoded JPEG bytes>"
}
```

**Thumbnail blob:**
```json
{ "v": 1, "photo_blob_id": "server-uuid", "width": 256, "height": 256, "data": "<base64 JPEG>" }
```

**Album manifest blob:**
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
    @PrimaryKey val localId: String,       // UUID, device-generated
    val serverBlobId: String?,             // null until uploaded
    val thumbnailBlobId: String?,
    val filename: String,
    val takenAt: Long,                     // epoch millis
    val mimeType: String,
    val width: Int, val height: Int,
    val latitude: Double?, val longitude: Double?,
    val localPath: String?,                // MediaStore URI
    val syncStatus: SyncStatus,            // PENDING | UPLOADING | SYNCED | FAILED
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

// Many-to-many photo ↔ album
@Entity("photo_album_xref", primaryKeys = ["photoLocalId", "albumLocalId"])
data class PhotoAlbumXRef(val photoLocalId: String, val albumLocalId: String)

// Upload work queue processed by BackupWorker
@Entity("blob_queue")
data class BlobQueueEntity(
    @PrimaryKey val id: String,
    val photoLocalId: String?,
    val albumLocalId: String?,
    val blobType: String,    // "photo" | "thumbnail" | "album_manifest"
    val priority: Int,       // 0=thumbnail, 1=photo, 2=album_manifest
    val attempts: Int,
    val lastAttemptAt: Long?,
    val status: String       // "queued" | "in_progress" | "done" | "failed"
)
```

---

## Encryption Flow

### Key Setup (one-time, Android)
1. User enters passphrase on `PassphraseSetupScreen`
2. App generates random 16-byte salt (stored in `EncryptedSharedPreferences`)
3. **Argon2id** (via BouncyCastle `Argon2BytesGenerator`) derives 32-byte DEK:
   `memory=65536 KB, time=3, parallelism=4`
4. DEK is wrapped by an **Android Keystore** AES-GCM key and stored encrypted in `EncryptedSharedPreferences`
5. Passphrase is discarded from memory; only the wrapped DEK persists

### Key Setup (browser)
1. User enters passphrase on `/setup` page
2. WASM Argon2id (`hash-wasm`) derives 32-byte key using same parameters + client-side salt
3. Key imported as `CryptoKey` via `crypto.subtle.importKey` for AES-GCM
4. Stored in `sessionStorage` (cleared on tab close — intentionally not persisted)

### Upload Path (per photo)
1. Read JPEG from MediaStore; extract EXIF via `ExifInterface`
2. Generate 256×256 JPEG thumbnail
3. Build JSON payloads for thumbnail and photo
4. Encrypt each: `nonce(12 bytes, SecureRandom) || AES-GCM(DEK, nonce, plaintext_json)`
5. Upload thumbnail first → receive `thumbnail_blob_id`
6. Insert `thumbnail_blob_id` into photo payload, re-encrypt, upload → receive `photo_blob_id`
7. Update Room DB: `syncStatus = SYNCED`, store blob IDs
8. Re-encrypt and re-upload any album manifests referencing this photo (delete old blob)

### Download / Display Path
- Gallery: download + decrypt **thumbnail** blobs only (~20 KB each, fast)
- Viewer: download + decrypt full photo blob on demand
- Custom Coil `Fetcher`: blob URL → API call → AES-GCM decrypt → `Bitmap` (runs on IO thread)

### Album Sync
- Albums are replaced on modification: re-encrypt manifest with new nonce → upload → delete old blob → update `serverManifestBlobId` in Room

---

## Backup Sync (BackupWorker)

- `PeriodicWorkRequest` every 1 hour
- Constraints: `NetworkType.UNMETERED` (WiFi-only, user-configurable), `BATTERY_NOT_LOW`
- Backoff: exponential starting at 15 min

**Worker logic:**
1. Query MediaStore for photos newer than `last_sync_timestamp` (DataStore Preferences)
2. Insert new photos as `PENDING` in Room; enqueue `BlobQueueEntity` rows (thumbnail + photo)
3. Process queue in priority order (thumbnail=0 → photo=1 → album_manifest=2)
4. On success: update Room, mark queue entry `done`
5. On failure: increment `attempts`; mark `failed` after 5 attempts
6. Update `last_sync_timestamp`

**Deduplication:** SHA-256 of plaintext sent as `X-Client-Hash`. On re-install, client compares server blob hashes with local files to skip re-uploads.

---

## 2FA Flow

### Enabling 2FA
1. User opens Settings → "Enable Two-Factor Authentication"
2. Client calls `POST /api/auth/2fa/setup`
3. Server generates 20-byte TOTP secret (base32), stores temporarily (`totp_enabled=0`), returns `otpauth://` URI + 10 backup codes
4. Client displays QR code (scanned by authenticator app)
5. Client displays 10 backup codes for user to save
6. User enters 6-digit code from authenticator app → client calls `POST /api/auth/2fa/confirm`
7. Server verifies via `totp-rs` → sets `totp_enabled=1`

### Login with 2FA
1. `POST /api/auth/login` → `{ requires_totp: true, totp_session_token }`
2. Client shows TOTP input
3. `POST /api/auth/login/totp` → full auth tokens

### Backup Codes
- 10 codes generated at 2FA setup, displayed once
- Stored as SHA-256 hashes in `totp_backup_codes` table
- Each code single-use (`used=1` after consumption)
- Re-generating 2FA setup issues new backup codes, invalidating old ones

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
# Configurable storage root — change this to any local path
root = "/data/storage"
default_quota_bytes = 10737418240   # 10 GB per user (0 = unlimited)
max_blob_size_bytes = 524288000     # 500 MB per individual upload

[auth]
# Change this to a random 64-char hex string in production!
jwt_secret = "CHANGE_ME_RANDOM_64_CHAR_HEX"
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true           # set false after initial setup
bcrypt_cost = 12

[web]
# Path to built React SPA (web/dist/). Set to "" to disable web frontend.
static_root = "./web/dist"
```

Config loaded via `SIMPLE_PHOTOS_CONFIG` env var, defaults to `./config.toml`.

---

## Blob Storage Layout on Disk

```
{storage.root}/
  <user_id[0..2]>/
    <user_id>/
      <blob_id[0..2]>/
        <blob_id>.bin
```

Two-level sharding avoids filesystem issues with millions of files per directory. `storage_path` in DB is relative to `storage.root` — root can be moved without a DB migration.

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
RUN apt-get update && apt-get install -y ca-certificates libsqlite3-0 curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/simple-photos-server ./
COPY --from=builder /usr/src/app/migrations ./migrations
RUN mkdir -p /data/storage /data/db
ENV SIMPLE_PHOTOS_CONFIG=/app/config.toml
EXPOSE 3000
ENTRYPOINT ["./simple-photos-server"]
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
      - ./web/dist:/app/web/dist:ro    # pre-built web frontend
      - photos_db:/data/db
      - photos_storage:/data/storage
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

**Non-Docker deployment:**
```bash
cargo build --release
SIMPLE_PHOTOS_CONFIG=/etc/simple-photos/config.toml ./target/release/simple-photos-server
```

---

## Key Rust Crates (`server/Cargo.toml`)

| Crate | Purpose |
|---|---|
| `axum 0.7` | Web framework |
| `sqlx 0.7` (sqlite, migrate) | Async DB with compile-time checked queries + embedded migrations |
| `tokio 1` (full) | Async runtime |
| `tower-http 0.5` | CORS, tracing, request body size limit |
| `jsonwebtoken 9` | JWT signing/verification (HS256) |
| `bcrypt 0.15` | Password hashing |
| `totp-rs 5` | TOTP secret generation + code verification (RFC 6238) |
| `uuid 1` (v4) | Blob and user IDs |
| `sha2 0.10` | SHA-256 for client_hash and refresh/backup token hashing |
| `serde / serde_json` | Serialization |
| `toml 0.8` | Config file parsing |
| `chrono 0.4` | Timestamps + ISO8601 |
| `thiserror / anyhow` | Error handling |
| `tracing / tracing-subscriber` | Structured logging |
| `tower-serve-static` or `include_dir` | Serve web frontend static files |

---

## Key Android Libraries

| Library | Purpose |
|---|---|
| Jetpack Compose BOM 2024.05 | Declarative UI |
| Hilt 2.51 | Dependency injection (incl. WorkManager integration via `hilt-work`) |
| Room 2.6 | Local SQLite ORM with Flow-based live queries |
| WorkManager 2.9 | Reliable background backup (Doze/app-standby safe) |
| Retrofit 2.11 + OkHttp 4.12 | Type-safe REST client; interceptor adds JWT header |
| Coil 2.6 | Image loading; custom `Fetcher` decrypts blobs before display |
| BouncyCastle `bcpkix-jdk15to18` 1.78 | Argon2id key derivation |
| `androidx.security:security-crypto` | `EncryptedSharedPreferences` for salt + wrapped DEK |
| DataStore Preferences | Server URL, WiFi-only toggle, last_sync_timestamp |
| Navigation Compose 2.7 | Type-safe navigation graph |
| ExifInterface 1.3 | Read date/GPS/dimensions from JPEG before encryption |
| Accompanist Permissions 0.34 | Compose-idiomatic `READ_MEDIA_IMAGES` permission flow |

---

## Key Web Libraries

| Library | Purpose |
|---|---|
| React 18 + TypeScript | UI framework |
| Vite | Build tool |
| TanStack Router | Type-safe SPA routing |
| TanStack Query | API data fetching + caching |
| Zustand | Lightweight global state |
| Dexie.js | IndexedDB wrapper (local photo/album cache) |
| Web Crypto API | AES-256-GCM encrypt/decrypt (browser built-in) |
| `hash-wasm` | Argon2id in browser via WASM |
| `qrcode.react` | QR code display for TOTP setup |
| shadcn/ui + Tailwind CSS | UI components and styling |
| `react-photo-album` | Masonry/grid photo layout |

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
| `GalleryScreen` | Staggered/grid photo view sorted by `takenAt`, lazy-loaded thumbnails |
| `AlbumListScreen` | Album grid with cover thumbnails |
| `AlbumDetailScreen` | Photos in album; add/remove |
| `PhotoViewerScreen` | Full-res, swipe navigation, share, delete |
| `SettingsScreen` | Server URL, WiFi-only toggle, backup status, 2FA management, logout |
| `ServerSetupScreen` | Enter server URL, test connection |
| `PassphraseSetupScreen` | Enter + confirm passphrase; Argon2id → wrapped DEK |
| `LoginScreen` | Username + password; inline TOTP field if 2FA enabled |
| `RegisterScreen` | New account creation |
| `TwoFactorSetupScreen` | QR code display, confirm code, show backup codes |

### Web
| Page | Features |
|---|---|
| `/gallery` | Masonry photo grid, lazy-loaded thumbnails |
| `/albums` | Album grid + detail view |
| `/photo/:id` | Lightbox full-res viewer, keyboard navigation, download |
| `/login` | Username + password + TOTP code (if enabled) |
| `/register` | New account |
| `/setup` | First-visit passphrase setup |
| `/settings` | Change password, enable/disable 2FA with QR code, backup codes |

---

## Verification / Testing

### Server
```bash
cd server
cargo build
cargo test
cp config.example.toml config.toml   # set jwt_secret and paths
./target/debug/simple-photos-server

# Smoke tests
curl http://localhost:3000/health
curl -X POST http://localhost:3000/api/auth/register \
  -H 'Content-Type: application/json' \
  -d '{"username":"test","password":"password123"}'
curl -X POST http://localhost:3000/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"test","password":"password123"}'

# Docker
docker-compose up --build
```

### Web Frontend
```bash
cd web
npm install
npm run dev          # Vite dev server, proxy /api to localhost:3000

# Test flows:
# 1. Register + login
# 2. Passphrase setup (Argon2id WASM → sessionStorage)
# 3. Upload a photo via browser drag-drop
# 4. Verify thumbnail in gallery
# 5. Enable 2FA: scan QR with authenticator app, confirm code
# 6. Log out + log back in with TOTP code
# 7. Use backup code instead of TOTP

npm run build        # produces web/dist/ for server to serve
```

### Android
- Run on emulator or device
- First-run: enter server URL → enter passphrase
- Register account, grant `READ_MEDIA_IMAGES`
- Verify backup worker triggers (trigger manually via WorkManager test API)
- Check Room DB for `SYNCED` entries after backup
- Verify decrypted thumbnails appear in gallery
- Open full photo in viewer, verify decryption
- Create album, add photos, verify album manifest re-upload
- Enable 2FA in Settings, log out, log back in with TOTP code
