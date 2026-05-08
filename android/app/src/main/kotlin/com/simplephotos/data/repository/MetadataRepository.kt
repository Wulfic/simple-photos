/**
 * Photo metadata sidecar repository — Google Photos JSON imports and
 * per-photo metadata records.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class MetadataRepository @Inject constructor(private val api: ApiService) {

    suspend fun importMetadata(
        metadata: GooglePhotosMetadata,
        photoId: String? = null,
        blobId: String? = null,
    ): ImportMetadataResponse =
        api.importMetadata(ImportMetadataRequest(metadata, photoId, blobId))

    suspend fun importBatch(entries: List<ImportMetadataBatchEntry>): ImportMetadataBatchResponse =
        api.importMetadataBatch(ImportMetadataBatchRequest(entries))

    suspend fun uploadSidecar(
        bytes: ByteArray,
        photoId: String? = null,
        blobId: String? = null,
    ): ImportMetadataResponse {
        val body: RequestBody = bytes.toRequestBody("application/json".toMediaType())
        return api.importMetadataUpload(body, photoId, blobId)
    }

    suspend fun list(photoId: String): List<PhotoMetadataRecord> =
        api.listPhotoMetadata(photoId).metadata

    suspend fun delete(photoId: String) {
        api.deletePhotoMetadata(photoId)
    }
}
