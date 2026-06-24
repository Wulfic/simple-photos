/**
 * ViewModel for the secure gallery screen, managing encrypted photo loading,
 * decryption, and secure album state.
 */
package com.simplephotos.ui.screens.securegallery

import com.simplephotos.data.decodeThumbEnvelope

import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SecureGalleryRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import javax.inject.Inject

private const val TAG = "SecureGalleryVM"

// ─────────────────────────────────────────────────────────────────────────────
// ViewModel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Manages password-protected secure galleries: unlock with account password,
 * browse galleries, view encrypted items, and add photos.
 */
@HiltViewModel
class SecureGalleryViewModel @Inject constructor(
    private val secureGalleryRepository: SecureGalleryRepository,
    private val photoRepository: PhotoRepository,
    private val albumRepository: AlbumRepository
) : ViewModel() {

    // Auth gate
    var isAuthenticated by mutableStateOf(false)
        private set
    var galleryToken by mutableStateOf("")
        private set
    var authError by mutableStateOf<String?>(null)
        private set
    var authLoading by mutableStateOf(false)
        private set

    // Gallery list
    var galleries by mutableStateOf<List<SecureGallery>>(emptyList())
        private set
    var galleriesLoading by mutableStateOf(false)
        private set

    // Selected gallery detail
    var selectedGallery by mutableStateOf<SecureGallery?>(null)
        private set
    var items by mutableStateOf<List<SecureGalleryItem>>(emptyList())
        private set
    var itemsLoading by mutableStateOf(false)
        private set

    // ── Picker source selection ─────────────────────────────────────────────
    // The "Add Photos" picker can draw from the full library OR a specific
    // album / smart album, mirroring the web flow. `pickerAlbums` are the
    // (id, displayName) sources offered alongside the implicit "All Photos";
    // `pickerPhotos` is the resolved list for the currently-selected source.
    var pickerAlbums by mutableStateOf<List<Pair<String, String>>>(emptyList())
        private set
    var pickerSourceId by mutableStateOf("all")
        private set
    var pickerPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
        private set

    var error by mutableStateOf<String?>(null)
        private set

    fun unlock(password: String) {
        viewModelScope.launch {
            authLoading = true
            authError = null
            try {
                Log.d(TAG, "Unlocking secure galleries…")
                val res = withContext(Dispatchers.IO) {
                    secureGalleryRepository.unlock(password)
                }
                galleryToken = res.galleryToken
                // Publish to the network holder so the auth interceptor attaches
                // X-Gallery-Token to secure-album media requests (photos/blobs).
                com.simplephotos.data.remote.GalleryTokenHolder.token = res.galleryToken
                isAuthenticated = true
                Log.i(TAG, "Unlock successful, token obtained")
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Unlock failed", e)
                authError = e.message ?: "Invalid password"
            } finally {
                authLoading = false
            }
        }
    }

    fun loadGalleries() {
        viewModelScope.launch {
            galleriesLoading = true
            try {
                val res = withContext(Dispatchers.IO) { secureGalleryRepository.listGalleries() }
                galleries = res.galleries
                Log.d(TAG, "Loaded ${galleries.size} galleries")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to load galleries", e)
                error = "Failed to load galleries: ${e.message}"
            } finally {
                galleriesLoading = false
            }
        }
    }

    fun selectGallery(gallery: SecureGallery) {
        Log.d(TAG, "selectGallery: id=${gallery.id} name=${gallery.name} items=${gallery.itemCount}")
        selectedGallery = gallery
        loadItems(gallery.id)
        loadPhotos()
    }

    fun deselectGallery() {
        selectedGallery = null
        items = emptyList()
    }

    private fun loadItems(galleryId: String) {
        viewModelScope.launch {
            itemsLoading = true
            try {
                val res = withContext(Dispatchers.IO) {
                    secureGalleryRepository.listItems(galleryId, galleryToken)
                }
                items = res.items
                Log.d(TAG, "Loaded ${items.size} items for gallery $galleryId")
                for (item in items) {
                    Log.d(TAG, "  item id=${item.id} blobId=${item.blobId} " +
                        "thumbBlobId=${item.encryptedThumbBlobId} " +
                        "w=${item.width} h=${item.height} type=${item.mediaType}")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to load items for gallery $galleryId", e)
                error = "Failed to load items: ${e.message}"
            } finally {
                itemsLoading = false
            }
        }
    }

    private fun loadPhotos() {
        viewModelScope.launch {
            try {
                val all = withContext(Dispatchers.IO) {
                    photoRepository.getAllPhotos().first()
                }
                // Default the picker to the full library each time an album opens.
                pickerSourceId = "all"
                pickerPhotos = all
                Log.d(TAG, "Loaded ${all.size} photos for picker")

                // Offer album sources: the user's own albums first, then the
                // smart albums that actually have content (skip empty ones so the
                // chip row stays uncluttered).
                val userAlbums = withContext(Dispatchers.IO) {
                    albumRepository.getAllAlbums().first()
                }
                val sources = mutableListOf<Pair<String, String>>()
                for (a in userAlbums) sources.add(a.localId to a.name)
                if (all.isNotEmpty()) sources.add("smart-recents" to "Recently Added")
                if (all.any { it.isFavorite }) sources.add("smart-favorites" to "Favorites")
                if (all.any { it.mediaType == "video" }) sources.add("smart-videos" to "Videos")
                pickerAlbums = sources
            } catch (e: Exception) {
                Log.w(TAG, "Failed to load photos for picker", e)
            }
        }
    }

    /**
     * Switch the "Add Photos" picker to a different source — the full library
     * ("all") or a specific album / smart album id. Resolves the photo list off
     * the main thread; [PhotoRepository.getAlbumPhotos] handles both real and
     * smart albums.
     */
    fun selectPickerSource(sourceId: String) {
        pickerSourceId = sourceId
        viewModelScope.launch {
            try {
                pickerPhotos = withContext(Dispatchers.IO) {
                    if (sourceId == "all") photoRepository.getAllPhotos().first()
                    else photoRepository.getAlbumPhotos(sourceId)
                }
                Log.d(TAG, "Picker source '$sourceId' → ${pickerPhotos.size} photos")
            } catch (e: Exception) {
                Log.w(TAG, "Failed to load picker source $sourceId", e)
                pickerPhotos = emptyList()
            }
        }
    }

    /**
     * Fetch a cover thumbnail (most-recent item) for a gallery, for the
     * album-list preview. Lazy + per-card: each card calls this in a
     * LaunchedEffect so only visible galleries fetch. Returns the decrypted
     * thumbnail bytes, or null if the gallery is empty / fetch fails.
     */
    suspend fun fetchGalleryCover(galleryId: String): ByteArray? = withContext(Dispatchers.IO) {
        try {
            val res = secureGalleryRepository.listItems(galleryId, galleryToken)
            val first = res.items.firstOrNull() ?: return@withContext null
            downloadThumb(first.blobId, first.encryptedThumbBlobId)
        } catch (e: Exception) {
            Log.w(TAG, "fetchGalleryCover failed for $galleryId", e)
            null
        }
    }

    fun createGallery(name: String) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    secureGalleryRepository.createGallery(name)
                }
                Log.i(TAG, "Created gallery: $name")
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Create gallery failed: $name", e)
                error = "Create failed: ${e.message}"
            }
        }
    }

    fun deleteGallery(gallery: SecureGallery) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { secureGalleryRepository.deleteGallery(gallery.id) }
                Log.i(TAG, "Deleted gallery: ${gallery.id}")
                if (selectedGallery?.id == gallery.id) {
                    selectedGallery = null
                    items = emptyList()
                }
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Delete gallery failed: ${gallery.id}", e)
                error = "Delete failed: ${e.message}"
            }
        }
    }

    fun addPhotosToGallery(blobIds: List<String>) {
        val gallery = selectedGallery ?: return
        Log.d(TAG, "addPhotosToGallery: ${blobIds.size} blobs → gallery ${gallery.id}")
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    coroutineScope {
                        blobIds.map { blobId ->
                            async {
                                Log.d(TAG, "  adding blobId=$blobId")
                                secureGalleryRepository.addItem(gallery.id, blobId)
                            }
                        }.awaitAll()
                    }
                }
                Log.i(TAG, "Added ${blobIds.size} photos to gallery ${gallery.id}")
                loadItems(gallery.id)
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Add photos failed", e)
                error = "Add photos failed: ${e.message}"
            }
        }
    }

    /**
     * Remove a single item from the selected secure gallery. The original
     * photo becomes visible again in the regular gallery (mirrors the web's
     * per-item removal).
     */
    fun removeItem(item: SecureGalleryItem) {
        val gallery = selectedGallery ?: return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    secureGalleryRepository.removeItem(gallery.id, item.id)
                }
                Log.i(TAG, "Removed item ${item.id} from gallery ${gallery.id}")
                loadItems(gallery.id)
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Remove item failed: ${item.id}", e)
                error = "Remove failed: ${e.message}"
            }
        }
    }

    /**
     * Decrypt a secure blob to a fresh temp file and return it (kept on disk).
     *
     * Videos and motion-photo trailers can't be handed to ExoPlayer as a
     * ByteArray — it needs a file/URI — and streaming the decrypt to disk
     * avoids holding a whole video in heap (OOM). The caller MUST delete the
     * returned file when done (e.g. in a DisposableEffect) so the decrypted
     * plaintext doesn't linger in the cache dir (confidentiality).
     */
    suspend fun downloadAndDecryptToFile(blobId: String, suffix: String): File = withContext(Dispatchers.IO) {
        Log.d(TAG, "downloadAndDecryptToFile: blobId=$blobId suffix=$suffix")
        val out = File.createTempFile("secure_play_", ".$suffix", photoRepository.getCacheDir())
        try {
            photoRepository.downloadAndDecryptBlobToFile(blobId, out)
            Log.d(TAG, "downloadAndDecryptToFile: blobId=$blobId → ${out.length()} bytes")
            out
        } catch (e: Exception) {
            Log.e(TAG, "downloadAndDecryptToFile failed: blobId=$blobId", e)
            out.delete()
            throw e
        }
    }

    /** Cache dir for ephemeral decrypted media (motion-video trailers, etc.). */
    fun cacheDir(): File = photoRepository.getCacheDir()

    /**
     * Download and decrypt an encrypted blob, returning the raw image bytes.
     *
     * Uses file-based streaming to avoid OOM on large blobs:
     * encrypted blob → temp file → decrypt → stream-extract base64 → output file → read.
     */
    suspend fun downloadAndDecrypt(blobId: String): ByteArray = withContext(Dispatchers.IO) {
        Log.d(TAG, "downloadAndDecrypt: blobId=$blobId")
        val outputFile = File.createTempFile("secure_dec_", ".tmp", photoRepository.getCacheDir())
        try {
            photoRepository.downloadAndDecryptBlobToFile(blobId, outputFile)
            val bytes = outputFile.readBytes()
            Log.d(TAG, "downloadAndDecrypt: blobId=$blobId → ${bytes.size} bytes decoded")
            bytes
        } catch (e: Exception) {
            Log.e(TAG, "downloadAndDecrypt failed: blobId=$blobId", e)
            throw e
        } finally {
            outputFile.delete()
        }
    }

    /**
     * Download a small encrypted thumbnail for a secure gallery item.
     *
     * Resolution order:
     * 1. If [encryptedThumbBlobId] is provided (from server item metadata),
     *    download that blob directly — it's a small (~30 KB) thumbnail.
     * 2. Otherwise fall back to `GET /api/blobs/{blobId}/thumb` which asks
     *    the server to resolve the thumbnail from the photos table.
     * 3. Last resort: download the full blob (may be large).
     */
    suspend fun downloadThumb(blobId: String, encryptedThumbBlobId: String? = null): ByteArray = withContext(Dispatchers.IO) {
        // 1. Direct thumbnail blob download (most reliable for cloned/Android items)
        if (encryptedThumbBlobId != null) {
            Log.d(TAG, "downloadThumb: using encryptedThumbBlobId=$encryptedThumbBlobId for blobId=$blobId")
            try {
                val thumbBytes = photoRepository.downloadAndDecryptBlob(encryptedThumbBlobId)
                decodeThumbEnvelope(thumbBytes)?.let { decoded ->
                    Log.d(TAG, "downloadThumb: direct thumb success, ${decoded.size} bytes")
                    return@withContext decoded
                }
            } catch (e: Exception) {
                Log.w(TAG, "downloadThumb: direct thumb download failed for $encryptedThumbBlobId, trying /thumb endpoint", e)
            }
        }

        // 2. Server-resolved thumbnail endpoint
        Log.d(TAG, "downloadThumb: trying /blobs/$blobId/thumb endpoint")
        try {
            val thumbBytes = photoRepository.downloadAndDecryptThumbBlob(blobId)
            if (thumbBytes != null) {
                decodeThumbEnvelope(thumbBytes)?.let { decoded ->
                    Log.d(TAG, "downloadThumb: /thumb endpoint success, ${decoded.size} bytes")
                    return@withContext decoded
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "downloadThumb: /thumb endpoint failed for $blobId", e)
        }

        // 3. Fallback: download full blob via streaming (avoids OOM)
        Log.w(TAG, "downloadThumb: falling back to full blob download for $blobId")
        downloadAndDecrypt(blobId)
    }
}
