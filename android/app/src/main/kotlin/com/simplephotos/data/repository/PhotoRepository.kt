package com.simplephotos.data.repository

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import com.simplephotos.crypto.CryptoManager
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.PlainPhotoRecord
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
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
    private val dataStore: DataStore<Preferences>,
    @ApplicationContext private val context: Context
) {
    private val thumbnailDir: File
        get() = File(context.filesDir, "thumbnails").also { it.mkdirs() }

    /** Cached encryption mode — refreshed on each sync. */
    private var cachedEncryptionMode: String? = null

    fun getAllPhotos(): Flow<List<PhotoEntity>> = db.photoDao().getAllPhotos()

    suspend fun getPhoto(id: String): PhotoEntity? = db.photoDao().getById(id)

    suspend fun insertPhoto(photo: PhotoEntity) = db.photoDao().insert(photo)

    /**
     * Get the encryption mode from server (cached per session).
     */
    suspend fun getEncryptionMode(): String {
        cachedEncryptionMode?.let { return it }
        return try {
            val settings = api.getEncryptionSettings()
            cachedEncryptionMode = settings.encryptionMode
            settings.encryptionMode
        } catch (_: Exception) {
            "plain" // default to plain if we can't reach the server
        }
    }

    /**
     * Get the base URL for building image URLs (for Coil).
     */
    suspend fun getServerBaseUrl(): String {
        val prefs = dataStore.data.first()
        return (prefs[KEY_SERVER_URL] ?: "http://localhost:8080").trimEnd('/')
    }

    suspend fun deletePhoto(photo: PhotoEntity) {
        // Delete from server based on mode
        val mode = getEncryptionMode()
        if (mode == "plain") {
            photo.serverPhotoId?.let { photoId ->
                try { api.deletePhoto(photoId) } catch (_: Exception) {}
            }
        } else {
            photo.serverBlobId?.let { blobId ->
                try { api.deleteBlob(blobId) } catch (_: Exception) {}
            }
            photo.thumbnailBlobId?.let { blobId ->
                try { api.deleteBlob(blobId) } catch (_: Exception) {}
            }
        }
        // Delete cached thumbnail
        photo.thumbnailPath?.let { File(it).delete() }
        db.photoDao().delete(photo)
    }

    // ── Plain-mode upload ────────────────────────────────────────────────

    /**
     * Upload a photo in plain mode — send raw file bytes to the server.
     */
    suspend fun uploadPhotoPlain(photo: PhotoEntity, photoData: ByteArray) {
        db.photoDao().updateSyncStatus(photo.localId, SyncStatus.UPLOADING)
        try {
            val body = photoData.toRequestBody("application/octet-stream".toMediaType())
            android.util.Log.d("PhotoRepository", "uploadPhotoPlain: sending ${photoData.size} bytes, filename=${photo.filename}, mime=${photo.mimeType}")
            val result = api.uploadPhoto(body, photo.filename, photo.mimeType)
            android.util.Log.d("PhotoRepository", "uploadPhotoPlain: server returned photoId=${result.photoId}, sizeBytes=${result.sizeBytes}")

            db.photoDao().markSyncedPlain(photo.localId, result.photoId)
        } catch (e: retrofit2.HttpException) {
            val errorBody = e.response()?.errorBody()?.string()
            android.util.Log.e("PhotoRepository", "uploadPhotoPlain HTTP ${e.code()}: $errorBody", e)
            db.photoDao().updateSyncStatus(photo.localId, SyncStatus.FAILED)
            throw e
        } catch (e: Exception) {
            android.util.Log.e("PhotoRepository", "uploadPhotoPlain failed: ${e.message}", e)
            db.photoDao().updateSyncStatus(photo.localId, SyncStatus.FAILED)
            throw e
        }
    }

    // ── Encrypted-mode upload ────────────────────────────────────────────

    /**
     * Upload a photo/GIF/video with its thumbnail (encrypted mode).
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

            android.util.Log.d("PhotoRepository", "uploadPhoto: encrypting thumbnail for ${photo.filename}")

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

            android.util.Log.d("PhotoRepository", "uploadPhoto: uploading thumbnail blob (${encryptedThumb.size} bytes, type=$thumbBlobType)")
            val thumbRes = api.uploadBlob(thumbBody, thumbBlobType, encryptedThumb.size.toString(), thumbHash)
            android.util.Log.d("PhotoRepository", "uploadPhoto: thumbnail uploaded, blobId=${thumbRes.blobId}")

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

            android.util.Log.d("PhotoRepository", "uploadPhoto: uploading media blob (${encryptedPhoto.size} bytes, type=$mediaBlobType)")
            val photoRes = api.uploadBlob(photoBody, mediaBlobType, encryptedPhoto.size.toString(), photoHash)
            android.util.Log.d("PhotoRepository", "uploadPhoto: media uploaded, blobId=${photoRes.blobId}")

            db.photoDao().markSynced(photo.localId, photoRes.blobId, thumbRes.blobId)

            // Cache uploaded thumbnail locally
            saveThumbnailToDisk(photo.localId, thumbnailData)
        } catch (e: retrofit2.HttpException) {
            val errorBody = e.response()?.errorBody()?.string()
            android.util.Log.e("PhotoRepository", "uploadPhoto HTTP ${e.code()}: $errorBody", e)
            db.photoDao().updateSyncStatus(photo.localId, SyncStatus.FAILED)
            throw e
        } catch (e: Exception) {
            android.util.Log.e("PhotoRepository", "uploadPhoto failed: ${e.message}", e)
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
     * Collect all filenames currently known on the server (plain mode).
     * Used by BackupWorker to skip uploading photos that already exist remotely.
     */
    suspend fun getServerFilenames(): Set<String> {
        val filenames = mutableSetOf<String>()
        var after: String? = null
        try {
            do {
                val result = api.listPhotos(after = after, limit = 500)
                for (p in result.photos) filenames.add(p.filename)
                after = result.nextCursor
            } while (after != null)
        } catch (_: Exception) { /* network error — return what we have */ }
        return filenames
    }

    // ── Sync from server ─────────────────────────────────────────────────

    /**
     * Pull photos from the server.
     * Checks encryption mode and uses the appropriate API.
     */
    suspend fun syncFromServer(): Int {
        val mode = getEncryptionMode()
        return if (mode == "encrypted") {
            syncFromServerEncrypted()
        } else {
            syncFromServerPlain()
        }
    }

    /**
     * Sync plain-mode photos from the server.
     * Fetches photo metadata and stores references in local DB.
     * Actual image data is loaded on-demand via authenticated URLs.
     *
     * Cross-matches by filename: if a local PENDING photo already exists
     * on the server, we link it (set serverPhotoId, mark SYNCED) instead
     * of creating a duplicate entry.
     */
    private suspend fun syncFromServerPlain(): Int {
        var imported = 0
        var after: String? = null

        do {
            val listResult = api.listPhotos(after = after, limit = 100)

            for (photo in listResult.photos) {
                // Skip if we already have this server photo ID tracked
                if (db.photoDao().getByServerPhotoId(photo.id) != null) continue

                // Skip server-side duplicates: if the filename looks like a
                // rename suffix (e.g. "IMG_001_1.jpg") and we already have
                // the base file tracked, don't create a second Room entry.
                val baseFilename = stripRenameSuffix(photo.filename)
                if (baseFilename != photo.filename) {
                    // This is a renamed duplicate — skip it entirely
                    continue
                }

                // Cross-match: check if we have a local photo with the same
                // filename that hasn't been uploaded yet. If so, link it to
                // this server record instead of inserting a new row.
                val existingLocal = db.photoDao().getSyncedByFilename(photo.filename)
                    ?: run {
                        // Also check PENDING/FAILED local photos by filename
                        val pending = db.photoDao().getByStatus(SyncStatus.PENDING) +
                                      db.photoDao().getByStatus(SyncStatus.FAILED)
                        pending.firstOrNull { it.filename == photo.filename }
                    }

                if (existingLocal != null && existingLocal.serverPhotoId == null) {
                    // Link existing local entry to the server record
                    db.photoDao().markSyncedPlain(existingLocal.localId, photo.id)
                    imported++
                    continue
                }

                val localId = java.util.UUID.randomUUID().toString()
                val takenAtMs = try {
                    photo.takenAt?.let { java.time.Instant.parse(it).toEpochMilli() }
                        ?: System.currentTimeMillis()
                } catch (_: Exception) {
                    System.currentTimeMillis()
                }

                val entity = PhotoEntity(
                    localId = localId,
                    serverPhotoId = photo.id,
                    filename = photo.filename,
                    takenAt = takenAtMs,
                    mimeType = photo.mimeType,
                    mediaType = photo.mediaType,
                    width = photo.width.toInt(),
                    height = photo.height.toInt(),
                    durationSecs = photo.durationSecs?.toFloat(),
                    latitude = photo.latitude,
                    longitude = photo.longitude,
                    sizeBytes = photo.sizeBytes,
                    syncStatus = SyncStatus.SYNCED
                )
                db.photoDao().insert(entity)
                imported++
            }

            after = listResult.nextCursor
        } while (after != null)

        return imported
    }

    /**
     * Pull encrypted-mode photos from the server.
     */
    private suspend fun syncFromServerEncrypted(): Int {
        var imported = 0
        var after: String? = null

        val blobTypes = listOf("photo", "gif", "video")

        for (blobType in blobTypes) {
            after = null
            do {
                val listResult = api.listBlobs(blobType = blobType, after = after, limit = 200)

                for (blob in listResult.blobs) {
                    if (db.photoDao().getByServerBlobId(blob.id) != null) continue

                    try {
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
                            } catch (_: Exception) {}
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
                            localPath = null,
                            thumbnailPath = thumbPath,
                            syncStatus = SyncStatus.SYNCED,
                            encryptedBlobSize = null
                        )
                        db.photoDao().insert(photo)
                        imported++
                    } catch (e: Exception) {
                        continue
                    }
                }

                after = if (listResult.blobs.isNotEmpty()) listResult.blobs.last().id else null
            } while (listResult.blobs.size == 200)
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

    /**
     * Strip server-side rename suffixes (e.g. "IMG_001_1.jpg" → "IMG_001.jpg").
     * The server appends `_N` before the extension when a filename collision
     * occurs on disk. We detect this pattern so the client can skip importing
     * these server-created duplicates.
     */
    private fun stripRenameSuffix(filename: String): String {
        val dot = filename.lastIndexOf('.')
        if (dot <= 0) return filename
        val stem = filename.substring(0, dot)
        val ext = filename.substring(dot)
        val match = Regex("^(.+)_\\d+$").find(stem) ?: return filename
        return match.groupValues[1] + ext
    }
}
