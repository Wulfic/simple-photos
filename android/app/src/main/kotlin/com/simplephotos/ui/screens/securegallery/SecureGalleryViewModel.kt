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
import com.simplephotos.data.SecureSmartAlbum
import com.simplephotos.data.SecureSmartAlbums
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

/**
 * A selectable source shown in the secure "Add Photos" album browser
 * (the "Albums" tab). [coverThumbPath] is an optional cached-thumbnail file
 * path for a small cover preview; null falls back to a folder icon.
 */
data class PickerAlbum(
    val id: String,
    val name: String,
    val coverThumbPath: String? = null,
)

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

    // ── Secure smart albums (built-in, media-type derived) ──────────────────
    // The aggregate feed across ALL secure galleries, fetched once after unlock
    // and refreshed on every mutation. Smart-album membership derives from it.
    var allItems by mutableStateOf<List<SecureGalleryItem>>(emptyList())
        private set
    var allItemsLoading by mutableStateOf(false)
        private set
    // When a smart album is open, `selectedGallery` is null and this holds the
    // smart id (a synthetic selection — never a fake SecureGallery instance, so
    // smart ids can't leak into deleteGallery/addPhotosToGallery).
    var selectedSmartAlbumId by mutableStateOf<String?>(null)
        private set

    /** Non-empty secure smart albums, recomputed from the aggregate feed. */
    val smartAlbums: List<SecureSmartAlbum>
        get() = SecureSmartAlbums.compute(allItems)

    // ── Picker source selection ─────────────────────────────────────────────
    // The "Add Photos" picker can draw from the full library OR a specific
    // album / smart album, mirroring the web flow. `pickerAlbums` are the
    // (id, displayName) sources offered alongside the implicit "All Photos";
    // `pickerPhotos` is the resolved list for the currently-selected source.
    var pickerAlbums by mutableStateOf<List<PickerAlbum>>(emptyList())
        private set
    var pickerSourceId by mutableStateOf("all")
        private set
    var pickerPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
        private set

    // Blob/photo/encrypted IDs that already live in ANY secure gallery. The
    // picker hides these so a photo already secured can't be added a second
    // time — same set the main gallery uses to hide secured originals.
    var secureBlobIds by mutableStateOf<Set<String>>(emptySet())
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
                loadAllItems()
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

    /**
     * Load the aggregate item feed for the secure smart albums. If a smart album
     * is currently open, its item list is re-derived from the fresh feed so a
     * mutation (remove) reflects immediately.
     */
    fun loadAllItems() {
        viewModelScope.launch {
            allItemsLoading = true
            try {
                val res = withContext(Dispatchers.IO) {
                    secureGalleryRepository.listAllItems(galleryToken)
                }
                allItems = res.items
                Log.d(TAG, "Loaded ${allItems.size} aggregate secure items")
                selectedSmartAlbumId?.let { items = SecureSmartAlbums.filter(allItems, it) }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to load aggregate secure items", e)
                error = "Failed to load secure albums: ${e.message}"
            } finally {
                allItemsLoading = false
            }
        }
    }

    fun selectGallery(gallery: SecureGallery) {
        Log.d(TAG, "selectGallery: id=${gallery.id} name=${gallery.name} items=${gallery.itemCount}")
        selectedSmartAlbumId = null
        selectedGallery = gallery
        loadItems(gallery.id)
        loadPhotos()
    }

    /**
     * Open a built-in secure smart album. No per-gallery fetch and no picker
     * prep (smart views are read-only) — items derive from the aggregate feed.
     */
    fun selectSmartAlbum(id: String) {
        Log.d(TAG, "selectSmartAlbum: id=$id")
        selectedGallery = null
        selectedSmartAlbumId = id
        items = SecureSmartAlbums.filter(allItems, id)
    }

    fun deselectGallery() {
        selectedGallery = null
        selectedSmartAlbumId = null
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

                // Photos already in ANY secure gallery must not be offered for
                // re-adding. This is the same id set the main gallery filters on
                // (see GalleryScreen) — originals, clones, and encrypted blobs.
                secureBlobIds = withContext(Dispatchers.IO) {
                    secureGalleryRepository.getSecureBlobIds()
                }
                Log.d(TAG, "Loaded ${secureBlobIds.size} secure blob ids for dedup")

                // Build the "Albums" browser sources: the user's own albums, then
                // the non-empty smart albums (Recently Added is its own top-level
                // chip, so it's intentionally not duplicated here).
                val byLocalId = all.associateBy { it.localId }
                val userAlbums = withContext(Dispatchers.IO) {
                    albumRepository.getAllAlbums().first()
                }
                val sources = mutableListOf<PickerAlbum>()
                for (a in userAlbums) {
                    val cover = a.coverPhotoLocalId?.let { byLocalId[it]?.thumbnailPath }
                    sources.add(PickerAlbum(a.localId, a.name, cover))
                }
                if (all.any { it.isFavorite }) {
                    sources.add(PickerAlbum("smart-favorites", "Favorites",
                        all.firstOrNull { it.isFavorite }?.thumbnailPath))
                }
                if (all.any { it.mediaType == "video" }) {
                    sources.add(PickerAlbum("smart-videos", "Videos",
                        all.firstOrNull { it.mediaType == "video" }?.thumbnailPath))
                }
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
                loadAllItems()
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
            // Secure each photo independently. Previously one failing addItem made
            // awaitAll() rethrow and cancel every sibling coroutine, aborting the
            // whole batch — some photos moved, the rest stayed in the regular
            // gallery ("most items removed but not all", #16). Each job now catches
            // its own failure and reports success/failure so the batch always
            // finishes and the user is told when photos are left behind.
            val outcomes = withContext(Dispatchers.IO) {
                coroutineScope {
                    blobIds.map { blobId ->
                        async {
                            try {
                                secureGalleryRepository.addItem(gallery.id, blobId)
                                true
                            } catch (e: Exception) {
                                Log.e(TAG, "  add failed for blobId=$blobId", e)
                                false
                            }
                        }
                    }.awaitAll()
                }
            }
            val added = outcomes.count { it }
            val failed = outcomes.size - added
            Log.i(TAG, "Added $added/${blobIds.size} photos to gallery ${gallery.id} ($failed failed)")
            if (failed > 0) {
                error = "$failed photo${if (failed != 1) "s" else ""} couldn't be secured and remain in your gallery"
            }
            loadItems(gallery.id)
            loadGalleries()
            loadAllItems()
        }
    }

    // ── Cross-secure-album move picker (#31) ────────────────────────────────
    // Items in the user's OTHER secure albums — the source pool for moving media
    // into the open album. Excludes the open album's own items and any item with
    // no owning gallery id (older rows we can't route a move for).
    val otherSecureItems: List<SecureGalleryItem>
        get() {
            val current = selectedGallery?.id ?: return emptyList()
            return allItems.filter { it.galleryId != null && it.galleryId != current }
        }

    /**
     * Move the given items (by id) from their current secure albums into the
     * open album (#31). Each is isolated so one failure never aborts the rest.
     */
    fun moveItemsIntoSelected(itemIds: List<String>) {
        val target = selectedGallery ?: return
        if (itemIds.isEmpty()) return
        val pool = otherSecureItems.associateBy { it.id }
        viewModelScope.launch {
            val outcomes = withContext(Dispatchers.IO) {
                coroutineScope {
                    itemIds.mapNotNull { id ->
                        val item = pool[id]
                        val src = item?.galleryId
                        if (src == null) {
                            Log.e(TAG, "  cannot move item $id: not in pool / no source gallery")
                            null
                        } else {
                            async {
                                try {
                                    secureGalleryRepository.moveItem(src, id, target.id)
                                    true
                                } catch (e: Exception) {
                                    Log.e(TAG, "  move failed for item $id", e)
                                    false
                                }
                            }
                        }
                    }.awaitAll()
                }
            }
            val moved = outcomes.count { it }
            val failed = itemIds.size - moved
            Log.i(TAG, "Moved $moved/${itemIds.size} items into ${target.id} ($failed failed)")
            if (failed > 0) {
                error = "$failed item${if (failed != 1) "s" else ""} couldn't be moved"
            }
            loadItems(target.id)
            loadGalleries()
            loadAllItems()
        }
    }

    /**
     * Persist (or clear, with null) a secure item's crop/edit metadata (#31),
     * then refresh so the tile + viewer re-render with the applied crop.
     */
    fun setItemCrop(item: SecureGalleryItem, cropMetadata: String?, onDone: () -> Unit = {}) {
        val gid = item.galleryId ?: selectedGallery?.id
        if (gid == null) {
            Log.e(TAG, "setItemCrop: no owning gallery for item ${item.id}")
            error = "Could not determine which album this photo belongs to."
            return
        }
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    secureGalleryRepository.setItemCrop(gid, item.id, cropMetadata)
                }
                Log.i(TAG, "Saved crop for secure item ${item.id} (hasCrop=${cropMetadata != null})")
                loadAllItems()
                selectedGallery?.let { loadItems(it.id) }
            } catch (e: Exception) {
                Log.e(TAG, "setItemCrop failed for ${item.id}", e)
                error = "Save failed: ${e.message}"
            } finally {
                onDone()
            }
        }
    }

    /**
     * Remove a single item (or whole burst) from the selected secure gallery.
     * Delegates to [removeItems] so burst stacks are removed in full.
     */
    fun removeItem(item: SecureGalleryItem) = removeItems(listOf(item))

    /**
     * Remove items from the selected secure gallery, returning their originals
     * to the regular gallery (mirrors the web's per-item removal).
     *
     * Burst-aware: any target that belongs to a burst pulls in ALL sibling
     * frames sharing its `burst_id`. The grid and viewer collapse a burst to a
     * single tile/page, so a naive single-item delete would strand the other
     * frames in the album while only the cover returned to the gallery — the
     * exact bug this fixes.
     */
    fun removeItems(targets: List<SecureGalleryItem>) {
        if (targets.isEmpty()) return
        // Route each item to its OWNING album: in a smart view `selectedGallery`
        // is null and items span multiple galleries, so we can't assume one
        // target gallery. Fall back to the selected real gallery when an item
        // predates gallery_id (older server).
        val fallbackGalleryId = selectedGallery?.id
        // Burst-aware, but gallery-SCOPED: siblings must share BOTH the burst_id
        // and the owning gallery. Frames of one burst live in one album, so this
        // just guards against a hypothetical cross-album burst_id collision.
        val burstKeys = targets
            .filter { !it.burstId.isNullOrEmpty() }
            .map { it.galleryId to it.burstId }
            .toSet()
        val siblings = items.filter {
            !it.burstId.isNullOrEmpty() && (it.galleryId to it.burstId) in burstKeys
        }
        val toRemove = (targets + siblings).distinctBy { it.id }
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    coroutineScope {
                        toRemove.mapNotNull { item ->
                            val gid = item.galleryId ?: fallbackGalleryId
                            if (gid == null) {
                                Log.e(TAG, "  cannot remove item ${item.id}: no owning gallery id")
                                null
                            } else {
                                async { secureGalleryRepository.removeItem(gid, item.id) }
                            }
                        }.awaitAll()
                    }
                }
                Log.i(TAG, "Removed ${toRemove.size} item(s) from secure album(s)")
                // Refresh the aggregate feed (re-derives smart items) and the
                // album list; real albums also re-fetch their own items.
                loadAllItems()
                loadGalleries()
                selectedGallery?.let { loadItems(it.id) }
            } catch (e: Exception) {
                Log.e(TAG, "Remove items failed (${toRemove.size} targets)", e)
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
