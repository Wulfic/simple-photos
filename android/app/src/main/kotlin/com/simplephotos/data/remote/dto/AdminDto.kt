/**
 * Admin DTOs — port, restart, browse, storage path, SSL update,
 * Local CA bundle, backup mode toggle.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Storage path ────────────────────────────────────────────────────────────

data class StoragePathResponse(
    @SerializedName("storage_path") val storagePath: String,
    val message: String? = null,
)

data class UpdateStoragePathRequest(val path: String)

data class BrowseDirectory(val name: String, val path: String)

data class BrowseResponse(
    @SerializedName("current_path") val currentPath: String,
    @SerializedName("parent_path") val parentPath: String? = null,
    val directories: List<BrowseDirectory>,
    val writable: Boolean = false,
)

// ── Port / restart ──────────────────────────────────────────────────────────

data class PortResponse(val port: Int, val message: String? = null)
data class UpdatePortRequest(val port: Int)
data class RestartResponse(val message: String)

// ── SSL update ──────────────────────────────────────────────────────────────

data class UpdateSslRequest(
    val enabled: Boolean,
    @SerializedName("cert_path") val certPath: String? = null,
    @SerializedName("key_path") val keyPath: String? = null,
)

// ── Backup mode ─────────────────────────────────────────────────────────────

data class BackupModeResponse(
    val mode: String,
    @SerializedName("server_ip") val serverIp: String? = null,
    @SerializedName("server_address") val serverAddress: String? = null,
    val port: Int? = null,
    @SerializedName("api_key") val apiKey: String? = null,
)

data class SetBackupModeRequest(val mode: String) // "primary" | "backup"

// ── Auto-scan ───────────────────────────────────────────────────────────────

data class AutoScanResponse(
    val message: String,
    @SerializedName("new_count") val newCount: Int = 0,
)
