# Simple Photos — Complete API Reference

All endpoints are prefixed with `/api` unless noted. Auth = `Authorization: Bearer <access_token>`. Admin = auth + admin role. Rate-limited endpoints noted with ⏱.

---

## Health

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/health` | None | — | `{ status: "ok", service: "simple-photos", version }` |

---

## Setup (public — only work when 0 users exist, except `/setup/status`)

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/setup/status` | None | — | `{ setup_complete: bool, registration_open: bool, version: string }` |
| `POST` | `/api/setup/init` | None (0 users) | `{ username, password }` | **201** `{ user_id, username, message }` |
| `GET` | `/api/setup/discover` | None (0 users) | — | `{ servers: [{ address, name, version }] }` |
| `POST` | `/api/setup/pair` | None (0 users) | `{ main_server_url, username, password }` | **201** `{ message, user_id, username, access_token, refresh_token, main_server_url }` |
| `POST` | `/api/setup/verify-backup` | None (0 users) | `{ address, username, password }` | `{ address, name, version, api_key?, photo_count }` |

---

## Auth ⏱

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `POST` | `/api/auth/register` | None ⏱ | `{ username, password }` | **201** `{ user_id, username }` |
| `POST` | `/api/auth/login` | None ⏱ | `{ username, password }` | `{ access_token, refresh_token, expires_in }` OR `{ requires_totp: true, totp_session_token }` |
| `POST` | `/api/auth/login/totp` | None ⏱ | `{ totp_session_token, totp_code?, backup_code? }` | `{ access_token, refresh_token, expires_in }` |
| `POST` | `/api/auth/refresh` | None ⏱ | `{ refresh_token }` | `{ access_token, refresh_token, expires_in }` |
| `POST` | `/api/auth/logout` | None ⏱ | `{ refresh_token }` | **204** |
| `PUT` | `/api/auth/password` | Bearer ⏱ | `{ current_password, new_password }` | **200** |
| `POST` | `/api/auth/verify-password` | Bearer ⏱ | `{ password }` | **200** |

### 2FA

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/auth/2fa/status` | Bearer | — | `{ totp_enabled: bool }` |
| `POST` | `/api/auth/2fa/setup` | Bearer | — | `{ otpauth_uri, backup_codes: [string] }` |
| `POST` | `/api/auth/2fa/confirm` | Bearer ⏱ | `{ totp_code }` | **200** |
| `POST` | `/api/auth/2fa/disable` | Bearer ⏱ | `{ totp_code }` | **204** |

---

## Blobs

| Method | Path | Auth | Request Body | Headers | Response |
|--------|------|------|-------------|---------|----------|
| `POST` | `/api/blobs` | Bearer | raw bytes (streaming) | `x-blob-type` (photo\|gif\|video\|audio\|thumbnail\|video_thumbnail\|album_manifest), `x-client-hash` (SHA-256 hex), `x-content-hash` (pre-encryption hash) | **201** `{ blob_id, upload_time, size }` |
| `GET` | `/api/blobs` | Bearer | — | Query: `blob_type`, `after`, `limit` | `{ blobs: [{ id, blob_type, size_bytes, client_hash, upload_time, content_hash }], next_cursor? }` |
| `GET` | `/api/blobs/{id}` | Bearer | — | Supports `Range`, `If-None-Match` → ETag/304 | binary stream (`application/octet-stream`) |
| `DELETE` | `/api/blobs/{id}` | Bearer | — | — | **204** |
| `GET` | `/api/blobs/{id}/thumb` | Bearer | — | — | binary stream (thumbnail via `encrypted_thumb_blob_id` join) |

---

## Photos

| Method | Path | Auth | Request Body / Headers | Response |
|--------|------|------|----------------------|----------|
| `GET` | `/api/photos` | Bearer | Query: `after`, `limit`, `media_type`, `favorites_only` | `{ photos: [PhotoRecord], next_cursor? }` |
| `POST` | `/api/photos/register` | Bearer | `{ filename, file_path, mime_type, media_type?, size_bytes, width?, height?, duration_secs?, taken_at?, latitude?, longitude? }` | **201** `{ photo_id, thumb_path, photo_hash }` |
| `POST` | `/api/photos/upload` | Bearer | raw bytes; Headers: `X-Filename`, `X-Mime-Type` | **201** `{ photo_id, filename, file_path, size_bytes, photo_hash }` (or **200** with existing record if hash dedup matches) |
| `GET` | `/api/photos/{id}/file` | Bearer | — | streaming file; supports Range, ETag, 304 |
| `GET` | `/api/photos/{id}/thumb` | Bearer | — | `image/jpeg` stream; or **202** `{ status: "pending" }` |
| `GET` | `/api/photos/{id}/web` | Bearer | — | web-compatible stream; or **202** `{ status: "converting" }` |
| `PUT` | `/api/photos/{id}/favorite` | Bearer | — | `{ id, is_favorite }` |
| `PUT` | `/api/photos/{id}/crop` | Bearer | `{ crop_metadata?: string (JSON) }` | `{ id, crop_metadata }` |
| `DELETE` | `/api/photos/{id}` | Bearer | — | **204** (soft-deletes to trash) |

**PhotoRecord fields:** `id, filename, file_path, mime_type, media_type, size_bytes, width, height, duration_secs, taken_at, latitude, longitude, thumb_path, created_at, is_favorite, crop_metadata, camera_model, photo_hash`

### Encrypted Sync

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/photos/encrypted-sync` | Bearer | Query: `after`, `limit` | `{ photos: [{ id, filename, mime_type, media_type, size_bytes, width, height, duration_secs, taken_at, created_at, encrypted_blob_id, encrypted_thumb_blob_id, is_favorite, crop_metadata, photo_hash }], next_cursor? }` |

