/**
 * Diagnostics + audit-log DTOs — full server diagnostics report (admin
 * + external) and audit log entries.
 *
 * NOTE: `DiagnosticsConfigResponse` and `UpdateDiagnosticsConfigRequest`
 * are declared in `LogDto.kt` (legacy location) — do not redeclare here.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class DiagnosticsResponse(
    val enabled: Boolean = true,
    val server: ServerDiagnostics? = null,
    val database: DatabaseDiagnostics? = null,
    val storage: StorageDiagnostics? = null,
    val users: UserDiagnostics? = null,
    val photos: PhotoDiagnostics? = null,
    val audit: AuditDiagnostics? = null,
    @SerializedName("client_logs") val clientLogs: ClientLogDiagnostics? = null,
    val backup: BackupDiagnostics? = null,
    val performance: PerformanceDiagnostics? = null,
    val message: String? = null,
)

data class ServerDiagnostics(
    val version: String? = null,
    @SerializedName("uptime_seconds") val uptimeSeconds: Long? = null,
    @SerializedName("rust_version") val rustVersion: String? = null,
    val os: String? = null,
    val arch: String? = null,
    @SerializedName("memory_rss_bytes") val memoryRssBytes: Long? = null,
    @SerializedName("cpu_seconds") val cpuSeconds: Double? = null,
    val pid: Long? = null,
    @SerializedName("storage_root") val storageRoot: String? = null,
    @SerializedName("db_path") val dbPath: String? = null,
    @SerializedName("tls_enabled") val tlsEnabled: Boolean? = null,
    @SerializedName("max_blob_size_mb") val maxBlobSizeMb: Long? = null,
    @SerializedName("started_at") val startedAt: String? = null,
)

data class DatabaseDiagnostics(
    @SerializedName("size_bytes") val sizeBytes: Long? = null,
    @SerializedName("wal_size_bytes") val walSizeBytes: Long? = null,
    @SerializedName("table_counts") val tableCounts: Map<String, Long>? = null,
    @SerializedName("journal_mode") val journalMode: String? = null,
    @SerializedName("page_size") val pageSize: Long? = null,
    @SerializedName("page_count") val pageCount: Long? = null,
    @SerializedName("freelist_count") val freelistCount: Long? = null,
)

data class StorageDiagnostics(
    @SerializedName("total_bytes") val totalBytes: Long? = null,
    @SerializedName("file_count") val fileCount: Long? = null,
    @SerializedName("disk_total_bytes") val diskTotalBytes: Long? = null,
    @SerializedName("disk_available_bytes") val diskAvailableBytes: Long? = null,
    @SerializedName("disk_used_percent") val diskUsedPercent: Double? = null,
)

data class UserDiagnostics(
    @SerializedName("total_users") val totalUsers: Long? = null,
    @SerializedName("admin_count") val adminCount: Long? = null,
    @SerializedName("totp_enabled_count") val totpEnabledCount: Long? = null,
)

data class PhotoDiagnostics(
    @SerializedName("total_photos") val totalPhotos: Long? = null,
    @SerializedName("encrypted_count") val encryptedCount: Long? = null,
    @SerializedName("total_file_bytes") val totalFileBytes: Long? = null,
    @SerializedName("total_thumb_bytes") val totalThumbBytes: Long? = null,
    @SerializedName("photos_with_thumbs") val photosWithThumbs: Long? = null,
    @SerializedName("photos_by_media_type") val photosByMediaType: Map<String, Long>? = null,
    @SerializedName("oldest_photo") val oldestPhoto: String? = null,
    @SerializedName("newest_photo") val newestPhoto: String? = null,
    @SerializedName("favorited_count") val favoritedCount: Long? = null,
    @SerializedName("tagged_count") val taggedCount: Long? = null,
)

data class AuditDiagnostics(
    @SerializedName("total_entries") val totalEntries: Long? = null,
    @SerializedName("entries_last_24h") val entriesLast24h: Long? = null,
    @SerializedName("entries_last_7d") val entriesLast7d: Long? = null,
    @SerializedName("events_by_type") val eventsByType: Map<String, Long>? = null,
    @SerializedName("recent_failures") val recentFailures: List<AuditFailureEntry>? = null,
)

data class AuditFailureEntry(
    @SerializedName("event_type") val eventType: String,
    @SerializedName("ip_address") val ipAddress: String? = null,
    @SerializedName("user_agent") val userAgent: String? = null,
    @SerializedName("created_at") val createdAt: String,
    val details: String? = null,
)

data class ClientLogDiagnostics(
    @SerializedName("total_entries") val totalEntries: Long? = null,
    @SerializedName("entries_last_24h") val entriesLast24h: Long? = null,
    @SerializedName("entries_last_7d") val entriesLast7d: Long? = null,
    @SerializedName("by_level") val byLevel: Map<String, Long>? = null,
    @SerializedName("unique_sessions") val uniqueSessions: Long? = null,
)

data class BackupDiagnostics(
    @SerializedName("server_count") val serverCount: Long? = null,
    @SerializedName("total_sync_logs") val totalSyncLogs: Long? = null,
    @SerializedName("last_sync_at") val lastSyncAt: String? = null,
)

data class PerformanceDiagnostics(
    @SerializedName("db_ping_ms") val dbPingMs: Double? = null,
    @SerializedName("cache_hit_ratio") val cacheHitRatio: Double? = null,
)

// ── Audit logs ──────────────────────────────────────────────────────────────

data class AuditLogEntry(
    val id: String,
    @SerializedName("event_type") val eventType: String,
    @SerializedName("user_id") val userId: String? = null,
    val username: String? = null,
    @SerializedName("ip_address") val ipAddress: String? = null,
    @SerializedName("user_agent") val userAgent: String? = null,
    val details: String? = null,
    @SerializedName("created_at") val createdAt: String,
)

data class AuditLogListResponse(
    val logs: List<AuditLogEntry>,
    @SerializedName("next_cursor") val nextCursor: String? = null,
    val total: Long? = null,
)
