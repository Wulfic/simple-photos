# Simple Photos

A self-hosted photo & video library with always-on end-to-end encryption, multi-device backup, and web + Android access.

## Features

### Core

- **Self-hosted** — runs on your own server, your data stays yours
- **End-to-end encryption** — always-on AES-256-GCM encryption with client-side key management, so only you can access your photos. Even if the server is compromised, your data remains secure.
- **Multi-user** — multiple accounts with per-user storage, shared albums, and admin controls
- **Themes** — full light/dark theme support
- **NO AI** — no AI features, no data collection, just a secure and private photo library

### Media

- **Photo, video, and audio support** — JPEG, PNG, GIF, WebP, AVIF, BMP, MP4, WebM, MP3, FLAC, OGG, WAV
- **Media conversion** — automatic FFmpeg-based conversion of non-native formats (HEIC → JPEG, MKV → MP4, WMA → MP3, and more) during import and scan
- **GIF autoplay** — animated GIFs play automatically in both the main gallery and secure galleries
- **Portrait orientation** — EXIF-aware dimension handling across upload, sync, and display for correct portrait rendering
- **Justified grid layout** — aspect-ratio-preserving gallery grid on both web and Android
- **Photo editing** — crop, rotate, and save copies with full metadata tracking
- **Thumbnails** — aspect-ratio-preserving thumbnails (512px) with EXIF orientation applied

### Organization

- **Albums** — organize photos into albums with optional sharing between users
- **Secure albums** — password-protected encrypted galleries for sensitive content, with support for multiple albums
- **Trash** — 30-day soft-delete with restore
- **Tags & search** — tag photos and search across your library
- **Favorites** — mark and filter favorite photos
- **Import** — drag-and-drop upload with Google Photos metadata import (single and batch)
- **Library export** — download your entire library as a ZIP archive

### Backup & Sync

- **Backup sync** — automatic server-to-server backup replication with support for multiple backup targets, including remote servers
- **Backup recovery** — restore a primary server from any backup with full data integrity (photos, blobs, albums, secure galleries, trash, metadata)
- **Encrypted sync** — encrypted blobs, thumbnails, and secure gallery data are fully replicated to backup servers
- **Conflict-free sync** — content-hash deduplication prevents duplicate blobs across primary and backup

### Security

- **2FA** — TOTP two-factor authentication with backup codes
- **Rate limiting** — brute-force protection on authentication endpoints
- **Audit logging** — server-side audit trail
- **TLS** — native HTTPS support with configurable certificates

### Android

- **Android app** — view and manage your library on the go with Jetpack Compose UI
- **Automatic backup** — background photo backup from your device via WorkManager
- **Justified grid** — aspect-ratio-preserving gallery layout matching the web experience
- **Server discovery** — automatic network scanning to find your server during setup

## Tech Stack

| Component | Technology |
|-----------|------------|
| Server    | Rust, Axum, SQLite (sqlx), FFmpeg |
| Web       | React, TypeScript, Vite, Tailwind CSS, Zustand, Dexie (IndexedDB) |
| Android   | Kotlin, Jetpack Compose, Hilt, Room, WorkManager |

## Getting Started

### Server

```bash
cd server
cp config.example.toml config.toml
# Edit config.toml with your settings
cargo build --release
./target/release/simple-photos-server
```

FFmpeg is required for media conversion, video thumbnails, and rendering. Install it via your system package manager.

### Web

```bash
cd web
npm install
npm run build
```

The built frontend is served by the Rust server from the configured `static_root`.

### Android

```bash
cd android
./gradlew assembleDebug
```

The APK is available at `android/app/build/outputs/apk/debug/app-debug.apk` and served by the server at `/api/downloads/android`.

### Docker

```bash
cd server
docker compose up -d
```

## API

See [API_REFERENCE.md](API_REFERENCE.md) for the full REST API documentation.

## Testing

The project includes a comprehensive end-to-end test suite covering authentication, media workflows, backup/recovery, multi-user scenarios, and more.

```bash
python3 -m pytest tests/ -v --tb=short
```

See [tests/README.md](tests/README.md) for details.

## Credits

- **Icons** — Custom icon set by [Angus_87](https://www.flaticon.com/authors/angus-87) on Flaticon
- **Developed by** WulfNet Designs

## Source Code

[GitHub Repository](https://github.com/wulfic/simple-photos)

## License

© 2026 WulfNet Designs. All rights reserved.
