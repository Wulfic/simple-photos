package com.simplephotos.data.repository

import android.content.Context
import com.simplephotos.crypto.CryptoManager
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class PhotoRepository @Inject constructor(
    private val api: ApiService,
    private val db: AppDatabase,
    private val crypto: CryptoManager,
    @ApplicationContext private val context: Context
) {
    private val thumbnailDir: File
        get() = File(context.filesDir, "thumbnails").also { it.mkdirs() }

    fun getAllPhotos(): Flow<List<PhotoEntity>> = db.photoDao().getAllPhotos()

    suspend fun getPhoto(id: String): PhotoEntity? = db.photoDao().getById(id)

    suspend fun insertPhoto(photo: PhotoEntity) = db.photoDao().insert(photo)

    suspend fun deletePhoto(photo: PhotoEntity) {
        // Delete from server if synced
        photo.serverBlobId?.let { blobId ->
            try { api.deleteBlob(blobId) } catch (_: Exception) {}
        }
        photo.thumbnailBlobId?.let { blobId ->
            try { api.deleteBlob(blobId) } catch (_: Exception) {}
        }
        // Delete cached thumbnail
        photo.thumbnailPath?.let { File(it).delete() }
        db.photoDao().delete(photo)
    }

    /**
     * Upload a photo/GIF/video with its thumbnail.
     */
    suspend fun uploadPhoto(photo: PhotoEntity, photoData: ByteArray, thumbnailData: ByteArray) {
        db.photoDao().updateSyncStatus(photo.localId, SyncStatus.UPLOADING)

        try {
            val thumbBlobType = if (photo.mediaType == "video") "video_thumbnail" else "thumbnail"
            val mediaBlobType = when (photo.mediaType) {
                "gif" -> "gif"
                "video" -> "video"
                else -> "photo"
            }

            // Build & encrypt thumbnail payload
            val thumbPayload = JSONObject().apply {
                put("v", 1)
                put("photo_blob_id", "")
                put("width", 256)
                put("height", 256)
                put("data", android.util.Base64.encodeToString(thumbnailData, android.util.Base64.NO_WRAP))
            }.toString()

            val encryptedThumb = crypto.encrypt(thumbPayload.toByteArray())
            val thumbHash = crypto.sha256Hex(encryptedThumb)
            val thumbBody = encryptedThumb.toRequestBody("application/octet-stream".toMediaType())
            val thumbRes = api.uploadBlob(thumbBody, thumbBlobType, encryptedThumb.size.toString(), thumbHash)

            // Build & encrypt media payload
            val mediaPayload = JSONObject().apply {
                put("v", 1)
                put("filename", photo.filename)
                put("taken_at", java.time.Instant.ofEpochMilli(photo.takenAt).toString())
                put("mime_type", photo.mimeType)
                put("media_type", photo.mediaType)
                put("width", photo.width)
                put("height", photo.height)
                if (photo.durationSecs != null) put("duration", photo.durationSecs.toDouble())
                if (photo.latitude != null) put("latitude", photo.latitude)
                if (photo.longitude != null) put("longitude", photo.longitude)
                put("album_ids", JSONArray())
                put("thumbnail_blob_id", thumbRes.blobId)
                put("data", android.util.Base64.encodeToString(photoData, android.util.Base64.NO_WRAP))
            }.toString()

            val encryptedPhoto = crypto.encrypt(mediaPayload.toByteArray())
            val photoHash = crypto.sha256Hex(encryptedPhoto)
            val photoBody = encryptedPhoto.toRequestBody("application/octet-stream".toMediaType())
            val photoRes = api.uploadBlob(photoBody, mediaBlobType, encryptedPhoto.size.toString(), photoHash)

            db.photoDao().markSynced(photo.localId, photoRes.blobId, thumbRes.blobId)

            // Cache uploaded thumbnail locally
            saveThumbnailToDisk(photo.localId, thumbnailData)
        } catch (e: Exception) {
            db.photoDao().updateSyncStatus(photo.localId, SyncStatus.FAILED)
            throw e
        }
    }

    /**
     * Download and decrypt a blob from the server.
     */
    suspend fun downloadAndDecryptBlob(blobId: String): ByteArray {
        val response = api.downloadBlob(blobId)
        val encrypted = response.bytes()
        return crypto.decrypt(encrypted)
    }

    /**
     * Pull photos from the server that don't exist locally yet.
     * Fetches blob metadata, downloads & decrypts photo/thumbnail payloads,
     * and inserts into the local DB.
     */
    suspend fun syncFromServer(): Int {
        var imported = 0
        var after: String? = null

        // Fetch all photo-type blobs from server with pagination
        val blobTypes = listOf("photo", "gif", "video")

        for (blobType in blobTypes) {
            after = null
            do {
                val listResult = api.listBlobs(blobType = blobType, after = after, limit = 50)

                for (blob in listResult.blobs) {
                    // Skip if we already have this blob locally
                    if (db.photoDao().getByServerBlobId(blob.id) != null) continue

                    try {
                        // Download and decrypt the media payload
                        val decrypted = downloadAndDecryptBlob(blob.id)
                        val payload = JSONObject(String(decrypted, Charsets.UTF_8))

                        val thumbnailBlobId = payload.optString("thumbnail_blob_id", "")
                        val filename = payload.optString("filename", "unknown")
                        val takenAt = payload.optString("taken_at", "")
                        val mimeType = payload.optString("mime_type", "image/jpeg")
                        val mediaType = payload.optString("media_type", "photo")
                        val width = payload.optInt("width", 0)
                        val height = payload.optInt("height", 0)
                        val duration = if (payload.has("duration")) payload.getDouble("duration").toFloat() else null
                        val lat = if (payload.has("latitude")) payload.getDouble("latitude") else null
                        val lng = if (payload.has("longitude")) payload.getDouble("longitude") else null

                        val takenAtMs = try {
                            java.time.Instant.parse(takenAt).toEpochMilli()
                        } catch (_: Exception) {
                            System.currentTimeMillis()
                        }

                        val localId = java.util.UUID.randomUUID().toString()

                        // Download & cache thumbnail
                        var thumbPath: String? = null
                        if (thumbnailBlobId.isNotEmpty()) {
                            try {
                                val thumbDecrypted = downloadAndDecryptBlob(thumbnailBlobId)
                                val thumbPayload = JSONObject(String(thumbDecrypted, Charsets.UTF_8))
                                val thumbBase64 = thumbPayload.optString("data", "")
                                if (thumbBase64.isNotEmpty()) {
                                    val thumbBytes = android.util.Base64.decode(thumbBase64, android.util.Base64.NO_WRAP)
                                    thumbPath = saveThumbnailToDisk(localId, thumbBytes)
                                }
                            } catch (_: Exception) {
                                // Thumbnail download failed — photo still importable without it
                            }
                        }

                        val photo = PhotoEntity(
                            localId = localId,
                            serverBlobId = blob.id,
                            thumbnailBlobId = thumbnailBlobId.ifEmpty { null },
                            filename = filename,
                            takenAt = takenAtMs,
                            mimeType = mimeType,
                            mediaType = mediaType,
                            width = width,
                            height = height,
                            durationSecs = duration,
                            latitude = lat,
                            longitude = lng,
                            localPath = null, // server-only photo
                            thumbnailPath = thumbPath,
                            syncStatus = SyncStatus.SYNCED,
                            encryptedBlobSize = null
                        )
                        db.photoDao().insert(photo)
                        imported++
                    } catch (e: Exception) {
                        // Skip individual failures — continue with next blob
                        continue
                    }
                }

                after = if (listResult.blobs.isNotEmpty()) listResult.blobs.last().id else null
            } while (listResult.blobs.size == 50) // Continue if we got a full page
        }

        return imported
    }

    /**
     * Save thumbnail JPEG bytes to app-internal storage.
     * Returns the absolute path to the saved file.
     */
    fun saveThumbnailToDisk(photoLocalId: String, thumbnailBytes: ByteArray): String {
        val file = File(thumbnailDir, "$photoLocalId.jpg")
        file.writeBytes(thumbnailBytes)
        return file.absolutePath
    }
}
