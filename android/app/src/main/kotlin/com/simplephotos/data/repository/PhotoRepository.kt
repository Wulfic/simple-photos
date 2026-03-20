/**
 * Repository for photo metadata operations including fetch, upload, favorites,
 * crop, duplicate detection, and edit-copy management.
 */
package com.simplephotos.data.repository

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import com.simplephotos.crypto.CryptoManager
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.DuplicatePhotoRequest
import com.simplephotos.data.remote.dto.DuplicatePhotoResponse
import com.simplephotos.data.remote.dto.FavoriteToggleResponse
import com.simplephotos.data.remote.dto.SetCropRequest
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.time.Instant
import java.time.LocalDateTime
import java.time.ZoneOffset
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Central photo/video management: upload (encrypted), download, decrypt,
 * sync from server, and local cache management.
 *
 * Media is wrapped in a JSON envelope (`{v, filename, data, ...}`) and
 * encrypted with AES-256-GCM before upload. The repository also handles
 * content-hash dedup.
 */
@Singleton
class PhotoRepository @Inject constructor(
    private val api: ApiService,
    private val db: AppDatabase,
    private val crypto: CryptoManager,
    private val dataStore: DataStore<Preferences>,
    @ApplicationContext private val context: Context
) {
    // ── Helpers ───────────────────────────────────────────────────────────

    /**
     * Parse an ISO-8601 timestamp string to epoch millis.
     * Handles both timezone-aware ("2026-02-22T17:12:42+00:00", "...Z")
     * and naive ("2008-05-24T23:39:22") formats. Naive timestamps are
     * treated as UTC to match the server's SQLite text sort order.
     */
    private fun parseIsoToEpochMs(iso: String): Long {
        // Try timezone-aware first (Instant handles +00:00 and Z)
        return try {
            val result = Instant.parse(iso).toEpochMilli()
            android.util.Log.d("PhotoRepository", "parseIso: '$iso' → $result (Instant.parse)")
            result
        } catch (_: Exception) {
            // Fallback: naive timestamp — treat as UTC
            try {
                val result = LocalDateTime.parse(iso).toInstant(ZoneOffset.UTC).toEpochMilli()
                android.util.Log.d("PhotoRepository", "parseIso: '$iso' → $result (LocalDateTime fallback)")
                result
            } catch (_: Exception) {
                val result = System.currentTimeMillis()
                android.util.Log.w("PhotoRepository", "parseIso: '$iso' → $result (FAILED — using currentTimeMillis)")
                result
            }
        }
    }

    private val thumbnailDir: File
        get() = File(context.filesDir, "thumbnails").also { it.mkdirs() }

    /** Short content-based hash: first 12 hex chars of SHA-256.
     *  Deterministic fingerprint for cross-platform alignment. */
    private fun computeContentHash(data: ByteArray): String =
        crypto.sha256Hex(data).take(12)

    fun getAllPhotos(): Flow<List<PhotoEntity>> = db.photoDao().getAllPhotos()

    suspend fun getPhoto(id: String): PhotoEntity? = db.photoDao().getById(id)

    suspend fun insertPhoto(photo: PhotoEntity) = db.photoDao().insert(photo)

    /**
     * Get the base URL for building image URLs (for Coil).
     */
    suspend fun getServerBaseUrl(): String {
        val prefs = dataStore.data.first()
        return (prefs[KEY_SERVER_URL] ?: "http://localhost:8080").trimEnd('/')
    }

    /**
     * Fetch conversion pipeline status from the server.
     * Returns null if the server is unreachable.
     */
    suspend fun getConversionStatus(): com.simplephotos.data.remote.dto.ConversionStatusResponse? {
        return try { api.getConversionStatus() } catch (_: Exception) { null }
    }

    suspend fun deletePhoto(photo: PhotoEntity) {
        // Delete encrypted blobs from server
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

    // ── Encrypted upload ─────────────────────────────────────────────────

    /**
     * Upload a photo/GIF/video with its thumbnail (encrypted mode).
     */
    suspend fun uploadPhoto(photo: PhotoEntity, photoData: ByteArray, thumbnailData: ByteArray?) {
        db.photoDao().updateSyncStatus(photo.localId, SyncStatus.UPLOADING)

        try {
            val thumbBlobType = if (photo.mediaType == "video") "video_thumbnail" else "thumbnail"
            val mediaBlobType = when (photo.mediaType) {
                "gif" -> "gif"
                "video" -> "video"
                else -> "photo"
            }

            var thumbBlobId: String? = null

            if (thumbnailData != null && thumbnailData.isNotEmpty()) {
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
                thumbBlobId = thumbRes.blobId
            } else {
                android.util.Log.d("PhotoRepository", "uploadPhoto: no thumbnail data for ${photo.filename}, skipping thumbnail upload")
            }

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
                if (thumbBlobId != null) put("thumbnail_blob_id", thumbBlobId)
                put("data", android.util.Base64.encodeToString(photoData, android.util.Base64.NO_WRAP))
            }.toString()

            val encryptedPhoto = crypto.encrypt(mediaPayload.toByteArray())
            val photoHash = crypto.sha256Hex(encryptedPhoto)
            // Content hash: short hash of original raw bytes for cross-platform alignment
            val contentHash = computeContentHash(photoData)
            val photoBody = encryptedPhoto.toRequestBody("application/octet-stream".toMediaType())

            android.util.Log.d("PhotoRepository", "uploadPhoto: uploading media blob (${encryptedPhoto.size} bytes, type=$mediaBlobType, contentHash=$contentHash)")
            val photoRes = api.uploadBlob(photoBody, mediaBlobType, encryptedPhoto.size.toString(), photoHash, contentHash)
            android.util.Log.d("PhotoRepository", "uploadPhoto: media uploaded, blobId=${photoRes.blobId}")

            db.photoDao().markSynced(photo.localId, photoRes.blobId, thumbBlobId)

            // Cache uploaded thumbnail locally
            if (thumbnailData != null && thumbnailData.isNotEmpty()) {
                saveThumbnailToDisk(photo.localId, thumbnailData)
            }
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
     * Download and decrypt a **thumbnail** blob via `GET /api/blobs/{id}/thumb`.
     *
     * The server resolves the photo blob ID → encrypted_thumb_blob_id internally,
     * so the caller only needs the photo's main blob ID. Returns the decrypted
     * thumbnail bytes (typically small, ~30 KB JPEG).
     *
     * Falls back to `null` if the server returns 404 (no thumbnail available).
     */
    suspend fun downloadAndDecryptThumbBlob(blobId: String): ByteArray? {
        return try {
            val response = api.downloadThumbBlob(blobId)
            val encrypted = response.bytes()
            crypto.decrypt(encrypted)
        } catch (e: retrofit2.HttpException) {
            if (e.code() == 404) null else throw e
        }
    }

    /**
     * Download an encrypted blob, decrypt it, extract the base64-encoded
     * "data" field from the JSON envelope, and write the decoded bytes
     * directly to [outputFile].
     *
     * Memory profile (for a 50 MB video):
     *   1. Stream download to temp file on disk          → ~8 KB heap
     *   2. Read temp file + AES-GCM decrypt              → ~encrypted-size heap (unavoidable for GCM auth)
     *   3. Stream-scan decrypted bytes for "data" field  → ~0 extra heap
     *   4. Base64-decode in 48 KB chunks → write to file → ~48 KB heap
     *   Total peak: ~1× blob size (vs ~4× before)
     *
     * This is used for video/audio where the raw bytes are large.
     * Photos still use [downloadAndDecryptBlob] since Coil needs a ByteArray.
     */
    suspend fun downloadAndDecryptBlobToFile(blobId: String, outputFile: File) {
        // Step 1: Stream the encrypted blob to a temp file (near-zero heap)
        val encryptedTempFile = File.createTempFile("enc_", ".tmp", context.cacheDir)
        try {
            val response = api.downloadBlob(blobId)
            response.byteStream().use { input ->
                encryptedTempFile.outputStream().buffered().use { output ->
                    input.copyTo(output, bufferSize = 8192)
                }
            }

            // Step 2: Read encrypted file and decrypt (one allocation for GCM)
            val encrypted = encryptedTempFile.readBytes()
            // Delete the encrypted temp file immediately to free disk space
            encryptedTempFile.delete()
            val decrypted = crypto.decrypt(encrypted)
            // encrypted is now GC-eligible

            // Step 3: Extract the base64 "data" field and decode to output file.
            // We scan for `"data":"` then read until the closing `"`, decoding
            // base64 in chunks to avoid holding the full decoded bytes in memory.
            streamExtractBase64ToFile(decrypted, outputFile)
            // decrypted is now GC-eligible
        } finally {
            // Ensure cleanup even on error
            if (encryptedTempFile.exists()) encryptedTempFile.delete()
        }
    }

    /**
     * Scan the decrypted JSON bytes for the `"data":"..."` field and
     * stream-decode the base64 content directly to [outputFile].
     *
     * This avoids creating a full String copy of the JSON envelope and
     * avoids holding the full base64 string + decoded bytes simultaneously.
     */
    private fun streamExtractBase64ToFile(decrypted: ByteArray, outputFile: File) {
        // Find the "data" field start: look for `"data":"` pattern
        val marker = "\"data\":\"".toByteArray(Charsets.UTF_8)
        var markerIdx = -1
        outer@ for (i in 0..decrypted.size - marker.size) {
            for (j in marker.indices) {
                if (decrypted[i + j] != marker[j]) continue@outer
            }
            markerIdx = i + marker.size
            break
        }
        if (markerIdx < 0) {
            // Fallback: try with a space after colon  `"data": "`
            val markerAlt = "\"data\": \"".toByteArray(Charsets.UTF_8)
            outer2@ for (i in 0..decrypted.size - markerAlt.size) {
                for (j in markerAlt.indices) {
                    if (decrypted[i + j] != markerAlt[j]) continue@outer2
                }
                markerIdx = i + markerAlt.size
                break
            }
        }
        if (markerIdx < 0) {
            throw IllegalStateException("Could not find \"data\" field in decrypted blob")
        }

        // Find the closing quote for the data field
        var endIdx = markerIdx
        while (endIdx < decrypted.size && decrypted[endIdx] != '"'.code.toByte()) {
            endIdx++
        }
        if (endIdx >= decrypted.size) {
            throw IllegalStateException("Could not find end of \"data\" field in decrypted blob")
        }

        // Decode base64 in chunks and write to output file.
        // Base64 decodes in groups of 4 chars → 3 bytes, so we process
        // in multiples of 4 characters (e.g. 49152 chars → 36864 bytes).
        outputFile.outputStream().buffered().use { out ->
            val chunkSize = 49152 // must be multiple of 4
            var pos = markerIdx
            while (pos < endIdx) {
                val end = minOf(pos + chunkSize, endIdx)
                val base64Chunk = String(decrypted, pos, end - pos, Charsets.UTF_8)
                val decoded = android.util.Base64.decode(base64Chunk, android.util.Base64.NO_WRAP)
                out.write(decoded)
                pos = end
            }
        }
    }

    // ── Sync from server ─────────────────────────────────────────────────

    /** Pull photos from the server (always encrypted). */
    suspend fun syncFromServer(): Int = syncFromServerEncrypted()

    /**
     * Pull encrypted-mode photos from the server using the lightweight sync
     * manifest. This avoids downloading and decrypting every full-size photo
     * blob — it reads photo metadata directly from the server's photos table
     * and then downloads only the small thumbnail blobs (~30 KB each).
     */
    private suspend fun syncFromServerEncrypted(): Int {
        var imported = 0
        var after: String? = null
        android.util.Log.i("PhotoRepository", "syncFromServerEncrypted: starting sync")

        do {
            val result = api.encryptedSync(after = after, limit = 500)
            android.util.Log.d("PhotoRepository", "syncFromServerEncrypted: fetched page with ${result.photos.size} photos, nextCursor=${result.nextCursor}")

            for (photo in result.photos) {
                val blobId = photo.encryptedBlobId ?: continue

                // Skip if already in local DB
                if (db.photoDao().getByServerBlobId(blobId) != null) continue

                val localId = java.util.UUID.randomUUID().toString()
                val serverTimestamp = photo.takenAt ?: photo.createdAt
                val takenAtMs = parseIsoToEpochMs(serverTimestamp)
                android.util.Log.d("PhotoRepository", "syncFromServerEncrypted: importing '${photo.filename}' serverTs='$serverTimestamp' → takenAtMs=$takenAtMs blobId=$blobId")

                // Download and decrypt thumbnail blob if available
                var thumbPath: String? = null
                val thumbBlobId = photo.encryptedThumbBlobId
                if (!thumbBlobId.isNullOrEmpty()) {
                    try {
                        val thumbDecrypted = downloadAndDecryptBlob(thumbBlobId)
                        val thumbPayload = JSONObject(String(thumbDecrypted, Charsets.UTF_8))
                        val thumbBase64 = thumbPayload.optString("data", "")
                        if (thumbBase64.isNotEmpty()) {
                            val thumbBytes = android.util.Base64.decode(thumbBase64, android.util.Base64.NO_WRAP)
                            thumbPath = saveThumbnailToDisk(localId, thumbBytes)
                        }
                    } catch (e: Exception) {
                        android.util.Log.w("PhotoRepository", "Thumbnail download failed for blob $blobId: ${e.message}")
                    }
                }

                val entity = PhotoEntity(
                    localId = localId,
                    serverBlobId = blobId,
                    thumbnailBlobId = thumbBlobId,
                    filename = photo.filename,
                    takenAt = takenAtMs,
                    mimeType = photo.mimeType,
                    mediaType = photo.mediaType,
                    width = photo.width.toInt(),
                    height = photo.height.toInt(),
                    durationSecs = photo.durationSecs?.toFloat(),
                    sizeBytes = photo.sizeBytes,
                    localPath = null,
                    thumbnailPath = thumbPath,
                    syncStatus = SyncStatus.SYNCED,
                    isFavorite = photo.isFavorite,
                    cropMetadata = photo.cropMetadata,
                    photoHash = photo.photoHash,
                    serverPhotoId = photo.id
                )
                db.photoDao().insert(entity)
                imported++
            }

            after = result.nextCursor
        } while (result.nextCursor != null)

        android.util.Log.i("PhotoRepository", "syncFromServerEncrypted: finished — imported $imported photos")
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
     * Update crop metadata for a photo in the local database.
     */
    suspend fun updateCropMetadata(localId: String, cropMetadata: String?) {
        db.photoDao().updateCropMetadata(localId, cropMetadata)
    }

    /**
     * Copy a local media file to a new path, handling both `content://` URIs
     * (from the Android Media Store) and regular filesystem paths.
     *
     * Returns the absolute path to the new file, or `null` if the copy failed.
     */
    fun copyLocalFile(sourcePath: String, destFile: File): String? {
        return try {
            val inputStream = if (sourcePath.startsWith("content://")) {
                context.contentResolver.openInputStream(android.net.Uri.parse(sourcePath))
            } else {
                val sourceFile = File(sourcePath)
                if (sourceFile.exists()) sourceFile.inputStream() else null
            }

            inputStream?.use { input ->
                destFile.outputStream().use { output ->
                    input.copyTo(output)
                }
                destFile.absolutePath
            }
        } catch (e: Exception) {
            android.util.Log.w("PhotoRepository", "Failed to copy local file: ${e.message}")
            null
        }
    }

    /** App-internal cache directory for temporary file copies. */
    fun getCacheDir(): File = context.cacheDir

    /**
     * Update only the thumbnail path for an existing photo entity in the local DB.
     */
    suspend fun updateThumbnailPath(localId: String, thumbnailPath: String) {
        db.photoDao().updateThumbnailPath(localId, thumbnailPath)
    }

    // ── Server-side metadata operations (used by PhotoViewerViewModel) ────

    /** Toggle the is_favorite flag on the server and return the new state. */
    suspend fun toggleFavorite(photoId: String): FavoriteToggleResponse =
        api.toggleFavorite(photoId)

    /** Persist crop/brightness/trim metadata on the server. */
    suspend fun setCropOnServer(photoId: String, cropMetadata: String?) {
        api.setCrop(photoId, SetCropRequest(cropMetadata))
    }

    /** Create a server-side duplicate of a photo ("Save Copy"). */
    suspend fun duplicatePhotoOnServer(
        photoId: String,
        cropMetadata: String?
    ): DuplicatePhotoResponse =
        api.duplicatePhoto(photoId, DuplicatePhotoRequest(cropMetadata))

    // ── Diagnostic helpers (used by GalleryViewModel) ────────────────────

    /** Count photos in a specific sync status (PENDING, FAILED, etc.). */
    suspend fun getPhotoCountByStatus(status: SyncStatus): Int =
        db.photoDao().getByStatus(status).size

    /** Look up a synced photo by its content hash (for import dedup). */
    suspend fun getSyncedByHash(hash: String): PhotoEntity? =
        db.photoDao().getSyncedByHash(hash)
}
