package com.simplephotos.ui.screens.viewer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.AddTagRequest
import com.simplephotos.data.remote.dto.RemoveTagRequest
import com.simplephotos.data.remote.dto.SetCropRequest
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.PhotoRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import javax.inject.Inject

// ---------------------------------------------------------------------------
// ViewModel — loads photo list for paging + handles deletion
// ---------------------------------------------------------------------------

@HiltViewModel
class PhotoViewerViewModel @Inject constructor(
    private val photoRepository: PhotoRepository,
    private val albumRepository: AlbumRepository,
    private val api: ApiService,
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
     */
    suspend fun downloadAndDecrypt(blobId: String): ByteArray = withContext(Dispatchers.IO) {
        val decrypted = photoRepository.downloadAndDecryptBlob(blobId)
        val payload = JSONObject(String(decrypted, Charsets.UTF_8))
        val dataBase64 = payload.getString("data")
        android.util.Base64.decode(dataBase64, android.util.Base64.NO_WRAP)
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
                val response = withContext(Dispatchers.IO) { api.getPhotoTags(photoId) }
                currentTags = response.tags
                // Also refresh all-tags list
                val tagsResponse = withContext(Dispatchers.IO) { api.listTags() }
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
                withContext(Dispatchers.IO) { api.addTag(photoId, AddTagRequest(cleaned)) }
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
                withContext(Dispatchers.IO) { api.removeTag(photoId, RemoveTagRequest(tag)) }
                currentTags = currentTags.filter { it != tag }
            } catch (_: Exception) {}
        }
    }

    /** Load favorite state for a specific photo (called when page changes). */
    fun loadFavoriteForPhoto(photoId: String?) {
        if (photoId == null || encryptionMode != "plain") return
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) {
                    api.listPhotos(limit = 500)
                }
                val photo = response.photos.find { it.id == photoId }
                isFavorite = photo?.isFavorite ?: false
            } catch (_: Exception) {
                isFavorite = false
            }
        }
    }

    /** Toggle the favorite state of the current photo. */
    fun toggleFavorite(photoId: String) {
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) { api.toggleFavorite(photoId) }
                isFavorite = response.isFavorite
            } catch (_: Exception) {}
        }
    }

    /** Save crop/brightness metadata for a photo. */
    fun saveCropMetadata(photo: PhotoEntity, metadata: String?) {
        viewModelScope.launch {
            try {
                if (encryptionMode == "plain" && photo.serverPhotoId != null) {
                    // Plain mode: save to server
                    withContext(Dispatchers.IO) {
                        api.setCrop(photo.serverPhotoId!!, SetCropRequest(metadata))
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

    /** Download the photo's full-resolution file bytes (for saving to device). */
    suspend fun downloadPhotoBytes(photo: PhotoEntity): ByteArray? = withContext(Dispatchers.IO) {
        try {
            when {
                encryptionMode == "plain" && photo.serverPhotoId != null -> {
                    val response = api.photoFile(photo.serverPhotoId!!)
                    response.bytes()
                }
                photo.serverBlobId != null -> {
                    downloadAndDecrypt(photo.serverBlobId!!)
                }
                else -> null
            }
        } catch (_: Exception) { null }
    }
}
