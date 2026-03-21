# Simple Photos

A self-hosted photo & video library with always-on end-to-end encryption, multi-device backup, and web + Android access.

## Features

- **Self-hosted** — runs on your own server, your data stays yours
- **End-to-end encryption** — always-on AES-256-GCM encryption with client-side key management, so only you can access your photos. This means even if the server is compromised, your data remains secure.
- **Photo, video, and audio support** — JPEG, PNG, GIF, WebP, AVIF, BMP, MP4, WebM, MP3, FLAC, OGG, WAV
- **Albums** — organize photos into albums with optional sharing between users
- **Secure albums** — password-protected galleries for sensitive content. Supports multiple password protected albums.
- **Trash** — 30-day soft-delete with restore
- **Tags & search** — tag photos and search across your library
- **Backup sync** — automatic server-to-server backup replication, with support for multiple backup targets, including remote servers.
- **2FA** — TOTP two-factor authentication
- **Themes** — full light/dark theme support
- **Android app** — view and manage your library on the go, with automatic photo backup from your device
- **NO AI** — no AI features, no data collection, just a secure and private photo library

## Tech Stack

| Component | Technology |
|-----------|------------|
| Server    | Rust, Axum, SQLite (sqlx) |
| Web       | React, TypeScript, Vite, Tailwind CSS |
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

## Credits

- **Icons** — Custom icon set by [Angus_87](https://www.flaticon.com/authors/angus-87) on Flaticon
- **Developed by** WulfNet Designs

## Source Code

[GitHub Repository](https://github.com/wulfic/simple-photos)

## License

© 2026 WulfNet Designs. All rights reserved.
