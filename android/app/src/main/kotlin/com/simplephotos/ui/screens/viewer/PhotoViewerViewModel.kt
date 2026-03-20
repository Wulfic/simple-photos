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
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
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

    /** Tags for the currently viewed photo (plain mode only). */
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
        // Tags are not supported in always-encrypted mode
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

    /** Load favorite state for a specific photo — no-op in encrypted mode. */
    fun loadFavoriteForPhoto(photoId: String?) {
        // Favorites are not supported in always-encrypted mode
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
        viewModelScope.launch {
            try {
                // Update local DB
                withContext(Dispatchers.IO) {
                    photoRepository.updateCropMetadata(photo.localId, metadata)
                }
                // Update in-memory list
                val idx = allPhotos.indexOfFirst { it.localId == photo.localId }
                if (idx >= 0) {
                    allPhotos = allPhotos.toMutableList().also {
                        it[idx] = it[idx].copy(cropMetadata = metadata)
                    }
                }
            } catch (_: Exception) {}
        }
    }

    /** Duplicate a photo with optional crop/edit metadata (Save Copy). */
    fun duplicatePhoto(photo: PhotoEntity, metadata: String?, onDone: () -> Unit = {}) {
        viewModelScope.launch {
            try {
                // Duplicate the local DB entry with new ID + metadata
                val copyId = java.util.UUID.randomUUID().toString()

                // Copy the local file if available, handling both content:// URIs
                // and regular filesystem paths via PhotoRepository.copyLocalFile().
                var newLocalPath: String? = null
                photo.localPath?.let { oldPath ->
                    val cacheDir = withContext(Dispatchers.IO) {
                        photoRepository.getCacheDir()
                    }
                    val ext = photo.filename.substringAfterLast('.', "jpg")
                    val destFile = java.io.File(cacheDir, "copy_${copyId}.$ext")
                    newLocalPath = withContext(Dispatchers.IO) {
                        photoRepository.copyLocalFile(oldPath, destFile)
                    }
                }

                val copyEntity = photo.copy(
                    localId = copyId,
                    serverPhotoId = null,
                    filename = if (photo.filename.startsWith("Copy of ")) photo.filename
                               else "Copy of ${photo.filename}",
                    cropMetadata = metadata,
                    createdAt = System.currentTimeMillis(),
                    localPath = newLocalPath,
                    syncStatus = SyncStatus.PENDING
                )
                withContext(Dispatchers.IO) {
                    photoRepository.insertPhoto(copyEntity)
                }

                // Generate an edited thumbnail for the copy so the gallery
                // reflects the crop/brightness/rotation at a glance.
                withContext(Dispatchers.IO) {
                    generateEditedThumbnail(photo, copyId, metadata)
                }
                onDone()
            } catch (_: Exception) {}
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
            val srcPath = original.thumbnailPath ?: return
            val srcBitmap = android.graphics.BitmapFactory.decodeFile(srcPath) ?: return

            // Parse crop metadata
            val meta = metadata?.let {
                try { org.json.JSONObject(it) } catch (_: Exception) { null }
            }

            val cx = meta?.optDouble("x", 0.0) ?: 0.0
            val cy = meta?.optDouble("y", 0.0) ?: 0.0
            val cw = meta?.optDouble("width", 1.0) ?: 1.0
            val ch = meta?.optDouble("height", 1.0) ?: 1.0
            val brightness = (meta?.optDouble("brightness", 0.0) ?: 0.0).toFloat()
            val rotate = (meta?.optInt("rotate", 0) ?: 0).toFloat()

            val SIZE = 256
            val output = android.graphics.Bitmap.createBitmap(SIZE, SIZE, android.graphics.Bitmap.Config.ARGB_8888)
            val canvas = android.graphics.Canvas(output)
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

            // Source rectangle (crop region on original thumbnail)
            val sx = (cx * srcBitmap.width).toInt()
            val sy = (cy * srcBitmap.height).toInt()
            val sw = (cw * srcBitmap.width).toInt().coerceAtLeast(1)
            val sh = (ch * srcBitmap.height).toInt().coerceAtLeast(1)
            val srcRect = android.graphics.Rect(sx, sy, sx + sw, sy + sh)

            // Fit cropped region into SIZE×SIZE
            val scale = maxOf(SIZE.toFloat() / sw, SIZE.toFloat() / sh)
            val dw = sw * scale
            val dh = sh * scale
            val dstRect = android.graphics.RectF(
                (SIZE - dw) / 2f, (SIZE - dh) / 2f,
                (SIZE + dw) / 2f, (SIZE + dh) / 2f,
            )

            canvas.save()
            if (rotate != 0f) {
                canvas.rotate(rotate, SIZE / 2f, SIZE / 2f)
            }
            canvas.drawBitmap(srcBitmap, srcRect, dstRect, paint)
            canvas.restore()
            srcBitmap.recycle()

            // Compress to JPEG and save
            val stream = java.io.ByteArrayOutputStream()
            output.compress(android.graphics.Bitmap.CompressFormat.JPEG, 85, stream)
            output.recycle()

            val thumbPath = photoRepository.saveThumbnailToDisk(copyId, stream.toByteArray())
            photoRepository.updateThumbnailPath(copyId, thumbPath)
        } catch (e: Exception) {
            android.util.Log.w("PhotoViewerVM", "Failed to generate edited thumbnail: ${e.message}")
        }
    }
}