### Edit Copies & Duplicates

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `POST` | `/api/photos/{id}/duplicate` | Bearer | `{ crop_metadata?: string }` | **201** `{ id, source_photo_id, filename, crop_metadata }` |
| `POST` | `/api/photos/{id}/copies` | Bearer | `{ name?, edit_metadata: string (JSON) }` | `{ id, photo_id, name, edit_metadata }` |
| `GET` | `/api/photos/{id}/copies` | Bearer | — | `{ copies: [{ id, name, edit_metadata, created_at }] }` |
| `DELETE` | `/api/photos/{id}/copies/{copy_id}` | Bearer | — | `{ ok: true }` |

### Conversion Status

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/photos/conversion-status` | Bearer | — | `{ pending_conversions, pending_awaiting_key, missing_thumbnails, converting: bool, enc_missing_thumbs, key_available: bool }` |

### Cleanup

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/photos/cleanup-status` | Bearer | — | `{ cleanable_count, cleanable_bytes }` |
| `POST` | `/api/photos/cleanup` | Bearer | — | `{ cleaned, errors?, message }` |

---

## Trash (30-day retention)

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/trash` | Bearer | Query: `after`, `limit` | `{ items: [TrashItem], next_cursor? }` |
| `DELETE` | `/api/trash` | Bearer | — | `{ deleted, message }` |
| `DELETE` | `/api/trash/{id}` | Bearer | — | **204** (permanent delete) |
| `POST` | `/api/trash/{id}/restore` | Bearer | — | **204** |
| `GET` | `/api/trash/{id}/thumb` | Bearer | — | `image/jpeg` stream |
| `POST` | `/api/blobs/{id}/trash` | Bearer | `{ filename, mime_type, media_type?, size_bytes?, width?, height?, duration_secs?, taken_at?, thumbnail_blob_id? }` | `{ trash_id, expires_at }` |

**TrashItem fields:** `id, photo_id, filename, file_path, mime_type, media_type, size_bytes, width, height, duration_secs, taken_at, latitude, longitude, thumb_path, deleted_at, expires_at, encrypted_blob_id, thumbnail_blob_id`

---

## Secure Galleries

| Method | Path | Auth | Request Body / Headers | Response |
|--------|------|------|----------------------|----------|
| `GET` | `/api/galleries/secure` | Bearer | — | `{ galleries: [{ id, name, created_at, item_count }] }` |
| `POST` | `/api/galleries/secure` | Bearer | `{ name }` | **201** `{ gallery_id, name }` |
| `POST` | `/api/galleries/secure/unlock` | Bearer | `{ password }` | `{ gallery_token, expires_in }` |
| `GET` | `/api/galleries/secure/blob-ids` | Bearer | — | `{ blob_ids: [string] }` |
| `DELETE` | `/api/galleries/secure/{id}` | Bearer | — | **204** |
| `GET` | `/api/galleries/secure/{id}/items` | Bearer | Header: `x-gallery-token` | `{ items: [{ id, blob_id, added_at }] }` |
| `POST` | `/api/galleries/secure/{id}/items` | Bearer | `{ blob_id }` | **201** `{ item_id }` |

---

## Shared Albums

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/sharing/albums` | Bearer | — | `[{ id, name, owner_username, is_owner, photo_count, member_count, created_at }]` |
| `POST` | `/api/sharing/albums` | Bearer | `{ name }` | **201** `{ id, name, created_at }` |
| `DELETE` | `/api/sharing/albums/{id}` | Bearer (owner) | — | **204** |
| `GET` | `/api/sharing/albums/{id}/members` | Bearer (member) | — | `[{ id, user_id, username, added_at }]` |
| `POST` | `/api/sharing/albums/{id}/members` | Bearer (owner) | `{ user_id }` | **201** `{ member_id, user_id }` |
| `DELETE` | `/api/sharing/albums/{id}/members/{user_id}` | Bearer (owner) | — | **204** |
| `GET` | `/api/sharing/albums/{id}/photos` | Bearer (member) | — | `[{ id, photo_ref, ref_type, added_at }]` |
| `POST` | `/api/sharing/albums/{id}/photos` | Bearer (member) | `{ photo_ref, ref_type: "blob" }` | **201** `{ photo_id }` |
| `DELETE` | `/api/sharing/albums/{album_id}/photos/{photo_id}` | Bearer (member) | — | **204** |
| `GET` | `/api/sharing/users` | Bearer | — | `[{ id, username }]` |

