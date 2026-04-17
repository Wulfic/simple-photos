/**
 * ViewModel that manages state and actions for the full-screen photo viewer.
 */
package com.simplephotos.ui.screens.viewer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.TagRepository
import android.util.Log
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import org.json.JSONObject
import javax.inject.Inject

// ---------------------------------------------------------------------------
// ViewModel — loads photo list for paging + handles deletion
// ---------------------------------------------------------------------------

/**
 * ViewModel for the full-screen photo/video viewer with horizontal paging.
 *
 * Handles: encrypted blob download & decryption, favorites,
 * crop/brightness metadata, photo duplication ("Save Copy"), and album removal.
 * Supports memory-efficient streaming decryption to temp files for large videos.
 */
@HiltViewModel
class PhotoViewerViewModel @Inject constructor(
    private val photoRepository: PhotoRepository,
    private val albumRepository: AlbumRepository,
    private val tagRepository: TagRepository,
    val okHttpClient: OkHttpClient,
    savedStateHandle: SavedStateHandle
) : ViewModel() {

    companion object {
        private const val TAG = "PhotoViewerVM"
    }

    private val initialPhotoId: String = savedStateHandle["photoId"] ?: ""

    /** Album context — non-null when viewer was opened from an album. */
    val albumId: String? = savedStateHandle["albumId"]

    /** Full photo list for paging (matches gallery order). */
    var allPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
        private set

    /** Index of the photo that was tapped in the gallery. */
    var initialPage by mutableStateOf(0)
        private set

    /** True while the photo list is still loading. */
    var listLoading by mutableStateOf(true)
        private set

    var serverBaseUrl by mutableStateOf("")
        private set

    var error by mutableStateOf<String?>(null)
        private set

    /** True while a server-side duplicate render (e.g. video re-encode) is in progress. */
    var isRenderingCopy by mutableStateOf(false)
        private set

    /** Tags for the currently viewed photo. */
    var currentTags by mutableStateOf<List<String>>(emptyList())
        private set

    /** All user tags for suggestions. */
    var allTags by mutableStateOf<List<String>>(emptyList())
        private set

    /** Favorite state for the currently viewed photo. */
    var isFavorite by mutableStateOf(false)
        private set

    init {
        loadPhotos()
    }

    private fun loadPhotos() {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }

                val photos = if (albumId != null) {
                    // Album context: load only photos in this album
                    withContext(Dispatchers.IO) {
                        val photoIds = albumRepository.getPhotoIdsForAlbum(albumId)
                        photoIds.mapNotNull { id -> photoRepository.getPhoto(id) }
                    }
                } else {
                    // Gallery context: load all photos
                    withContext(Dispatchers.IO) {
                        photoRepository.getAllPhotos().first()
                    }
                }
                allPhotos = photos
                initialPage = photos.indexOfFirst { it.localId == initialPhotoId }
                    .coerceAtLeast(0)
            } catch (e: Exception) {
                error = e.message
            } finally {
                listLoading = false
            }
        }
    }

    /**
     * Download and decrypt an encrypted blob, returning the raw media bytes.
     * Called from per-page composables for encrypted-mode photos.
     *
     * Memory: the decrypted JSON envelope contains a base64-encoded "data"
     * field. We extract just that field and decode it, then let the
     * intermediate strings become GC-eligible before returning.
     */
    suspend fun downloadAndDecrypt(blobId: String): ByteArray = withContext(Dispatchers.IO) {
        val decrypted = photoRepository.downloadAndDecryptBlob(blobId)
        // Parse and extract the base64 payload. We use JSONObject which
        // unfortunately copies the string, but we null-out intermediates
        // so GC can reclaim them during the Base64 decode.
        val jsonStr = String(decrypted, Charsets.UTF_8)
        // decrypted ByteArray is now GC-eligible (jsonStr holds the data)
        val payload = JSONObject(jsonStr)
        val dataBase64 = payload.getString("data")
        // payload and jsonStr are now GC-eligible (only dataBase64 is needed)
        android.util.Base64.decode(dataBase64, android.util.Base64.NO_WRAP)
    }

    /**
     * Download, decrypt, and write media directly to a temp file.
     * Used for video/audio to avoid OOM — the decoded bytes never live
     * entirely in the Java heap (only the encrypted blob + decrypted JSON
     * are in memory; base64 is decoded in chunks to disk).
     *
     * Peak heap: ~1× blob size (vs ~4× with downloadAndDecrypt).
     */
    suspend fun downloadAndDecryptToFile(blobId: String, outputFile: java.io.File) = withContext(Dispatchers.IO) {
        photoRepository.downloadAndDecryptBlobToFile(blobId, outputFile)
    }

    fun deletePhoto(photo: PhotoEntity, onDeleted: () -> Unit) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { photoRepository.deletePhoto(photo) }
                onDeleted()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    /** Remove a photo from the current album (does NOT delete the photo). */
    fun removeFromAlbum(photo: PhotoEntity, onRemoved: () -> Unit) {
        val aid = albumId ?: return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    albumRepository.removePhotoFromAlbum(photo.localId, aid)
                    try {
                        albumRepository.getAlbum(aid)?.let { albumRepository.syncAlbum(it) }
                    } catch (_: Exception) {}
                }
                // Remove from in-memory list and navigate
                allPhotos = allPhotos.filter { it.localId != photo.localId }
                onRemoved()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    /** Load tags for a specific photo — no-op in encrypted mode. */
    fun loadTagsForPhoto(photoId: String?) {
        if (photoId == null) return
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) { tagRepository.getPhotoTags(photoId) }
                currentTags = response.tags.sorted()
            } catch (_: Exception) {
                currentTags = emptyList()
            }
        }
    }

    /** Load all user tags for suggestions. */
    fun loadAllTags() {
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) { tagRepository.listTags() }
                allTags = response.tags.sorted()
            } catch (_: Exception) {}
        }
    }

    /** Add a tag to the current photo. */
    fun addTag(photoId: String, tag: String) {
        // Strip dangerous control chars, bidi overrides, zero-width chars
        val cleaned = tag.trim().lowercase()
            .replace(Regex("[\\u0000-\\u001F\\u007F\\u0080-\\u009F\\u200B-\\u200F\\u202A-\\u202E\\u2066-\\u2069\\uFEFF\\uFFFE]"), "")
            .take(100)
        if (cleaned.isEmpty()) return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { tagRepository.addTag(photoId, cleaned) }
                if (!currentTags.contains(cleaned)) {
                    currentTags = (currentTags + cleaned).sorted()
                }
                if (!allTags.contains(cleaned)) {
                    allTags = (allTags + cleaned).sorted()
                }
            } catch (_: Exception) {}
        }
    }

    /** Remove a tag from the current photo. */
    fun removeTag(photoId: String, tag: String) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { tagRepository.removeTag(photoId, tag) }
                currentTags = currentTags.filter { it != tag }
            } catch (_: Exception) {}
        }
    }

    /** Load favorite state from the current photo entity. */
    fun loadFavoriteForPhoto(photo: PhotoEntity?) {
        isFavorite = photo?.isFavorite ?: false
    }

    /** Toggle the favorite state of the current photo. */
    fun toggleFavorite(photoId: String) {
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) { photoRepository.toggleFavorite(photoId) }
                isFavorite = response.isFavorite
                // Update the local allPhotos list so loadFavoriteForPhoto()
                // returns the correct value when the user swipes away and back.
                val idx = allPhotos.indexOfFirst { it.serverPhotoId == photoId }
                if (idx >= 0) {
                    allPhotos = allPhotos.toMutableList().also {
                        it[idx] = it[idx].copy(isFavorite = response.isFavorite)
                    }
                }
            } catch (_: Exception) {}
        }
    }

    /** Save crop/brightness/trim metadata for a photo. */
    fun saveCropMetadata(photo: PhotoEntity, metadata: String?) {
        Log.d(TAG, "[EDIT:saveEdit] photo=${photo.localId}, server=${photo.serverPhotoId}, " +
            "dims=${photo.width}×${photo.height}, mediaType=${photo.mediaType}, " +
            "hasMeta=${metadata != null}, meta=$metadata")
        viewModelScope.launch {
            try {
                // Update local DB
                withContext(Dispatchers.IO) {
                    photoRepository.updateCropMetadata(photo.localId, metadata)
                }
                Log.d(TAG, "[EDIT:saveEdit] Local DB updated for ${photo.localId}")
                // Update in-memory list
                val idx = allPhotos.indexOfFirst { it.localId == photo.localId }
                if (idx >= 0) {
                    allPhotos = allPhotos.toMutableList().also {
                        it[idx] = it[idx].copy(cropMetadata = metadata)
                    }
                }
                // Sync to server so web and other clients see the edit
                photo.serverPhotoId?.let { serverId ->
                    withContext(Dispatchers.IO) {
                        try {
                            photoRepository.setCropOnServer(serverId, metadata)
                            Log.d(TAG, "[EDIT:saveEdit] Server sync OK for $serverId")
                        } catch (e: Exception) {
                            Log.w(TAG, "[EDIT:saveEdit] Server sync failed for $serverId: ${e.message}")
                        }
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "[EDIT:saveEdit] Failed: ${e.message}", e)
            }
        }
    }

    /** Duplicate a photo with optional crop/edit metadata (Save Copy).
     *  When the photo has a server record, calls the server duplicate
     *  endpoint which renders edits via ffmpeg into a fully independent file. */
    fun duplicatePhoto(photo: PhotoEntity, metadata: String?, onDone: () -> Unit = {}) {
        Log.d(TAG, "[EDIT:saveCopy] Starting duplicate for photo=${photo.localId}, " +
            "server=${photo.serverPhotoId}, dims=${photo.width}×${photo.height}, " +
            "mediaType=${photo.mediaType}, mime=${photo.mimeType}, " +
            "hasMeta=${metadata != null}, meta=$metadata")
        // Exit edit mode and show rendering banner immediately so the user
        // isn't stuck on the edit screen during a 60-90 s video re-encode.
        onDone()
        isRenderingCopy = true
        Log.d(TAG, "[EDIT:saveCopy] isRenderingCopy set to TRUE")
        viewModelScope.launch {
            try {
                val copyId = java.util.UUID.randomUUID().toString()

                // If the photo has a server record, call the server's duplicate
                // endpoint — it renders via ffmpeg and produces an independent file
                // with crop_metadata=NULL (edits baked in).
                var serverCopyId: String? = null
                var serverWidth = photo.width
                var serverHeight = photo.height
                var serverDuration = photo.durationSecs
                var serverBlobId: String? = null
                var serverThumbBlobId: String? = null
                photo.serverPhotoId?.let { serverId ->
                    try {
                        // Use NonCancellable so the server render (which may
                        // take 60-90 s for video re-encoding) finishes even if
                        // the user navigates away from the viewer.
                        val res = withContext(Dispatchers.IO + NonCancellable) {
                            photoRepository.duplicatePhotoOnServer(serverId, metadata)
                        }
                        serverCopyId = res.id
                        // Use server-probed dimensions (correct after rotation/crop)
                        if (res.width > 0 && res.height > 0) {
                            serverWidth = res.width
                            serverHeight = res.height
                        }
                        if (res.durationSecs != null) {
                            serverDuration = res.durationSecs
                        }
                        serverBlobId = res.encryptedBlobId
                        serverThumbBlobId = res.encryptedThumbBlobId
                        Log.d(TAG, "[EDIT:saveCopy] Server duplicate OK: copyId=${res.id}, " +
                            "dims=${res.width}×${res.height}, duration=${res.durationSecs}, " +
                            "blobId=${res.encryptedBlobId}, thumbBlobId=${res.encryptedThumbBlobId}, " +
                            "sizeBytes=${res.sizeBytes}")
                    } catch (e: Exception) {
                        Log.w(TAG, "[EDIT:saveCopy] Server duplicate failed: ${e.message}")
                    }
                }

                // For local-only copies (no server), keep the original content URI
                // so BackupWorker can upload it later. For server copies, make a
                // cache copy for offline viewing.
                var newLocalPath: String? = null
                if (serverCopyId != null) {
                    photo.localPath?.let { oldPath ->
                        val cacheDir = withContext(Dispatchers.IO) {
                            photoRepository.getCacheDir()
                        }
                        val ext = photo.filename.substringAfterLast('.', "jpg")
                        val destFile = java.io.File(cacheDir, "copy_${copyId}.$ext")
                        newLocalPath = withContext(Dispatchers.IO) {
                            photoRepository.copyLocalFile(oldPath, destFile)
                        }
                        Log.d(TAG, "[EDIT:saveCopy] Local file copied: $oldPath → ${destFile.absolutePath}")
                    }
                } else {
                    // Keep original content URI so BackupWorker can read it
                    newLocalPath = photo.localPath
                    Log.d(TAG, "[EDIT:saveCopy] Local-only copy, reusing original localPath: $newLocalPath")
                }

                val copyEntity = photo.copy(
                    localId = copyId,
                    serverPhotoId = serverCopyId,
                    filename = if (photo.filename.startsWith("Copy of ")) photo.filename
                               else "Copy of ${photo.filename}",
                    // Server bakes edits into the file, so crop_metadata is NULL
                    // when we have a server copy. For local-only copies, keep metadata
                    // so the viewer renders edits client-side.
                    cropMetadata = if (serverCopyId != null) null else metadata,
                    width = serverWidth,
                    height = serverHeight,
                    durationSecs = serverDuration,
                    createdAt = System.currentTimeMillis(),
                    localPath = newLocalPath,
                    syncStatus = if (serverCopyId != null) SyncStatus.SYNCED else SyncStatus.PENDING,
                    // Server copies use their own blob IDs. Local-only copies
                    // must have null blob IDs so BackupWorker picks them up.
                    serverBlobId = if (serverCopyId != null) serverBlobId else null,
                    thumbnailBlobId = if (serverCopyId != null) serverThumbBlobId else null,
                    // Clear content hash so BackupWorker doesn't dedup against original
                    photoHash = null,
                )
                withContext(Dispatchers.IO) {
                    photoRepository.insertPhoto(copyEntity)
                }
                Log.d(TAG, "[EDIT:saveCopy] Copy inserted to DB: localId=$copyId, " +
                    "serverPhotoId=$serverCopyId, dims=${copyEntity.width}×${copyEntity.height}, " +
                    "blobId=${copyEntity.serverBlobId}, thumbBlobId=${copyEntity.thumbnailBlobId}, " +
                    "syncStatus=${copyEntity.syncStatus}, cropMetadata=${copyEntity.cropMetadata}")

                // For server copies, prefer the server-generated thumbnail
                // (correct orientation, edits baked in). Fall back to
                // generating one locally from the original's thumbnail
                // if the server thumbnail download fails.
                if (serverCopyId != null) {
                    var thumbSaved = false
                    if (serverThumbBlobId != null) {
                        withContext(Dispatchers.IO) {
                            try {
                                val thumbDecrypted = photoRepository.downloadAndDecryptBlob(serverThumbBlobId!!)
                                val thumbPayload = org.json.JSONObject(String(thumbDecrypted, Charsets.UTF_8))
                                val thumbBase64 = thumbPayload.optString("data", "")
                                if (thumbBase64.isNotEmpty()) {
                                    val thumbBytes = android.util.Base64.decode(thumbBase64, android.util.Base64.NO_WRAP)
                                    val thumbPath = photoRepository.saveThumbnailToDisk(copyId, thumbBytes)
                                    photoRepository.updateThumbnailPath(copyId, thumbPath)
                                    Log.d(TAG, "[EDIT:thumb] Downloaded server thumbnail for copy $copyId (${thumbBytes.size} bytes)")
                                    thumbSaved = true
                                }
                            } catch (e: Exception) {
                                Log.w(TAG, "[EDIT:thumb] Server thumbnail download failed, falling back to local: ${e.message}")
                            }
                        }
                    }
                    if (!thumbSaved) {
                        withContext(Dispatchers.IO) {
                            generateEditedThumbnail(photo, copyId, metadata)
                        }
                    }
                } else {
                    // Copy the original's thumbnail as-is for the local copy
                    withContext(Dispatchers.IO) {
                        val srcPath = photo.thumbnailPath
                        if (srcPath != null) {
                            try {
                                val thumbBytes = java.io.File(srcPath).readBytes()
                                val thumbPath = photoRepository.saveThumbnailToDisk(copyId, thumbBytes)
                                photoRepository.updateThumbnailPath(copyId, thumbPath)
                                Log.d(TAG, "[EDIT:thumb] Copied original thumbnail for local copy $copyId")
                            } catch (e: Exception) {
                                Log.w(TAG, "[EDIT:thumb] Failed to copy thumbnail: ${e.message}")
                            }
                        }
                    }
                }
                Log.d(TAG, "[EDIT:saveCopy] Duplicate complete, clearing rendering flag")
                isRenderingCopy = false
            } catch (e: Exception) {
                Log.e(TAG, "[EDIT:saveCopy] Failed: ${e.message}", e)
                isRenderingCopy = false
            }
        }
    }

    /**
     * Download the photo's full-resolution file and save it to [outputFile].
     *
     * Downloads + decrypts the blob to file via
     * [PhotoRepository.downloadAndDecryptBlobToFile].
     *
     * Returns `true` on success, `false` on failure.
     */
    suspend fun downloadPhotoToFile(photo: PhotoEntity, outputFile: java.io.File): Boolean =
        withContext(Dispatchers.IO) {
            try {
                when {
                    photo.serverBlobId != null -> {
                        photoRepository.downloadAndDecryptBlobToFile(photo.serverBlobId!!, outputFile)
                        true
                    }
                    else -> false
                }
            } catch (_: Exception) { false }
        }

    /** Download the photo's full-resolution file bytes (for saving local files to device). */
    suspend fun downloadPhotoBytes(photo: PhotoEntity): ByteArray? = withContext(Dispatchers.IO) {
        try {
            when {
                photo.serverBlobId != null -> {
                    downloadAndDecrypt(photo.serverBlobId!!)
                }
                else -> null
            }
        } catch (_: Exception) { null }
    }

    // ── Thumbnail helpers ────────────────────────────────────────────────

    /**
     * Generate a thumbnail for an edited copy by reading the original's
     * cached thumbnail, applying crop/brightness/rotation via Canvas, and
     * saving the result for the new [copyId].
     *
     * Non-fatal: if the original has no thumbnail or decoding fails the copy
     * simply won't have a thumbnail immediately — the gallery will show a
     * placeholder until the next sync fills it in.
     */
    private suspend fun generateEditedThumbnail(
        original: PhotoEntity,
        copyId: String,
        metadata: String?,
    ) {
        try {
            val srcPath = original.thumbnailPath ?: run {
                Log.d(TAG, "[EDIT:thumb] No thumbnail path for original ${original.localId}, skipping")
                return
            }
            val srcBitmap = android.graphics.BitmapFactory.decodeFile(srcPath) ?: run {
                Log.w(TAG, "[EDIT:thumb] Failed to decode thumbnail at $srcPath")
                return
            }
            Log.d(TAG, "[EDIT:thumb] Source thumbnail: ${srcBitmap.width}×${srcBitmap.height}, path=$srcPath")

            // Parse crop metadata
            val meta = metadata?.let {
                try { org.json.JSONObject(it) } catch (_: Exception) { null }
            }

            val cx = meta?.optDouble("x", 0.0) ?: 0.0
            val cy = meta?.optDouble("y", 0.0) ?: 0.0
            val cw = meta?.optDouble("width", 1.0) ?: 1.0
            val ch = meta?.optDouble("height", 1.0) ?: 1.0
            val brightness = (meta?.optDouble("brightness", 0.0) ?: 0.0).toFloat()
            val rotateDeg = meta?.optInt("rotate", 0) ?: 0

            val paint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG or android.graphics.Paint.FILTER_BITMAP_FLAG)

            // Apply brightness via ColorMatrix
            if (brightness != 0f) {
                val b = brightness / 100f  // -1..1 range
                val cm = android.graphics.ColorMatrix(floatArrayOf(
                    1f, 0f, 0f, 0f, b * 255f,
                    0f, 1f, 0f, 0f, b * 255f,
                    0f, 0f, 1f, 0f, b * 255f,
                    0f, 0f, 0f, 1f, 0f,
                ))
                paint.colorFilter = android.graphics.ColorMatrixColorFilter(cm)
            }

            // 1. Crop the source bitmap to the selected region
            val sx = (cx * srcBitmap.width).toInt().coerceIn(0, srcBitmap.width - 1)
            val sy = (cy * srcBitmap.height).toInt().coerceIn(0, srcBitmap.height - 1)
            val sw = (cw * srcBitmap.width).toInt().coerceAtLeast(1).coerceAtMost(srcBitmap.width - sx)
            val sh = (ch * srcBitmap.height).toInt().coerceAtLeast(1).coerceAtMost(srcBitmap.height - sy)

            // 2. Determine output dimensions after crop + rotation.
            //    For 90°/270° rotations, width and height swap.
            val isSwapped = (rotateDeg == 90 || rotateDeg == 270)
            val croppedW = if (isSwapped) sh else sw
            val croppedH = if (isSwapped) sw else sh

            // 3. Scale to fit within 256px on the longest edge (preserve aspect ratio)
            val maxSize = 256f
            val scale = maxSize / maxOf(croppedW, croppedH).toFloat()
            val outW = (croppedW * scale).toInt().coerceAtLeast(1)
            val outH = (croppedH * scale).toInt().coerceAtLeast(1)

            val output = android.graphics.Bitmap.createBitmap(outW, outH, android.graphics.Bitmap.Config.ARGB_8888)
            val canvas = android.graphics.Canvas(output)

            // 4. Apply rotation via Matrix, then draw scaled crop
            val matrix = android.graphics.Matrix()
            // Scale the cropped region to fit the output
            val scaleX = outW.toFloat() / sw.toFloat()
            val scaleY = outH.toFloat() / sh.toFloat()
            if (rotateDeg != 0) {
                // Translate so that rotation pivot is at center of the cropped region
                // mapped into output space, then rotate, then scale.
                matrix.postTranslate(-sw / 2f, -sh / 2f)
                matrix.postRotate(rotateDeg.toFloat())
                matrix.postScale(
                    outW.toFloat() / (if (isSwapped) sh else sw).toFloat(),
                    outH.toFloat() / (if (isSwapped) sw else sh).toFloat()
                )
                matrix.postTranslate(outW / 2f, outH / 2f)
            } else {
                matrix.postScale(scaleX, scaleY)
            }

            // Extract cropped portion as a new bitmap.
            // createBitmap may share pixel data with the source when the crop
            // covers the full image, so we must NOT recycle srcBitmap until
            // after drawing is complete.
            val cropped = android.graphics.Bitmap.createBitmap(srcBitmap, sx, sy, sw, sh)

            canvas.drawBitmap(cropped, matrix, paint)
            // Now safe to recycle both — drawing is done
            if (cropped !== srcBitmap) cropped.recycle()
            srcBitmap.recycle()

            // Compress to JPEG and save
            val stream = java.io.ByteArrayOutputStream()
            output.compress(android.graphics.Bitmap.CompressFormat.JPEG, 85, stream)
            output.recycle()

            val thumbPath = photoRepository.saveThumbnailToDisk(copyId, stream.toByteArray())
            photoRepository.updateThumbnailPath(copyId, thumbPath)
            Log.d(TAG, "[EDIT:thumb] Generated thumbnail for copy $copyId: ${outW}×${outH}, " +
                "rotate=$rotateDeg, crop=($sx,$sy,${sw}×${sh}), path=$thumbPath")
        } catch (e: Exception) {
            Log.w(TAG, "[EDIT:thumb] Failed to generate edited thumbnail: ${e.message}", e)
        }
    }
}
