/**
 * ViewModel that manages state and actions for the full-screen photo viewer.
 */
package com.simplephotos.ui.screens.viewer

import com.simplephotos.data.decodeThumbEnvelope

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.album.AlbumPhotoResolver
import com.simplephotos.data.album.VIEWER_PHOTO_IDS_KEY
import com.simplephotos.data.album.pageIndexOf
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.dto.FullMetadataResponse
import com.simplephotos.data.remote.dto.MetadataUpdateRequest
import com.simplephotos.data.repository.AiRepository
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.TagRepository
import com.simplephotos.data.remote.dto.PhotoFace
import android.util.Log
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
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
    private val aiRepository: AiRepository,
    private val resolver: AlbumPhotoResolver,
    val okHttpClient: OkHttpClient,
    savedStateHandle: SavedStateHandle
) : ViewModel() {

    companion object {
        private const val TAG = "PhotoViewerVM"
    }

    private val initialPhotoId: String = savedStateHandle["photoId"] ?: ""

    /** Album context — non-null when viewer was opened from an album. */
    val albumId: String? = savedStateHandle["albumId"]

    /**
     * The launching grid's own resolved order, when it had one (E3a).
     *
     * Set by `NavGraph` from the previous back-stack entry's `SavedStateHandle`
     * before this ViewModel is constructed — nav arguments cannot carry a list,
     * and this is Compose Navigation's channel for one. Non-null only for
     * Search / People / Pets / Memories / Trips, whose grids come from server
     * endpoints and therefore have an order neither the album branch nor the
     * gallery branch can reproduce. Null for the gallery and for albums, which
     * the resolver already resolves identically to their grids (E3).
     */
    private val handoffPhotoIds: List<String>? =
        savedStateHandle.get<ArrayList<String>>(VIEWER_PHOTO_IDS_KEY)?.takeIf { it.isNotEmpty() }

    /**
     * The list the pager swipes through — [ResolvedPhotos.photos], which is
     * *the same list object the launching grid renders*, not a re-derivation of
     * it (E3). Secure-excluded, burst policy applied per album kind, ordered by
     * the album's persisted #52 sort.
     *
     * In the main gallery's context that means one entry per burst group, so a
     * 46-shot burst occupies a single page instead of hijacking 46 swipes;
     * individual frames are browsed via the burst filmstrip ([burstFramesFor]).
     * In a regular album it means every frame, because the grid shows every
     * frame there — matching web, whose regular albums stay faithful to the
     * manifest the user built. On a handoff surface ([handoffPhotoIds]) it means
     * the grid's own order with the secure exclusion applied and nothing else,
     * because the grid already decided the rest.
     */
    var allPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
        private set

    /**
     * Membership before burst collapse ([ResolvedPhotos.members]). Used only to
     * resolve a stack's frames for the filmstrip without a network round-trip.
     *
     * NOT the source [allPhotos] is derived from — deriving one list from the
     * other in here is exactly the defect E3 fixed. Both come from one resolver
     * call, and the mutation helpers below edit both in step.
     */
    private var allPhotosRaw: List<PhotoEntity> = emptyList()

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

    // ── Info-panel metadata (full EXIF view + edit) ──────────────────────
    /** Full metadata + raw EXIF for the currently viewed photo. Null until
     *  loaded (or if the load failed — the panel then shows local fields). */
    var fullMetadata by mutableStateOf<FullMetadataResponse?>(null)
        private set

    /** True while a metadata save / EXIF write is in flight. */
    var metadataSaving by mutableStateOf(false)
        private set

    /** Last metadata save error, shown in the Info-panel editor. */
    var metadataError by mutableStateOf<String?>(null)
        private set

    /** Faces detected in the currently viewed photo (for the People section of
     *  the Info panel). Empty until loaded / when AI is off or none detected. */
    var photoFaces by mutableStateOf<List<PhotoFace>>(emptyList())
        private set

    init {
        loadPhotos()
    }

    private fun loadPhotos() {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }

                // ONE resolver call, shared with the grid that launched us. Two
                // shapes, never two derivations:
                //
                //  • The grid handed its list over (Search / People / Pets /
                //    Memories / Trips, E3a) — page exactly that, secure-filtered
                //    and nothing else. Their order comes from a server endpoint;
                //    re-deriving it here is impossible, which is why they used
                //    to page the gallery's order instead of their own.
                //  • It did not (gallery, albums) — the resolver rebuilds the
                //    identical list from the identical inputs, applying the
                //    secure exclusion, the per-kind burst policy and the album's
                //    persisted #52 sort (E3).
                val resolved = handoffPhotoIds
                    ?.let { resolver.resolveExplicit(it) }
                    ?: resolver.resolve(albumId)
                allPhotosRaw = resolved.members
                allPhotos = resolved.photos
                val page = resolved.pageIndexOf(initialPhotoId)
                if (page < 0) {
                    // Not on this surface — secured, or not in the local mirror
                    // (the non-album grids resolve from server endpoints and
                    // draw thumbnails straight from the server, so they can show
                    // a tile for a photo this device has never mirrored and hand
                    // over its raw server id). Render "Photo not found" rather
                    // than opening page 0, which would silently show an
                    // unrelated photo and, for a secured id, would be the exact
                    // leak the secure exclusion above just closed.
                    Log.w(TAG, "[list] photo $initialPhotoId is not in the resolved " +
                        "list for album=$albumId handoff=${handoffPhotoIds?.size} " +
                        "(${allPhotos.size} items)")
                    allPhotos = emptyList()
                    initialPage = 0
                } else {
                    initialPage = page
                }
            } catch (e: Exception) {
                error = e.message
            } finally {
                listLoading = false
            }
        }
    }

    /**
     * Apply an in-place edit to the loaded lists.
     *
     * Edits BOTH lists rather than mutating one and re-deriving the other: the
     * derivation is the resolver's, it needs inputs this ViewModel no longer
     * holds (the secure set, the persisted sort), and re-running it here would
     * reintroduce the second derivation E3 removed. A field edit changes no
     * membership and no order, so touching both in step is also the cheaper and
     * more obviously correct operation.
     */
    private fun updateLoadedPhotos(
        match: (PhotoEntity) -> Boolean,
        transform: (PhotoEntity) -> PhotoEntity,
    ) {
        allPhotosRaw = allPhotosRaw.map { if (match(it)) transform(it) else it }
        allPhotos = allPhotos.map { if (match(it)) transform(it) else it }
    }

    /** Drop a photo from the loaded lists after it leaves the album. Same
     *  both-lists-in-step rule as [updateLoadedPhotos]. */
    private fun removeLoadedPhoto(localId: String) {
        allPhotosRaw = allPhotosRaw.filter { it.localId != localId }
        allPhotos = allPhotos.filter { it.localId != localId }
    }

    /**
     * All frames belonging to [burstId], ordered oldest-first (taken_at ASC,
     * matching the server's `GET /api/photos/burst/{id}` ordering and the web
     * BurstStrip). Resolved entirely from the locally-loaded list — no network.
     */
    fun burstFramesFor(burstId: String): List<PhotoEntity> =
        allPhotosRaw.filter { it.burstId == burstId }
            .sortedBy { it.takenAt }

    /**
     * Download and decrypt an encrypted blob, returning the raw media bytes.
     * Called from per-page composables for encrypted-mode photos.
     *
     * Format-aware: handles both the v1 monolithic envelope (base64 `data` in
     * JSON) and the v2 chunked container (large files) — see [ChunkedBlob].
     */
    suspend fun downloadAndDecrypt(blobId: String): ByteArray = withContext(Dispatchers.IO) {
        photoRepository.downloadAndDecryptMediaBytes(blobId)
    }

    /**
     * The streaming source backing encrypted-video playback (issue #17). The
     * repository implements it; the viewer builds a [MediaBlobDataSource.Factory]
     * from it so ExoPlayer fetches + decrypts only the frames it reads.
     */
    val encryptedBlobStream: com.simplephotos.data.repository.EncryptedBlobStream
        get() = photoRepository

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

    /**
     * Download the embedded motion-photo video for [photoId] to [outputFile].
     * The server returns a ready-to-play MP4 (extracted/decrypted server-side),
     * so this is a plain stream-to-file — no decryption. See
     * [PhotoRepository.downloadMotionVideoToFile].
     */
    suspend fun downloadMotionVideoToFile(photoId: String, outputFile: java.io.File) = withContext(Dispatchers.IO) {
        photoRepository.downloadMotionVideoToFile(photoId, outputFile)
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
                // Remove from in-memory lists and navigate
                removeLoadedPhoto(photo.localId)
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
                // Update the loaded lists so loadFavoriteForPhoto() returns the
                // correct value when the user swipes away and back.
                updateLoadedPhotos({ it.serverPhotoId == photoId }) {
                    it.copy(isFavorite = response.isFavorite)
                }
            } catch (_: Exception) {}
        }
    }

    // ── Info-panel metadata operations ───────────────────────────────────

    /** Load full metadata (+ raw EXIF) for the Info panel. Best-effort: on
     *  failure [fullMetadata] is left null and the panel shows local fields. */
    fun loadFullMetadata(serverPhotoId: String) {
        viewModelScope.launch {
            metadataError = null
            try {
                val meta = withContext(Dispatchers.IO) { photoRepository.getFullMetadata(serverPhotoId) }
                fullMetadata = meta
            } catch (e: Exception) {
                Log.w(TAG, "[meta] loadFullMetadata failed for $serverPhotoId: ${e.message}")
            }
        }
    }

    /** Reset Info-panel metadata state when switching photos / closing. */
    fun clearFullMetadata() {
        fullMetadata = null
        metadataError = null
    }

    /** Load the faces detected in a photo for the Info-panel People section.
     *  Best-effort: on failure (AI off / none) [photoFaces] is left empty. */
    fun loadPhotoFaces(serverPhotoId: String) {
        viewModelScope.launch {
            try {
                photoFaces = withContext(Dispatchers.IO) { aiRepository.listPhotoFaces(serverPhotoId) }
            } catch (e: Exception) {
                Log.w(TAG, "[faces] loadPhotoFaces failed for $serverPhotoId: ${e.message}")
                photoFaces = emptyList()
            }
        }
    }

    /** Clear the Info-panel face list when switching photos / closing. */
    fun clearPhotoFaces() {
        photoFaces = emptyList()
    }

    /**
     * PATCH the changed metadata fields. [request] must already contain ONLY
     * the fields the user changed (nulls are omitted on the wire). When the
     * Photo Type changed, pass [newSubtype] so the local cache + in-memory list
     * are updated and the live pano/360 viewer switches immediately.
     */
    fun saveMetadata(
        serverPhotoId: String,
        request: MetadataUpdateRequest,
        newSubtype: String?,
        onSaved: () -> Unit = {},
    ) {
        viewModelScope.launch {
            metadataSaving = true
            metadataError = null
            try {
                withContext(Dispatchers.IO) { photoRepository.updateMetadata(serverPhotoId, request) }
                // Refresh so the read-only view reflects the saved values.
                try {
                    fullMetadata = withContext(Dispatchers.IO) { photoRepository.getFullMetadata(serverPhotoId) }
                } catch (e: Exception) {
                    Log.w(TAG, "[meta] refresh after save failed: ${e.message}")
                }
                if (newSubtype != null) applyLocalSubtype(serverPhotoId, newSubtype)
                onSaved()
            } catch (e: Exception) {
                Log.e(TAG, "[meta] saveMetadata failed for $serverPhotoId: ${e.message}", e)
                metadataError = e.message ?: "Failed to save"
            } finally {
                metadataSaving = false
            }
        }
    }

    /** Write the current DB metadata back to the file's EXIF (jpeg/tiff). */
    fun writeExif(serverPhotoId: String) {
        viewModelScope.launch {
            metadataSaving = true
            metadataError = null
            try {
                withContext(Dispatchers.IO) { photoRepository.writeExif(serverPhotoId) }
                try {
                    fullMetadata = withContext(Dispatchers.IO) { photoRepository.getFullMetadata(serverPhotoId) }
                } catch (e: Exception) {
                    Log.w(TAG, "[meta] refresh after writeExif failed: ${e.message}")
                }
            } catch (e: Exception) {
                Log.e(TAG, "[meta] writeExif failed for $serverPhotoId: ${e.message}", e)
                metadataError = e.message ?: "Failed to write EXIF"
            } finally {
                metadataSaving = false
            }
        }
    }

    /** Update the local DB + in-memory subtype so the pano/360 viewer switches
     *  without waiting for the periodic resync. Mirrors the web's local db write. */
    private suspend fun applyLocalSubtype(serverPhotoId: String, subtype: String) {
        try {
            withContext(Dispatchers.IO) { photoRepository.updateLocalSubtype(serverPhotoId, subtype) }
        } catch (e: Exception) {
            Log.w(TAG, "[meta] local subtype update failed for $serverPhotoId: ${e.message}")
        }
        updateLoadedPhotos({ it.serverPhotoId == serverPhotoId }) {
            it.copy(photoSubtype = subtype)
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
                // Update in-memory lists
                updateLoadedPhotos({ it.localId == photo.localId }) {
                    it.copy(cropMetadata = metadata)
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
                                decodeThumbEnvelope(thumbDecrypted)?.let { thumbBytes ->
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

    /**
     * Download the original UNCONVERTED source file (the retained pre-conversion
     * original) for a converted photo to [outputFile]. Returns true on success.
     */
    suspend fun downloadSourceToFile(serverPhotoId: String, outputFile: java.io.File): Boolean =
        withContext(Dispatchers.IO) {
            try {
                photoRepository.downloadSourceFileToFile(serverPhotoId, outputFile)
            } catch (_: Exception) {
                false
            }
        }

    /**
     * If [sourceFile] is an AVIF/HEIC/HEIF image (formats many gallery apps
     * can't open), decode it and re-encode as JPEG, returning the JPEG bytes.
     * Returns null when it's not such a format OR decode is unavailable on this
     * device — AVIF decode needs API 31+ (the platform ImageDecoder added AVIF
     * in Android 12) and HEIF needs API 28+ — so the caller falls back to saving
     * the original bytes. Used by the viewer Download action so saved files are
     * universally openable ("convert AVIF to JPEG").
     */
    suspend fun transcodeToJpegIfNeeded(sourceFile: java.io.File, filename: String): ByteArray? =
        withContext(Dispatchers.IO) {
            val ext = filename.substringAfterLast('.', "").lowercase()
            if (ext != "avif" && ext != "heic" && ext != "heif") return@withContext null
            try {
                val bitmap: android.graphics.Bitmap? =
                    if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.P) {
                        val src = android.graphics.ImageDecoder.createSource(sourceFile)
                        android.graphics.ImageDecoder.decodeBitmap(src) { decoder, _, _ ->
                            // Software allocator → a readable bitmap we can compress
                            // (the default hardware bitmap can't be re-encoded).
                            decoder.allocator = android.graphics.ImageDecoder.ALLOCATOR_SOFTWARE
                            decoder.isMutableRequired = false
                        }
                    } else {
                        android.graphics.BitmapFactory.decodeFile(sourceFile.absolutePath)
                    }
                if (bitmap == null) {
                    Log.w(TAG, "[download] $ext decode returned null for $filename")
                    return@withContext null
                }
                val out = java.io.ByteArrayOutputStream()
                bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 95, out)
                bitmap.recycle()
                out.toByteArray()
            } catch (e: Throwable) {
                Log.w(TAG, "[download] $ext->JPEG transcode failed for $filename: ${e.message}")
                null
            }
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
