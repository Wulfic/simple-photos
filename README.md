# Simple Photos

A self-hosted photo & video library with end-to-end encryption, multi-server backup & restore, and web + Android access.

###Authors Note
Simple Photos was born out of a desire for a secure, private, and user-friendly photo management solution that I could host myself. But found that other solutions lacked one critical thing, backup servers! Forcing users to choose between only having one backup target, or use multiple solutions; So I decided to build something from the ground up that met all my needs — and hopefully yours too!

## Features

### Core

- **Self-hosted** — runs on your own server, your data stays yours
- **End-to-end encryption** — always-on AES-256-GCM encryption with client-side key management, so only you can access your photos. Even if the server is compromised, your data remains secure.
- **Multi-user** — multiple accounts with per-user storage, shared albums, and admin controls
- **Themes** — full light/dark theme support

### Media

- **Supported Photo, video, and audio formats** — JPEG, PNG, GIF, WebP, AVIF, BMP, MP4, WebM, MP3, FLAC, OGG, WAV
- **Media conversion** — automatic FFmpeg-based conversion of non-native formats (I.E HEIC → JPEG, MKV → MP4, WMA → MP3) during import and scan
- **Photo/Video/Audio editing** — crop, rotate, adjust brightness, and trim videos/audio using non-destructive edits stored as metadata or rendered into a saved copy.  


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

### Security

- **2FA** — TOTP two-factor authentication with backup codes
- **Rate limiting** — brute-force protection on authentication endpoints
- **Audit logging** — server-side audit trail
- **TLS** — native HTTPS support with configurable certificates

### Android

- **Android app** — view and manage your library
- **Automatic backup** — background photo backup from your device
- **Server discovery** — automatic network scanning to find your server during setup

## Tech Stack

| Component | Technology |
|-----------|------------|
| Server    | Rust, Axum, SQLite (sqlx), FFmpeg |
| Web       | React, TypeScript, Vite, Tailwind CSS, Zustand, Dexie (IndexedDB) |
| Android   | Kotlin, Jetpack Compose, Hilt, Room, WorkManager |

## Getting Started

The install scripts handle everything — building the server, web frontend, and (optionally) the Android APK. They support both Docker and bare-metal (native) installations with auto-port detection, admin account creation, and backup server pairing.

### Linux / macOS

```bash
./install.sh
```

### Windows (PowerShell)

```powershell
.\install.ps1
```

### CLI Flags

```
--mode <native|docker>  Installation mode
--port <number>         Starting port (auto-increments if busy)
--role <primary|backup> Server role (default: primary)
--name <string>         Instance name (for Docker containers)
--storage <path>        Path to photo storage directory
--admin-user <string>   Admin username (skip interactive prompt)
--admin-pass <string>   Admin password (skip interactive prompt)
--backup-api-key <key>  Backup API key for backup servers
--primary-url <url>     Primary server URL (for backup pairing)
--no-build-android      Skip Android APK build prompt
--no-start              Don't start the server after install
--yes                   Auto-accept all prompts
--help                  Show this help
```

**Examples:**

```bash
# Native install on port 8080
./install.sh --mode native --port 8080

# Docker install as a backup server
./install.sh --mode docker --role backup --port 8081

# Fully non-interactive
./install.sh --mode docker --port 8080 --admin-user admin --admin-pass secret --yes
```

### Prerequisites

- **Rust** (for native builds)
- **Node.js** (for building the web frontend)
- **Docker** (for Docker-mode installs)
- **FFmpeg** (required for media conversion, video thumbnails, and rendering)
- **Android SDK** (optional, for building the Android APK)

All prerequisites are handled automatically by the install scripts, minus docker installation on Windows (which requires manual setup).

The built web frontend is served by the Rust server from the configured `static_root`. The Android APK is available at `android/app/build/outputs/apk/debug/app-debug.apk` and served by the server at `/api/downloads/android` (a download button is available in settings).

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

This project is licensed under the [MIT License](LICENSE).

© 2026 WulfNet Designs — [github.com/Wulfic/simple-photos](https://github.com/Wulfic/simple-photos)

Attribution is required when redistributing or using this software. Please retain the original copyright notice and link to the source repository.
