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
 * Handles: encrypted blob download & decryption, tags (plain mode), favorites,
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

    var encryptionMode by mutableStateOf("plain")
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
                encryptionMode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }

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

    /** Load tags for a specific photo (called when page changes). */
    fun loadTagsForPhoto(photoId: String?) {
        if (photoId == null || encryptionMode != "plain") return
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) { tagRepository.getPhotoTags(photoId) }
                currentTags = response.tags
                // Also refresh all-tags list
                val tagsResponse = withContext(Dispatchers.IO) { tagRepository.listTags() }
                allTags = tagsResponse.tags
            } catch (_: Exception) {
                currentTags = emptyList()
            }
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

    /** Load favorite state for a specific photo (called when page changes). */
    fun loadFavoriteForPhoto(photoId: String?) {
        if (photoId == null || encryptionMode != "plain") return
        // Read from the local allPhotos list (already loaded from DB) instead
        // of fetching up to 500 DTOs from the server on every page swipe.
        val photo = allPhotos.find { it.serverPhotoId == photoId }
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
        viewModelScope.launch {
            try {
                if (encryptionMode == "plain" && photo.serverPhotoId != null) {
                    // Plain mode: save to server
                    withContext(Dispatchers.IO) {
                        photoRepository.setCropOnServer(photo.serverPhotoId!!, metadata)
                    }
                }
                // Also update local DB
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
                if (encryptionMode == "plain" && photo.serverPhotoId != null) {
                    val response = withContext(Dispatchers.IO) {
                        photoRepository.duplicatePhotoOnServer(photo.serverPhotoId!!, metadata)
                    }
                    val copyEntity = photo.copy(
                        localId = response.id,
                        serverPhotoId = response.id,
                        filename = response.filename,
                        cropMetadata = response.cropMetadata,
                        createdAt = System.currentTimeMillis(),
                        syncStatus = SyncStatus.SYNCED
                    )
                    withContext(Dispatchers.IO) {
                        photoRepository.insertPhoto(copyEntity)
                    }
                }
                // Encrypted mode: duplicate the local DB entry with new ID + metadata
                else {
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
                }
                onDone()
            } catch (_: Exception) {}
        }
    }

    /**
     * Download the photo's full-resolution file and save it to [outputFile].
     *
     * - **Plain mode:** Streams the response body directly to disk via
     *   [PhotoRepository.downloadPlainPhotoToFile], using constant (~8 KB)
     *   heap regardless of file size — safe for multi-hundred-MB videos.
     * - **Encrypted mode:** Downloads + decrypts to file via
     *   [PhotoRepository.downloadAndDecryptBlobToFile].
     *
     * Returns `true` on success, `false` on failure.
     */
    suspend fun downloadPhotoToFile(photo: PhotoEntity, outputFile: java.io.File): Boolean =
        withContext(Dispatchers.IO) {
            try {
                when {
                    encryptionMode == "plain" && photo.serverPhotoId != null -> {
                        photoRepository.downloadPlainPhotoToFile(photo.serverPhotoId!!, outputFile)
                        true
                    }
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
                // Plain mode: stream to temp file, then read — avoids holding
                // the Retrofit response body and the ByteArray simultaneously.
                encryptionMode == "plain" && photo.serverPhotoId != null -> {
                    val tempFile = java.io.File.createTempFile("dl_", ".tmp", photoRepository.getCacheDir())
                    try {
                        photoRepository.downloadPlainPhotoToFile(photo.serverPhotoId!!, tempFile)
                        tempFile.readBytes()
                    } finally {
                        tempFile.delete()
                    }
                }
                photo.serverBlobId != null -> {
                    downloadAndDecrypt(photo.serverBlobId!!)
                }
                else -> null
            }
        } catch (_: Exception) { null }
    }
}