---

## Tags & Search

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/tags` | Bearer | — | `{ tags: [string] }` |
| `GET` | `/api/photos/{id}/tags` | Bearer | — | `{ photo_id, tags: [string] }` |
| `POST` | `/api/photos/{id}/tags` | Bearer | `{ tag }` | **201** |
| `DELETE` | `/api/photos/{id}/tags` | Bearer | `{ tag }` | **204** |
| `GET` | `/api/search` | Bearer | Query: `q`, `limit` | `{ results: [{ id, filename, media_type, mime_type, thumb_path, created_at, taken_at, latitude, longitude, width, height, tags }] }` |

---

## Google Photos Import / Metadata

| Method | Path | Auth | Request Body / Headers | Response |
|--------|------|------|----------------------|----------|
| `POST` | `/api/import/metadata` | Bearer | `{ metadata: GooglePhotosMetadata, photo_id?, blob_id? }` | **201** `{ metadata_id, storage_path?, is_encrypted }` |
| `POST` | `/api/import/metadata/batch` | Bearer | `{ entries: [{ metadata, photo_id?, blob_id? }] }` | `{ imported, failed, results: [{ index, metadata_id?, error? }] }` |
| `POST` | `/api/import/metadata/upload` | Bearer | raw JSON sidecar bytes; Headers: `X-Photo-Id`, `X-Blob-Id` | **201** `{ metadata_id, storage_path?, is_encrypted }` |
| `GET` | `/api/photos/{id}/metadata` | Bearer | — | `{ metadata: [PhotoMetadataRecord], next_cursor? }` |
| `DELETE` | `/api/photos/{id}/metadata` | Bearer | — | **204** |

---

## Client Logs

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `POST` | `/api/client-logs` | Bearer | `{ session_id, entries: [{ level, tag, message, context?, client_ts }] }` | `{ inserted }` |
| `GET` | `/api/admin/client-logs` | Admin | Query: `user_id`, `session_id`, `level`, `after`, `limit` | `{ logs: [{ id, user_id, session_id, level, tag, message, context?, client_ts, created_at }], next_cursor? }` |

---

## Settings

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/settings/encryption` | Bearer | — | `{ encryption_mode }` (always `"encrypted"`) |
| `GET` | `/api/settings/storage-stats` | Bearer | — | `{ photo_bytes, photo_count, video_bytes, video_count, other_blob_bytes, other_blob_count, user_total_bytes, fs_total_bytes, fs_free_bytes }` |
| `GET` | `/api/settings/audio-backup` | Bearer | — | `{ audio_backup_enabled: bool }` |

---

## Downloads (public)

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/downloads/android` | None | — | APK binary (`application/vnd.android.package-archive`) or **404** with error JSON |

---

## Admin — User Management

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `POST` | `/api/admin/users` | Admin | `{ username, password, role?: "admin"\|"user" }` | **201** `{ user_id, username, role }` |
| `GET` | `/api/admin/users` | Admin | — | `[{ id, username, role, totp_enabled, created_at }]` |
| `DELETE` | `/api/admin/users/{id}` | Admin | — | **204** |
| `PUT` | `/api/admin/users/{id}/role` | Admin | `{ role: "admin"\|"user" }` | `{ message, user_id, role }` |
| `PUT` | `/api/admin/users/{id}/password` | Admin | `{ new_password }` | `{ message }` |
| `DELETE` | `/api/admin/users/{id}/2fa` | Admin | — | `{ message }` |
| `POST` | `/api/admin/users/{id}/2fa/setup` | Admin | — | `{ otpauth_uri, backup_codes }` |
| `POST` | `/api/admin/users/{id}/2fa/confirm` | Admin | `{ totp_code }` | `{ message }` |

---

## Admin — Storage & Server Config

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/admin/storage` | Admin | — | `{ storage_path, message }` |
| `PUT` | `/api/admin/storage` | Admin | `{ path }` | `{ storage_path, message }` |
| `GET` | `/api/admin/browse` | Admin | Query: `path` | `{ current_path, parent_path?, directories: [{ name, path }], writable }` |
| `GET` | `/api/admin/port` | Admin | — | `{ port, message }` |
| `PUT` | `/api/admin/port` | Admin | `{ port: u16 }` | `{ port, message }` |
| `POST` | `/api/admin/restart` | Admin | — | `{ message }` |
| `GET` | `/api/admin/ssl` | Admin | — | `{ enabled, cert_path?, key_path?, message }` |
| `PUT` | `/api/admin/ssl` | Admin | `{ enabled, cert_path?, key_path? }` | `{ enabled, cert_path?, key_path?, message }` |

