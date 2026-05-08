/**
 * Server-side import repository — admin import scan + Google Photos
 * pairing.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import okhttp3.ResponseBody
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class ImportRepository @Inject constructor(private val api: ApiService) {

    suspend fun scan(path: String? = null): ImportScanResponse = api.adminImportScan(path)

    suspend fun fetchFile(path: String): ResponseBody = api.adminImportFile(path)

    suspend fun googlePhotosScan(path: String): GooglePhotosScanResponse =
        api.adminGooglePhotosScan(path)

    suspend fun googlePhotosImport(path: String): GooglePhotosImportResponse =
        api.adminGooglePhotosImport(GooglePhotosImportRequest(path))
}
