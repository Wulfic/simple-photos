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
import java.time.Instant
import java.time.LocalDateTime
import java.time.ZoneOffset
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Central photo/video management: upload (plain and encrypted modes), download,
 * decrypt, sync from server, and local cache management.
 *
 * In encrypted mode, media is wrapped in a JSON envelope (`{v, filename, data,
 * ...}`) and encrypted with AES-256-GCM before upload. In plain mode, raw
 * bytes are sent directly. The repository also handles content-hash dedup
 * and server-side filename cross-matching.
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

    /** Cached encryption mode — refreshed on each sync. */
    private var cachedEncryptionMode: String? = null

    /** Short content-based hash: first 12 hex chars of SHA-256.
     *  Deterministic fingerprint for cross-platform alignment. */
    private fun computeContentHash(data: ByteArray): String =
        crypto.sha256Hex(data).take(12)

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
     * Download a plain-mode photo/video file from the server and stream it
     * directly to [outputFile].  Uses `@Streaming` on the Retrofit call so
     * the response body is never buffered entirely in memory — safe for
     * multi-hundred-MB videos that would OOM with `response.bytes()`.
     */
    suspend fun downloadPlainPhotoToFile(photoId: String, outputFile: File) {
        val response = api.photoFile(photoId)
        response.byteStream().use { input ->
            outputFile.outputStream().buffered().use { output ->
                input.copyTo(output, bufferSize = 8192)
            }
        }
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
        android.util.Log.i("PhotoRepository", "syncFromServerPlain: starting sync")

        do {
            val listResult = api.listPhotos(after = after, limit = 100)
            android.util.Log.d("PhotoRepository", "syncFromServerPlain: fetched page with ${listResult.photos.size} photos, nextCursor=${listResult.nextCursor}")

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
                val serverTimestamp = photo.takenAt ?: photo.createdAt
                val takenAtMs = parseIsoToEpochMs(serverTimestamp)
                android.util.Log.d("PhotoRepository", "syncFromServerPlain: importing '${photo.filename}' serverTs='$serverTimestamp' → takenAtMs=$takenAtMs")

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
                    syncStatus = SyncStatus.SYNCED,
                    isFavorite = photo.isFavorite,
                    cropMetadata = photo.cropMetadata,
                    cameraModel = photo.cameraModel,
                    photoHash = photo.photoHash
                )
                db.photoDao().insert(entity)
                imported++
            }

            after = listResult.nextCursor
        } while (after != null)

        android.util.Log.i("PhotoRepository", "syncFromServerPlain: finished — imported $imported photos")
        return imported
    }

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
}