---

## Admin — Server Import & Scan

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/admin/import/scan` | Admin | Query: `path` | `{ directory, files: [{ name, path, size, mime_type, modified? }], total_size }` |
| `GET` | `/api/admin/import/file` | Admin | Query: `path` | streaming binary |
| `POST` | `/api/admin/photos/scan` | Admin | — | `{ registered, metadata_updated, skipped_audio, message }` |
| `POST` | `/api/admin/photos/auto-scan` | Admin | — | `{ message: "Scan complete", new_count }` |
| `GET` | `/api/admin/import/google-photos/scan` | Admin | Query: `path` | `{ directory, media_files, sidecar_files, paired, unpaired_media: [string], unpaired_sidecars: [string] }` |
| `POST` | `/api/admin/import/google-photos` | Admin | `{ path }` | `{ photos_imported, metadata_imported, errors: [string] }` |

---

## Admin — Encryption

Encryption is always enabled (AES-256-GCM). There is no plain mode or migration flow.

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `POST` | `/api/admin/encryption/store-key` | Admin | `{ key: string (64-char hex, 32-byte AES-256-GCM key) }` | **200** `{ message: "Encryption key stored" }` |

**Errors:** `400` invalid key format, `401`/`403` not admin.

---

## Admin — Conversion

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `POST` | `/api/admin/photos/convert` | Admin | — | **202** `{ message: "Conversion triggered" }` |
| `POST` | `/api/admin/photos/reconvert` | Admin | `{ key_hex: string (64 hex) }` | **202** `{ message, needs_conversion }` |

---

## Admin — Backup Servers

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/admin/backup/servers` | Admin | — | `{ servers: [{ id, name, address, sync_frequency_hours, last_sync_at, last_sync_status, last_sync_error, enabled, created_at }] }` |
| `POST` | `/api/admin/backup/servers` | Admin | `{ name, address, api_key?, sync_frequency_hours? }` | **201** `{ id, name, address, sync_frequency_hours }` |
| `PUT` | `/api/admin/backup/servers/{id}` | Admin | `{ name?, address?, api_key?, sync_frequency_hours?, enabled? }` | `{ message, id }` |
| `DELETE` | `/api/admin/backup/servers/{id}` | Admin | — | **204** |
| `GET` | `/api/admin/backup/servers/{id}/status` | Admin | — | `{ reachable, version?, error? }` |
| `GET` | `/api/admin/backup/servers/{id}/logs` | Admin | — | `[{ id, server_id, started_at, completed_at, status, photos_synced, bytes_synced, error }]` |
| `POST` | `/api/admin/backup/servers/{id}/sync` | Admin | — | `{ message: "Sync started", sync_id }` |
| `POST` | `/api/admin/backup/servers/{id}/recover` | Admin | — | **202** `{ message, recovery_id }` |
| `GET` | `/api/admin/backup/servers/{id}/photos` | Admin | — | `[BackupPhotoRecord]` (proxied from backup server) |
| `GET` | `/api/admin/backup/discover` | Admin | — | `{ servers: [{ address, name, version }] }` |

