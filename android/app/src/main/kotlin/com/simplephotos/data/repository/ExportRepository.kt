/**
 * Export pipeline repository — start, status polling, file listing,
 * and download streaming.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import okhttp3.ResponseBody
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class ExportRepository @Inject constructor(private val api: ApiService) {

    suspend fun start(
        scope: String? = null,
        photoIds: List<String>? = null,
        albumId: String? = null,
        includeMetadata: Boolean = true,
        decrypt: Boolean = true,
        stripGeo: Boolean = false,
    ): ExportStartResponse = api.startExport(
        ExportStartRequest(scope, photoIds, albumId, includeMetadata, decrypt, stripGeo)
    )

    suspend fun status(): ExportStatusResponse = api.getExportStatus()

    suspend fun listFiles(): List<ExportFile> = api.listExportFiles().files

    suspend fun download(fileId: String): ResponseBody = api.downloadExportFile(fileId)
}
