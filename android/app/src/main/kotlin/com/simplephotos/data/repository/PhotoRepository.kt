/**
 * Repository for photo metadata operations including fetch, upload, favorites,
 * crop, duplicate detection, and edit-copy management.
 */
package com.simplephotos.data.repository

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import com.simplephotos.crypto.ChunkedBlob
import com.simplephotos.crypto.CryptoManager
import com.simplephotos.data.collapseBursts
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.DuplicatePhotoRequest
import com.simplephotos.data.remote.dto.DuplicatePhotoResponse
import com.simplephotos.data.remote.dto.FavoriteToggleResponse
import com.simplephotos.data.remote.dto.FullMetadataResponse
import com.simplephotos.data.remote.dto.MetadataUpdateRequest
import com.simplephotos.data.remote.dto.MetadataUpdateResponse
import com.simplephotos.data.remote.dto.SetCropRequest
import com.simplephotos.data.remote.dto.WriteExifResponse
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.asRequestBody
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
    companion object {
        private const val TAG = "PhotoRepository"
    }
    /** Expose the API service for banner polling in the gallery. */
    val apiService: ApiService get() = api

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

    /**
     * Read camera make/model from EXIF, formatted to match the server's
     * `metadata::extract_media_metadata` (model alone when it already starts
     * with the make, else "Make Model"). Used server-side to timestamp-group
     * bursts; returns null when no camera info is present.
     */
    private fun extractCameraModel(data: ByteArray): String? {
        return try {
            val exif = androidx.exifinterface.media.ExifInterface(data.inputStream())
            val make = exif.getAttribute(androidx.exifinterface.media.ExifInterface.TAG_MAKE)?.trim()
            val model = exif.getAttribute(androidx.exifinterface.media.ExifInterface.TAG_MODEL)?.trim()
            when {
                !make.isNullOrEmpty() && !model.isNullOrEmpty() ->
                    if (model.startsWith(make)) model else "$make $model"
                !model.isNullOrEmpty() -> model
                !make.isNullOrEmpty() -> make
                else -> null
            }
        } catch (e: Exception) {
            android.util.Log.w("PhotoRepository", "extractCameraModel failed: ${e.message}")
            null
        }
    }

    /** Bytes of a file head scanned for XMP/EXIF subtype + camera detection. */
    private val subtypeScanPrefixBytes = 256 * 1024

    /**
     * Read up to [maxBytes] from the head of [input] into a prefix buffer. Used by
     * the chunked upload path to classify subtype/camera from a large file's
     * header without ever buffering the whole file. A single `read` may return
     * fewer bytes than asked, so loop until the buffer is full or the stream ends.
     */
    private fun readPrefix(input: java.io.InputStream, maxBytes: Int): ByteArray {
        val buf = ByteArray(maxBytes)
        var off = 0
        while (off < maxBytes) {
            val n = input.read(buf, off, maxBytes - off)
            if (n < 0) break
            off += n
        }
        return if (off == maxBytes) buf else buf.copyOf(off)
    }

    fun getAllPhotos(): Flow<List<PhotoEntity>> = db.photoDao().getAllPhotos()

    suspend fun getPhoto(id: String): PhotoEntity? = db.photoDao().getById(id)

    /**
     * Resolve the ordered photo list for an album — handles BOTH virtual
     * "smart" albums (favorites/photos/gifs/videos/audio/recents) AND
     * user-created albums (photo↔album xref).
     *
     * Shared by the album-detail grid ([AlbumDetailViewModel]) and the
     * full-screen pager ([PhotoViewerViewModel]). They MUST use the same
     * source so the viewer pages in the exact order the grid shows; otherwise
     * the tapped index lands on the wrong photo. Smart albums were previously
     * only resolved by the grid, so opening one in the viewer found zero
     * photos → "Photo not found".
     */
    suspend fun getAlbumPhotos(albumId: String): List<PhotoEntity> {
        if (albumId.startsWith("smart-")) {
            val all = getAllPhotos().first()
            return when (albumId) {
                // Collapse bursts BEFORE capping so a 46-shot burst counts as
                // one item toward the 100-item recents window (matching grid).
                "smart-recents" -> all.sortedByDescending { it.createdAt }.collapseBursts().take(100)
                // Collapse bursts so a burst stack counts/renders as one item,
                // matching the main gallery and the web (which collapses bursts
                // everywhere it lists photos). collapseBursts is a no-op for
                // media without a burstId, so videos/gifs/audio are unaffected.
                "smart-favorites" -> all.filter { it.isFavorite }.collapseBursts()
                "smart-photos" -> all.filter { it.mediaType == "photo" || it.mediaType == "gif" }.collapseBursts()
                "smart-gifs" -> all.filter { it.mediaType == "gif" }
                "smart-videos" -> all.filter { it.mediaType == "video" }
                "smart-audio" -> all.filter { it.mediaType == "audio" }
                else -> all
            }
        }
        val photoIds = db.albumDao().getPhotoIdsForAlbum(albumId)
        return photoIds.mapNotNull { id -> getPhoto(id) }
    }

    suspend fun getPhotosByServerPhotoIds(ids: List<String>): List<PhotoEntity> =
        db.photoDao().getByServerPhotoIds(ids)

    /**
     * Expand a selection of photo localIds so that any burst-stack
     * representative also pulls in every other frame of its burst group.
     *
     * Burst stacks render as a single tile, so a multi-select only ever holds
     * the cover frame's localId. Adding that to an album would silently drop
     * the rest of the burst; this re-hydrates the full membership. Non-burst
     * ids pass through unchanged.
     */
    suspend fun expandBurstSelection(localIds: Collection<String>): List<String> {
        val ids = localIds.toList()
        if (ids.isEmpty()) return ids
        val burstIds = db.photoDao().getByIds(ids)
            .mapNotNull { it.burstId }
            .filter { it.isNotEmpty() }
            .distinct()
        if (burstIds.isEmpty()) return ids
        val members = db.photoDao().getByBurstIds(burstIds).map { it.localId }
        return (ids + members).distinct()
    }

    suspend fun insertPhoto(photo: PhotoEntity) = db.photoDao().insert(photo)

    /**
     * Get the base URL for building image URLs (for Coil).
     */
    suspend fun getServerBaseUrl(): String {
        val prefs = dataStore.data.first()
        return (prefs[KEY_SERVER_URL] ?: "http://localhost:8080").trimEnd('/')
    }

    suspend fun deletePhoto(photo: PhotoEntity) {
        android.util.Log.i(TAG, "deletePhoto: starting soft-delete for '${photo.filename}' " +
                "(localId=${photo.localId}, serverBlobId=${photo.serverBlobId}, " +
                "serverPhotoId=${photo.serverPhotoId}, thumbBlobId=${photo.thumbnailBlobId})")

        // ── Soft-delete on server (move to trash with 30-day recovery) ──
        val blobId = photo.serverBlobId
        if (blobId != null) {
            val takenAtIso = try {
                java.time.Instant.ofEpochMilli(photo.takenAt).toString()
            } catch (_: Exception) { null }

            val request = com.simplephotos.data.remote.dto.SoftDeleteBlobRequest(
                thumbnailBlobId = photo.thumbnailBlobId,
                filename = photo.filename,
                mimeType = photo.mimeType,
                mediaType = photo.mediaType,
                sizeBytes = photo.sizeBytes,
                width = photo.width,
                height = photo.height,
                durationSecs = photo.durationSecs?.toDouble(),
                takenAt = takenAtIso,
            )
            try {
                val resp = api.softDeleteBlob(blobId, request)
                android.util.Log.i(TAG, "deletePhoto: blob $blobId moved to trash " +
                        "(trashId=${resp.trashId}, expiresAt=${resp.expiresAt})")
            } catch (e: Exception) {
                // If the blob is already gone (404), continue with local cleanup.
                // Any other error should propagate so the user knows it failed.
                val is404 = e is retrofit2.HttpException && e.code() == 404
                if (is404) {
                    android.util.Log.w(TAG, "deletePhoto: blob $blobId not found on server (already deleted?), continuing local cleanup")
                } else {
                    android.util.Log.e(TAG, "deletePhoto: soft-delete failed for blob $blobId: ${e.message}")
                    throw e
                }
            }
        } else {
            android.util.Log.w(TAG, "deletePhoto: no serverBlobId — local-only item, skipping server call")
        }

        // ── Local cleanup ───────────────────────────────────────────────
        photo.thumbnailPath?.let { path ->
            val deleted = File(path).delete()
            android.util.Log.d(TAG, "deletePhoto: local thumbnail $path deleted=$deleted")
        }
        db.photoDao().delete(photo)
        android.util.Log.i(TAG, "deletePhoto: removed from local DB (localId=${photo.localId})")
    }

    // ── Encrypted upload ─────────────────────────────────────────────────

    /**
     * Upload a photo/GIF/video with its thumbnail (encrypted mode).
     */
    suspend fun uploadPhoto(photo: PhotoEntity, photoData: ByteArray, thumbnailData: ByteArray?) {
        db.photoDao().updateSyncStatus(photo.localId, SyncStatus.UPLOADING)

        try {
            val mediaBlobType = when (photo.mediaType) {
                "gif" -> "gif"
                "video" -> "video"
                else -> "photo"
            }

            val thumbBlobId = uploadThumbnailBlob(photo, thumbnailData)

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

            // ── Special-photo subtype detection (client-side) ────────────────
            // The server only ever sees the encrypted blob, so it cannot scan
            // XMP to classify motion/panorama/360/HDR/burst photos. Detect here
            // from the plaintext bytes and ship the result with registration.
            // camera_model lets the server timestamp-group bursts that carry no
            // XMP BurstID (Samsung et al.) via burst::detect_bursts_for_user.
            val detected = if (photo.mediaType == "photo") {
                com.simplephotos.data.media.MediaSubtypeDetector.detect(photoData, photo.width, photo.height)
            } else {
                com.simplephotos.data.media.MediaSubtype()
            }
            val cameraModel = if (photo.mediaType == "photo") extractCameraModel(photoData) else null
            if (detected.photoSubtype != null || detected.burstId != null || cameraModel != null) {
                android.util.Log.d("PhotoRepository",
                    "uploadPhoto: detected subtype=${detected.photoSubtype} burstId=${detected.burstId} camera=$cameraModel for ${photo.filename}")
            }

            // Register on the server + mark synced (shared with the chunked path).
            registerAndMark(
                photo = photo,
                photoBlobId = photoRes.blobId,
                thumbBlobId = thumbBlobId,
                contentHash = contentHash,
                detected = detected,
                cameraModel = cameraModel,
                thumbnailData = thumbnailData,
            )
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
     * Encrypt and upload a photo, GIF, or large video as a **v2 chunked** container with
     * bounded heap — the large-media counterpart to [uploadPhoto], which buffers
     * the whole file (base64 → JSON → AES-GCM, ~5× size) and OOMs the phone on big
     * videos. The media bytes are streamed straight from [openSource] to a temp
     * file via [ChunkedBlob.encryptStreamToFile]; only the small thumbnail and
     * metadata frame are ever held in memory.
     *
     * [dataLen] is the original source size (the caller already stat'd it to decide
     * the chunked path). [openSource] must yield a fresh stream on every call — it
     * is opened once to scan a bounded header prefix for subtype/camera and again
     * to stream the encrypt, so the whole file is never buffered.
     */
    suspend fun uploadPhotoChunked(
        photo: PhotoEntity,
        dataLen: Long,
        openSource: () -> java.io.InputStream,
        thumbnailData: ByteArray?,
    ) {
        db.photoDao().updateSyncStatus(photo.localId, SyncStatus.UPLOADING)

        val tmp = File.createTempFile("upload_", ".spchnk", context.cacheDir)
        try {
            val mediaBlobType = when (photo.mediaType) {
                "gif" -> "gif"
                "video" -> "video"
                else -> "photo"
            }

            val thumbBlobId = uploadThumbnailBlob(photo, thumbnailData)

            // Special-photo subtype + camera model (photo-only), derived from a
            // bounded header prefix so a multi-GB source is never buffered — mirrors
            // the full-bytes detection in [uploadPhoto]. Videos/GIFs carry no subtype.
            val detected: com.simplephotos.data.media.MediaSubtype
            val cameraModel: String?
            if (photo.mediaType == "photo") {
                val prefix = openSource().use { readPrefix(it, subtypeScanPrefixBytes) }
                detected = com.simplephotos.data.media.MediaSubtypeDetector.detect(prefix, photo.width, photo.height)
                cameraModel = extractCameraModel(prefix)
                if (detected.photoSubtype != null || detected.burstId != null || cameraModel != null) {
                    android.util.Log.d(TAG,
                        "uploadPhotoChunked: detected subtype=${detected.photoSubtype} burstId=${detected.burstId} camera=$cameraModel for ${photo.filename}")
                }
            } else {
                detected = com.simplephotos.data.media.MediaSubtype()
                cameraModel = null
            }

            // v2 metadata frame: the v1 envelope minus the inline `data`, plus the
            // chunk parameters. Field names mirror the server's chunked encoder
            // (photos/server_migrate_encrypt.rs) so web/server decode it identically.
            val meta = JSONObject().apply {
                put("v", ChunkedBlob.FORMAT_V2)
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
                put("chunk_size", ChunkedBlob.CHUNK_SIZE)
                put("data_len", dataLen)
            }.toString()

            // Stream-encrypt straight to disk — peak heap ≈ one chunk (~4 MiB).
            val result = openSource().use { src ->
                ChunkedBlob.encryptStreamToFile(crypto, src, tmp, meta.toByteArray())
            }

            android.util.Log.d(TAG, "uploadPhotoChunked: streaming media blob (${result.blobSize} bytes, type=$mediaBlobType, contentHash=${result.contentHashHex})")
            val photoBody = tmp.asRequestBody("application/octet-stream".toMediaType())
            val photoRes = api.uploadBlob(
                photoBody,
                mediaBlobType,
                result.blobSize.toString(),
                result.clientHashHex,
                result.contentHashHex,
                ChunkedBlob.FORMAT_V2.toString(),
            )
            android.util.Log.d(TAG, "uploadPhotoChunked: media uploaded, blobId=${photoRes.blobId}")

            registerAndMark(
                photo = photo,
                photoBlobId = photoRes.blobId,
                thumbBlobId = thumbBlobId,
                contentHash = result.contentHashHex,
                detected = detected,
                cameraModel = cameraModel,
                thumbnailData = thumbnailData,
            )
        } catch (e: retrofit2.HttpException) {
            val errorBody = e.response()?.errorBody()?.string()
            android.util.Log.e(TAG, "uploadPhotoChunked HTTP ${e.code()}: $errorBody", e)
            db.photoDao().updateSyncStatus(photo.localId, SyncStatus.FAILED)
            throw e
        } catch (e: Exception) {
            android.util.Log.e(TAG, "uploadPhotoChunked failed: ${e.message}", e)
            db.photoDao().updateSyncStatus(photo.localId, SyncStatus.FAILED)
            throw e
        } finally {
            if (tmp.exists() && !tmp.delete()) {
                android.util.Log.w(TAG, "uploadPhotoChunked: failed to delete temp file ${tmp.absolutePath}")
            }
        }
    }

    /**
     * Encrypt and upload [thumbnailData] as a v1 thumbnail blob, returning its blob
     * id (or null when there's no thumbnail). Shared by [uploadPhoto] and
     * [uploadPhotoChunked]; thumbnails are always small, so they stay v1.
     */
    private suspend fun uploadThumbnailBlob(photo: PhotoEntity, thumbnailData: ByteArray?): String? {
        if (thumbnailData == null || thumbnailData.isEmpty()) {
            android.util.Log.d(TAG, "uploadThumbnailBlob: no thumbnail data for ${photo.filename}, skipping")
            return null
        }
        val thumbBlobType = if (photo.mediaType == "video") "video_thumbnail" else "thumbnail"

        // Decode thumbnail to get actual dimensions (thumbnails are now
        // aspect-ratio-preserving, not always 256×256).
        val thumbBitmap = android.graphics.BitmapFactory.decodeByteArray(thumbnailData, 0, thumbnailData.size)
        val thumbW = thumbBitmap?.width ?: 256
        val thumbH = thumbBitmap?.height ?: 256
        thumbBitmap?.recycle()

        val thumbPayload = JSONObject().apply {
            put("v", 1)
            put("photo_blob_id", "")
            put("width", thumbW)
            put("height", thumbH)
            put("data", android.util.Base64.encodeToString(thumbnailData, android.util.Base64.NO_WRAP))
        }.toString()

        val encryptedThumb = crypto.encrypt(thumbPayload.toByteArray())
        val thumbHash = crypto.sha256Hex(encryptedThumb)
        val thumbBody = encryptedThumb.toRequestBody("application/octet-stream".toMediaType())

        android.util.Log.d(TAG, "uploadThumbnailBlob: uploading thumbnail blob (${encryptedThumb.size} bytes, type=$thumbBlobType)")
        val thumbRes = api.uploadBlob(thumbBody, thumbBlobType, encryptedThumb.size.toString(), thumbHash)
        android.util.Log.d(TAG, "uploadThumbnailBlob: thumbnail uploaded, blobId=${thumbRes.blobId}")
        return thumbRes.blobId
    }

    /**
     * Register an uploaded encrypted photo on the server, then mark it synced and
     * cache its thumbnail locally. Shared finalisation step for both the v1
     * ([uploadPhoto]) and v2 ([uploadPhotoChunked]) upload paths.
     */
    private suspend fun registerAndMark(
        photo: PhotoEntity,
        photoBlobId: String,
        thumbBlobId: String?,
        contentHash: String,
        detected: com.simplephotos.data.media.MediaSubtype,
        cameraModel: String?,
        thumbnailData: ByteArray?,
    ) {
        val regReq = com.simplephotos.data.remote.dto.RegisterEncryptedPhotoRequest(
            filename = photo.filename,
            mimeType = photo.mimeType,
            mediaType = photo.mediaType,
            width = photo.width,
            height = photo.height,
            durationSecs = photo.durationSecs?.toDouble(),
            takenAt = java.time.Instant.ofEpochMilli(photo.takenAt).toString(),
            latitude = photo.latitude,
            longitude = photo.longitude,
            encryptedBlobId = photoBlobId,
            encryptedThumbBlobId = thumbBlobId,
            photoHash = contentHash,
            photoSubtype = detected.photoSubtype,
            burstId = detected.burstId,
            cameraModel = cameraModel
        )
        val regRes = api.registerEncryptedPhoto(regReq)
        android.util.Log.d(TAG, "registerAndMark: registered photo, serverPhotoId=${regRes.photoId}, duplicate=${regRes.duplicate}")

        db.photoDao().markSynced(photo.localId, regRes.photoId, photoBlobId, thumbBlobId, contentHash)
        // Persist locally too so the device gallery badges it immediately
        // (server timestamp-burst grouping is reflected on the next sync).
        if (detected.photoSubtype != null || detected.burstId != null) {
            db.photoDao().backfillSubtypeFields(regRes.photoId, detected.photoSubtype, detected.burstId, null)
        }

        // Cache uploaded thumbnail locally
        if (thumbnailData != null && thumbnailData.isNotEmpty()) {
            saveThumbnailToDisk(photo.localId, thumbnailData)
        }
    }

    /**
     * Stream the embedded motion-photo video for [photoId] to [outputFile].
     *
     * Hits `GET /api/photos/{id}/motion-video`, which returns a ready-to-play
     * MP4 (the server extracts/decrypts the trailer). No client-side decryption
     * — unlike photo/video blobs, this is already plaintext `video/mp4`.
     */
    suspend fun downloadMotionVideoToFile(photoId: String, outputFile: File) {
        val response = api.serveMotionVideo(photoId)
        response.byteStream().use { input ->
            outputFile.outputStream().buffered().use { output ->
                input.copyTo(output, bufferSize = 8192)
            }
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
     * Download a blob and decrypt it into raw **media bytes**, handling both the
     * v1 monolithic envelope and the v2 chunked container (large files). Used
     * for photos that Coil renders from a `ByteArray`; videos use the streaming
     * [downloadAndDecryptBlobToFile] path instead.
     */
    suspend fun downloadAndDecryptMediaBytes(blobId: String): ByteArray {
        val response = api.downloadBlob(blobId)
        val encrypted = response.bytes()
        return ChunkedBlob.decryptPhotoBlobBytes(crypto, encrypted)
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

            // Step 2: Detect the container format from the leading magic bytes.
            val head = ByteArray(ChunkedBlob.MAGIC_SIZE)
            val headLen = encryptedTempFile.inputStream().use { it.read(head) }

            if (ChunkedBlob.isChunked(head, headLen)) {
                // v2 chunked: stream each ~4 MiB chunk frame straight to disk so a
                // multi-gigabyte video never lives entirely in the Java heap.
                encryptedTempFile.inputStream().use { input ->
                    ChunkedBlob.decryptChunkedStreamToFile(crypto, input, outputFile)
                }
            } else {
                // v1 monolithic: decrypt the whole blob (one GCM allocation), then
                // stream the base64 "data" field to disk in chunks.
                val encrypted = encryptedTempFile.readBytes()
                val decrypted = crypto.decrypt(encrypted)
                // encrypted is now GC-eligible
                streamExtractBase64ToFile(decrypted, outputFile)
                // decrypted is now GC-eligible
            }
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

        // Decode the base64 "data" value to the output file.
        //
        // This previously decoded fixed 48k-char chunks with Base64.NO_WRAP,
        // which threw "bad base-64" for blobs whose base64 uses the URL-safe
        // alphabet ('-'/'_' instead of '+'/'/') — some secure items (e.g.
        // motion/burst added via the web "secure add" flow) are encoded that
        // way, while Android-backed-up blobs use the standard alphabet. Two
        // fixes: (1) normalise URL-safe → standard in place so either alphabet
        // decodes, (2) stream-decode the whole value via Base64InputStream so a
        // chunk boundary can never split a 4-char group (and a missing-padding
        // tail is flushed at EOF). Whitespace is skipped by the decoder.
        var urlSafe = false
        for (i in markerIdx until endIdx) {
            when (decrypted[i]) {
                '-'.code.toByte() -> { decrypted[i] = '+'.code.toByte(); urlSafe = true }
                '_'.code.toByte() -> { decrypted[i] = '/'.code.toByte(); urlSafe = true }
            }
        }
        val regionLen = endIdx - markerIdx
        android.util.Log.d(TAG, "decoding base64 data: $regionLen chars (len%4=${regionLen % 4}, urlSafe=$urlSafe)")
        java.io.ByteArrayInputStream(decrypted, markerIdx, regionLen).use { region ->
            android.util.Base64InputStream(region, android.util.Base64.NO_WRAP).use { b64In ->
                outputFile.outputStream().buffered().use { out -> b64In.copyTo(out, 64 * 1024) }
            }
        }
    }

    // ── Sync from server ─────────────────────────────────────────────────

    /** Pull photos from the server (always encrypted). */
    suspend fun syncFromServer(): Int {
        deduplicateLocalEntities()
        val imported = syncFromServerEncrypted()
        // Remove local entries for photos that were deleted on server
        // (e.g. trashed from web or another device).
        reconcileServerDeletions()
        // Also pull crop_metadata updates for already-synced photos so
        // non-destructive edits from web/other devices are reflected.
        val cropUpdated = syncCropMetadata()
        val favUpdated = syncFavorites()
        return imported + cropUpdated + favUpdated
    }

    /**
     * Detect photos deleted on the server (via web or another device) and
     * remove them from the local DB. Compares local SYNCED entries that have
     * a serverPhotoId against the server's encrypted-sync manifest.
     *
     * Only removes entries that:
     * 1. Have a serverPhotoId (were synced from server)
     * 2. Don't have a localPath (not locally-captured photos)
     *
     * This ensures locally-captured photos that haven't been uploaded yet
     * are never deleted during reconciliation.
     */
    private suspend fun reconcileServerDeletions() {
        try {
            // Collect ALL server photo IDs from the encrypted-sync endpoint
            val serverPhotoIds = mutableSetOf<String>()
            var after: String? = null
            do {
                val result = api.encryptedSync(after = after, limit = 500)
                for (photo in result.photos) {
                    serverPhotoIds.add(photo.id)
                }
                after = result.nextCursor
            } while (result.nextCursor != null)

            // Find local entries synced from server that no longer exist there
            val localSynced = db.photoDao().getByStatus(SyncStatus.SYNCED)
            val serverOnlyLocal = localSynced.filter { it.serverPhotoId != null && it.localPath == null }

            var removed = 0
            for (photo in serverOnlyLocal) {
                if (photo.serverPhotoId!! !in serverPhotoIds) {
                    android.util.Log.i(TAG, "reconcileServerDeletions: removing '${photo.filename}' " +
                            "(serverPhotoId=${photo.serverPhotoId}) — no longer on server")
                    photo.thumbnailPath?.let { File(it).delete() }
                    db.photoDao().delete(photo)
                    removed++
                }
            }

            if (removed > 0) {
                android.util.Log.i(TAG, "reconcileServerDeletions: removed $removed orphaned entries")
            } else {
                android.util.Log.d(TAG, "reconcileServerDeletions: all entries in sync")
            }
        } catch (e: Exception) {
            android.util.Log.w(TAG, "reconcileServerDeletions: failed — ${e.message}")
        }
    }

    /**
     * One-time cleanup: find local entities (from MediaStore scan) that have
     * a duplicate server-synced entity (from encrypted sync) and merge them.
     * Keeps the local entity (which has localPath) and deletes the server-only duplicate.
     */
    private suspend fun deduplicateLocalEntities() {
        try {
            // Find server-synced entities (have serverPhotoId, no localPath)
            // that overlap with local entities (have localPath, no serverPhotoId)
            // by matching serverBlobId.
            val allPhotos = db.photoDao().getAllPhotosSnapshot()
            val serverOnly = allPhotos.filter { it.serverPhotoId != null && it.localPath == null }
            val localOnly = allPhotos.filter { it.serverPhotoId == null && it.localPath != null }

            if (serverOnly.isEmpty() || localOnly.isEmpty()) return

            // Build lookup by photoHash and filename+takenAt
            val localByHash = localOnly.filter { it.photoHash != null }
                .associateBy { it.photoHash!! }
            val localByKey = localOnly.associateBy { "${it.filename}|${it.takenAt}" }

            var deduped = 0
            for (serverEntity in serverOnly) {
                val localMatch = serverEntity.photoHash?.let { localByHash[it] }
                    ?: localByKey["${serverEntity.filename}|${serverEntity.takenAt}"]
                    ?: continue

                // Merge: update the local entity with server fields, delete server-only dup
                db.photoDao().mergeServerPhoto(
                    localId = localMatch.localId,
                    serverPhotoId = serverEntity.serverPhotoId!!,
                    blobId = serverEntity.serverBlobId ?: localMatch.serverBlobId ?: continue,
                    thumbBlobId = serverEntity.thumbnailBlobId,
                    cropMetadata = serverEntity.cropMetadata,
                    photoHash = serverEntity.photoHash,
                    isFavorite = serverEntity.isFavorite
                )
                // Delete the server-only duplicate and its thumbnail file
                serverEntity.thumbnailPath?.let { File(it).delete() }
                db.photoDao().deleteById(serverEntity.localId)
                deduped++
            }
            if (deduped > 0) {
                android.util.Log.i("PhotoRepository", "deduplicateLocalEntities: merged $deduped duplicate entries")
            }
        } catch (e: Exception) {
            android.util.Log.w("PhotoRepository", "deduplicateLocalEntities failed: ${e.message}")
        }
    }

    /**
     * Pull encrypted-mode photos from the server using the lightweight sync
     * manifest. This avoids downloading and decrypting every full-size photo
     * blob — it reads photo metadata directly from the server's photos table
     * and then downloads only the small thumbnail blobs (~30 KB each).
     */
    private suspend fun syncFromServerEncrypted(): Int {
        var imported = 0
        var merged = 0
        var after: String? = null
        android.util.Log.i("PhotoRepository", "syncFromServerEncrypted: starting sync")

        do {
            val result = api.encryptedSync(after = after, limit = 500)
            android.util.Log.d("PhotoRepository", "syncFromServerEncrypted: fetched page with ${result.photos.size} photos, nextCursor=${result.nextCursor}")

            for (photo in result.photos) {
                val blobId = photo.encryptedBlobId ?: continue

                // Skip if already in local DB — use serverPhotoId (unique per
                // photo row) instead of blobId because duplicates / edit copies
                // share the same encrypted_blob_id.
                val existing = db.photoDao().getByServerPhotoId(photo.id)
                if (existing != null) {
                    // Back-fill burst / motion / photo-subtype fields whenever
                    // they have appeared (or changed) on the server. Without
                    // this, photos synced before the server learnt their
                    // subtype would never get burst-stacking or LIVE overlay
                    // on this device.
                    if (existing.photoSubtype != photo.photoSubtype ||
                        existing.burstId != photo.burstId ||
                        existing.motionVideoBlobId != photo.motionVideoBlobId) {
                        db.photoDao().backfillSubtypeFields(
                            serverPhotoId = photo.id,
                            subtype = photo.photoSubtype,
                            burstId = photo.burstId,
                            motionBlobId = photo.motionVideoBlobId,
                        )
                    }
                    continue
                }

                val serverTimestamp = photo.takenAt ?: photo.createdAt
                val takenAtMs = parseIsoToEpochMs(serverTimestamp)

                // ── Merge with existing local entity ─────────────────────
                // When a photo was scanned from MediaStore and uploaded, the
                // local entity has localPath + serverBlobId but no serverPhotoId.
                // Instead of creating a duplicate, merge the server photo ID
                // into the existing entity so edits use the server API.
                val localMatch = photo.photoHash?.let { hash ->
                    db.photoDao().getLocalByHash(hash)
                } ?: db.photoDao().getLocalByFilenameAndDate(photo.filename, takenAtMs)

                if (localMatch != null) {
                    db.photoDao().mergeServerPhoto(
                        localId = localMatch.localId,
                        serverPhotoId = photo.id,
                        blobId = blobId,
                        thumbBlobId = photo.encryptedThumbBlobId,
                        cropMetadata = photo.cropMetadata,
                        photoHash = photo.photoHash,
                        isFavorite = photo.isFavorite
                    )
                    // mergeServerPhoto only sets the core identity columns;
                    // back-fill burst/motion/subtype separately so the merged
                    // row participates in burst stacking and LIVE overlays.
                    if (photo.photoSubtype != null || photo.burstId != null || photo.motionVideoBlobId != null) {
                        db.photoDao().backfillSubtypeFields(
                            serverPhotoId = photo.id,
                            subtype = photo.photoSubtype,
                            burstId = photo.burstId,
                            motionBlobId = photo.motionVideoBlobId,
                        )
                    }
                    android.util.Log.d("PhotoRepository", "syncFromServerEncrypted: merged server photo '${photo.filename}' (${photo.id}) into local entity ${localMatch.localId}")
                    merged++
                    continue
                }

                val localId = java.util.UUID.randomUUID().toString()
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

                // If the photo has crop_metadata with 90°/270° rotation, swap
                // width/height so the grid tile reflects the edited orientation.
                var w = photo.width.toInt()
                var h = photo.height.toInt()
                if (!photo.cropMetadata.isNullOrEmpty()) {
                    try {
                        val cm = org.json.JSONObject(photo.cropMetadata)
                        val rot = ((cm.optInt("rotate", 0) % 360) + 360) % 360
                        if ((rot == 90 || rot == 270) && w > 0 && h > 0) {
                            val tmp = w; w = h; h = tmp
                        }
                    } catch (_: Exception) { /* ignore malformed JSON */ }
                }

                val entity = PhotoEntity(
                    localId = localId,
                    serverBlobId = blobId,
                    thumbnailBlobId = thumbBlobId,
                    filename = photo.filename,
                    takenAt = takenAtMs,
                    // Library import order — mirror the server's created_at (the
                    // web stores this as `addedAt`). Without this, createdAt
                    // defaulted to the LOCAL insert time, so "Recently Added"
                    // reflected device-sync order instead of server-add order
                    // and never matched the web/server recents list.
                    createdAt = parseIsoToEpochMs(photo.createdAt),
                    mimeType = photo.mimeType,
                    mediaType = photo.mediaType,
                    width = w,
                    height = h,
                    durationSecs = photo.durationSecs?.toFloat(),
                    sizeBytes = photo.sizeBytes,
                    localPath = null,
                    thumbnailPath = thumbPath,
                    syncStatus = SyncStatus.SYNCED,
                    isFavorite = photo.isFavorite,
                    cropMetadata = photo.cropMetadata,
                    photoHash = photo.photoHash,
                    serverPhotoId = photo.id,
                    photoSubtype = photo.photoSubtype,
                    burstId = photo.burstId,
                    motionVideoBlobId = photo.motionVideoBlobId,
                    sourcePath = photo.sourcePath
                )
                db.photoDao().insert(entity)
                imported++
            }

            after = result.nextCursor
        } while (result.nextCursor != null)

        android.util.Log.i("PhotoRepository", "syncFromServerEncrypted: finished — imported $imported, merged $merged photos")
        return imported + merged
    }

    /**
     * Pull crop_metadata updates from the server for photos that have already
     * been synced.  The main encrypted-sync skips existing photos, so non-
     * destructive edits made on another device would never arrive otherwise.
     */
    suspend fun syncCropMetadata(): Int {
        var updated = 0
        try {
            val records = api.cropSync()
            for (record in records) {
                val existing = db.photoDao().getByServerPhotoId(record.id) ?: continue
                if (existing.cropMetadata != record.cropMetadata) {
                    db.photoDao().updateCropMetadata(existing.localId, record.cropMetadata)
                    updated++
                    android.util.Log.d("PhotoRepository",
                        "syncCropMetadata: updated crop for ${record.id} (local=${existing.localId})")
                }
            }
        } catch (e: Exception) {
            android.util.Log.w("PhotoRepository", "syncCropMetadata failed: ${e.message}")
        }
        return updated
    }

    /**
     * Pull is_favorite updates from the server for photos that have already
     * been synced.  The main encrypted-sync skips existing photos, so
     * favorites toggled on the web would never arrive otherwise.
     */
    suspend fun syncFavorites(): Int {
        var updated = 0
        try {
            val records = api.favoriteSync()
            for (record in records) {
                val existing = db.photoDao().getByServerPhotoId(record.id) ?: continue
                if (existing.isFavorite != record.isFavorite) {
                    db.photoDao().updateFavorite(existing.localId, record.isFavorite)
                    updated++
                    android.util.Log.d("PhotoRepository",
                        "syncFavorites: updated fav for ${record.id} (local=${existing.localId}) → ${record.isFavorite}")
                }
            }
        } catch (e: Exception) {
            android.util.Log.w("PhotoRepository", "syncFavorites failed: ${e.message}")
        }
        return updated
    }

    /**
     * Save thumbnail bytes to app-internal storage.
     * Detects GIF magic bytes and uses `.gif` extension so Coil's
     * GifDecoder can animate them; everything else gets `.jpg`.
     * Returns the absolute path to the saved file.
     */
    fun saveThumbnailToDisk(photoLocalId: String, thumbnailBytes: ByteArray): String {
        val isGif = thumbnailBytes.size >= 3 &&
            thumbnailBytes[0] == 0x47.toByte() && // 'G'
            thumbnailBytes[1] == 0x49.toByte() && // 'I'
            thumbnailBytes[2] == 0x46.toByte()    // 'F'
        val ext = if (isGif) "gif" else "jpg"
        val file = File(thumbnailDir, "$photoLocalId.$ext")
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

    /** Toggle the is_favorite flag on the server and persist to local DB. */
    suspend fun toggleFavorite(photoId: String): FavoriteToggleResponse {
        val response = api.toggleFavorite(photoId)
        val local = db.photoDao().getByServerPhotoId(photoId)
        if (local != null) {
            db.photoDao().updateFavorite(local.localId, response.isFavorite)
        }
        return response
    }

    /** Persist crop/brightness/trim metadata on the server. */
    suspend fun setCropOnServer(photoId: String, cropMetadata: String?) {
        android.util.Log.d(TAG, "[setCrop] photoId=$photoId, meta=$cropMetadata")
        api.setCrop(photoId, SetCropRequest(cropMetadata))
    }

    // ── Metadata edit (full EXIF view + edit + write-back) ───────────────
    // Mirrors web/src/api/metadata.ts; used by PhotoViewerViewModel/ViewerInfoPanel.

    /** Fetch the full metadata + raw EXIF tags for the Info panel. */
    suspend fun getFullMetadata(photoId: String): FullMetadataResponse =
        api.getFullMetadata(photoId)

    /** PATCH only the changed metadata fields (nulls omitted by Gson). */
    suspend fun updateMetadata(photoId: String, request: MetadataUpdateRequest): MetadataUpdateResponse =
        api.updateMetadata(photoId, request)

    /** Write the current DB metadata back to the file's EXIF (jpeg/tiff). */
    suspend fun writeExif(photoId: String): WriteExifResponse =
        api.writeExif(photoId)

    /** Persist a manual photo-subtype correction to the local cache so the
     *  pano/360 viewer switches immediately (the periodic resync would lag). */
    suspend fun updateLocalSubtype(serverPhotoId: String, subtype: String?) {
        db.photoDao().updatePhotoSubtype(serverPhotoId, subtype)
    }

    /** Create a server-side duplicate of a photo ("Save Copy"). */
    suspend fun duplicatePhotoOnServer(
        photoId: String,
        cropMetadata: String?
    ): DuplicatePhotoResponse {
        android.util.Log.d(TAG, "[duplicate] photoId=$photoId, meta=$cropMetadata")
        val res = api.duplicatePhoto(photoId, DuplicatePhotoRequest(cropMetadata))
        android.util.Log.d(TAG, "[duplicate] Response: id=${res.id}, dims=${res.width}×${res.height}, " +
            "blobId=${res.encryptedBlobId}, thumbBlobId=${res.encryptedThumbBlobId}, " +
            "sizeBytes=${res.sizeBytes}, mime=${res.mimeType}")
        return res
    }

    // ── Diagnostic helpers (used by GalleryViewModel) ────────────────────

    /** Count photos in a specific sync status (PENDING, FAILED, etc.). */
    suspend fun getPhotoCountByStatus(status: SyncStatus): Int =
        db.photoDao().getByStatus(status).size

    /** Look up a synced photo by its content hash (for import dedup). */
    suspend fun getSyncedByHash(hash: String): PhotoEntity? =
        db.photoDao().getSyncedByHash(hash)
}
