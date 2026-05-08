/**
 * Admin server-controls repository — port, restart, browse, storage path,
 * SSL update + Local CA bundle, backup mode, audit logs, full diagnostics,
 * and auto-scan trigger.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import okhttp3.ResponseBody
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class AdminServerRepository @Inject constructor(private val api: ApiService) {

    // Storage path
    suspend fun getStoragePath(): StoragePathResponse = api.getStoragePath()

    suspend fun updateStoragePath(path: String): StoragePathResponse =
        api.updateStoragePath(UpdateStoragePathRequest(path))

    suspend fun browse(path: String? = null): BrowseResponse = api.browseDirectory(path)

    // Port / restart
    suspend fun getPort(): PortResponse = api.getServerPort()

    suspend fun updatePort(port: Int): PortResponse = api.updateServerPort(UpdatePortRequest(port))

    suspend fun restart(): RestartResponse = api.restartServer()

    // SSL
    suspend fun getSslStatus(): SslStatusResponse = api.getSslStatus()

    suspend fun updateSsl(
        enabled: Boolean,
        certPath: String? = null,
        keyPath: String? = null,
    ): SslStatusResponse =
        api.updateSslConfig(UpdateSslRequest(enabled, certPath, keyPath))

    suspend fun downloadLocalCaBundle(): ResponseBody = api.downloadLocalCaBundle()

    // Backup mode
    suspend fun getBackupMode(): BackupModeResponse = api.getBackupMode()

    suspend fun setBackupMode(mode: String): BackupModeResponse =
        api.setBackupMode(SetBackupModeRequest(mode))

    // Auto-scan
    suspend fun autoScan(): AutoScanResponse = api.autoScanPhotos()

    // Diagnostics + audit logs
    suspend fun diagnostics(): DiagnosticsResponse = api.getDiagnostics()

    suspend fun auditLogs(
        eventType: String? = null,
        userId: String? = null,
        ipAddress: String? = null,
        after: String? = null,
        before: String? = null,
        limit: Int? = null,
    ): AuditLogListResponse =
        api.listAuditLogs(eventType, userId, ipAddress, after, before, limit)
}