### Backup Mode

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/admin/backup/mode` | Admin | — | `{ mode, server_ip, server_address, port, api_key? }` |
| `POST` | `/api/admin/backup/mode` | Admin | `{ mode: "primary"\|"backup" }` | `{ mode, server_ip, server_address, port, api_key? }` |

### Audio Backup Toggle

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `PUT` | `/api/admin/audio-backup` | Admin | `{ audio_backup_enabled: bool }` | `{ audio_backup_enabled, message }` |

---

## Backup Serve (server-to-server, X-API-Key auth)

These endpoints are called by the primary server to sync/recover from a backup. Auth via `X-API-Key` header.

| Method | Path | Auth | Request Body / Headers | Response |
|--------|------|------|----------------------|----------|
| `GET` | `/api/backup/list` | X-API-Key | — | `[{ id, filename, file_path, mime_type, media_type, size_bytes, width, height, duration_secs, taken_at, latitude, longitude, thumb_path, created_at }]` |
| `GET` | `/api/backup/list-trash` | X-API-Key | — | `[{ id, file_path, size_bytes }]` |
| `GET` | `/api/backup/download/{photo_id}` | X-API-Key | — | streaming binary; Response header: `X-File-Path` |
| `GET` | `/api/backup/download/{photo_id}/thumb` | X-API-Key | — | `image/jpeg` stream |
| `POST` | `/api/backup/receive` | X-API-Key | raw bytes; Headers: `X-Photo-Id`, `X-File-Path`, `X-Source` ("photos"\|"trash"), `X-Content-Hash` (SHA-256 hex, optional) | `{ status: "ok", photo_id, size_bytes }` |

---

## Admin — Diagnostics

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/admin/diagnostics/config` | Admin | — | `{ diagnostics_enabled, client_diagnostics_enabled }` |
| `PUT` | `/api/admin/diagnostics/config` | Admin | `{ diagnostics_enabled?, client_diagnostics_enabled? }` | `{ diagnostics_enabled, client_diagnostics_enabled }` |
| `GET` | `/api/admin/diagnostics` | Admin | — | Full `DiagnosticsResponse` (see below) or `{ enabled: false, server: { version, uptime_seconds, started_at }, message }` when disabled |
| `GET` | `/api/admin/audit-logs` | Admin | Query: `event_type`, `user_id`, `ip_address`, `after`, `before`, `limit` | `{ logs: [{ id, event_type, user_id, username, ip_address, user_agent, details, created_at }], next_cursor?, total }` |

**DiagnosticsResponse shape:**
```json
{
  "enabled": true,
  "server": { "version", "uptime_seconds", "rust_version", "os", "arch", "memory_rss_bytes", "cpu_seconds", "pid", "storage_root", "db_path", "tls_enabled", "max_blob_size_mb", "started_at" },
  "database": { "size_bytes", "wal_size_bytes", "table_counts": { "users": N, ... }, "journal_mode", "page_size", "page_count", "freelist_count" },
  "storage": { "total_bytes", "file_count", "disk_total_bytes", "disk_available_bytes", "disk_used_percent" },
  "users": { "total_users", "admin_count", "totp_enabled_count" },
  "photos": { "total_photos", "encrypted_count", "total_file_bytes", "total_thumb_bytes", "photos_with_thumbs", "photos_by_media_type": {}, "oldest_photo", "newest_photo", "favorited_count", "tagged_count" },
  "audit": { "total_entries", "entries_last_24h", "entries_last_7d", "events_by_type": {}, "recent_failures": [{ "event_type", "ip_address", "user_agent", "created_at", "details" }] },
  "client_logs": { "total_entries", "entries_last_24h", "entries_last_7d", "by_level": {}, "unique_sessions" },
  "backup": { "server_count", "total_sync_logs", "last_sync_at" },
  "performance": { "db_ping_ms", "cache_hit_ratio" }
}
```

---

## External Diagnostics (HTTP Basic Auth — admin credentials)

Auth: `Authorization: Basic base64(username:password)` — must be an admin user.

| Method | Path | Auth | Request Body | Response |
|--------|------|------|-------------|----------|
| `GET` | `/api/external/diagnostics/health` | Basic | — | `{ status, version, uptime_seconds, started_at, memory_rss_bytes, cpu_seconds, db_ping_ms, disk_used_percent, total_photos, total_users }` |
| `GET` | `/api/external/diagnostics` | Basic | — | Full `DiagnosticsResponse` (same shape as admin diagnostics above) |
| `GET` | `/api/external/diagnostics/storage` | Basic | — | `{ storage: StorageStats, photos: { total_photos, total_file_bytes, total_thumb_bytes, photos_by_media_type }, database: { size_bytes, wal_size_bytes } }` |
| `GET` | `/api/external/diagnostics/audit` | Basic | — | `{ audit: AuditSummary, users: UserStats, client_logs: ClientLogSummary }` |

---

## Error Format

All errors return JSON:
```json
{
  "error": "Human-readable message"
}
```

Common status codes: `400` (bad request), `401` (unauthorized), `403` (forbidden/admin required), `404` (not found), `409` (conflict), `429` (rate limited), `500` (internal).
