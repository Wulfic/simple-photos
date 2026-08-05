package com.simplephotos.ui.screens.album

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.SecureBlobIds
import com.simplephotos.data.collapseBursts
import com.simplephotos.data.excludeSecure
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.FaceCluster
import com.simplephotos.data.remote.dto.GeoMemory
import com.simplephotos.data.remote.dto.GeoTrip
import com.simplephotos.data.remote.dto.PetCluster
import com.simplephotos.data.remote.dto.SharedAlbumInfo
import com.simplephotos.data.repository.AiRepository
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.GeoRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SecureGalleryRepository
import com.simplephotos.data.repository.SharingRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

/**
 * ViewModel for the album list screen. Loads user albums, smart album counts,
 * shared albums, syncs album manifests from the server, and manages album CRUD.
 */
@HiltViewModel
class AlbumViewModel @Inject constructor(
    private val albumRepository: AlbumRepository,
    private val authRepository: AuthRepository,
    private val photoRepository: PhotoRepository,
    private val sharingRepository: SharingRepository,
    private val aiRepository: AiRepository,
    private val geoRepository: GeoRepository,
    private val secureGalleryRepository: SecureGalleryRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    val albums = albumRepository.getAllAlbums()
    var error by mutableStateOf<String?>(null)
    var showCreateDialog by mutableStateOf(false)
    var newAlbumName by mutableStateOf("")
    var username by mutableStateOf("")
        private set

    /** Map of albumId -> first PhotoEntity (for cover image preview) */
    var albumCoverPhotos by mutableStateOf<Map<String, PhotoEntity>>(emptyMap())
        private set

    /** Map of albumId -> visible (secure-excluded) member count, for the tile
     *  badge. Regular album tiles previously showed no count at all (#12); the
     *  count is secure-excluded so it matches the album-detail grid (#16). */
    var albumCounts by mutableStateOf<Map<String, Int>>(emptyMap())
        private set

    /** Blob IDs currently inside a secure gallery — excluded from smart counts
     *  and per-album counts so a secured photo isn't double-counted (#16). */
    private var secureBlobIds: Set<String> = emptySet()

    /** Base URL for server-based thumbnails */
    var serverBaseUrl by mutableStateOf("")
        private set

    /** Photo counts for smart/default albums */
    var totalCount by mutableStateOf(0)
        private set
    /** Count for the "Recently Added" smart album (capped at 100, like web). */
    var recentCount by mutableStateOf(0)
        private set
    var favoritesCount by mutableStateOf(0)
        private set
    var photosCount by mutableStateOf(0)
        private set
    var gifsCount by mutableStateOf(0)
        private set
    var videosCount by mutableStateOf(0)
        private set
    var audioCount by mutableStateOf(0)
        private set

    /** Cover photos for smart albums (keyed by smart album ID) */
    var smartAlbumCoverPhotos by mutableStateOf<Map<String, PhotoEntity>>(emptyMap())
        private set

    // ── Discover sections (people / pets / memories / trips) ────────────
    var peopleClusters by mutableStateOf<List<FaceCluster>>(emptyList())
        private set
    var petClusters by mutableStateOf<List<PetCluster>>(emptyList())
        private set
    var memories by mutableStateOf<List<GeoMemory>>(emptyList())
        private set
    var trips by mutableStateOf<List<GeoTrip>>(emptyList())
        private set

    // The Takeout reconstruction latch, persisted as the mirror size at the
    // moment a pass proved there was nothing left to do (-1 = never).
    //
    // It used to be a plain in-memory flag, so every process start re-ran the
    // whole pass — a /source-albums fetch plus per-album DB work — against a
    // mirror that might still be syncing. Keying it on the photo count keeps the
    // steady state free while still self-healing: the moment the mirror actually
    // grows, the count no longer matches and reconstruction runs again to pick
    // up whatever arrived. Cleared with everything else on logout.
    private var materializedAtPhotoCount = -1

    // Per-process: the gap the previous pass reported, for the "nothing changed
    // and the gap is identical" rule in AlbumRepository.takeoutSettled.
    private var lastUnmatched = -1

    // ── Shared albums ────────────────────────────────────────────────────
    var sharedAlbums by mutableStateOf<List<SharedAlbumInfo>>(emptyList())
        private set
    var sharedLoading by mutableStateOf(true)
        private set
    var showCreateSharedDialog by mutableStateOf(false)
    var newSharedAlbumName by mutableStateOf("")

    init {
        viewModelScope.launch {
            try {
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
                materializedAtPhotoCount = prefs[KEY_TAKEOUT_MATERIALIZED_AT] ?: -1
            } catch (_: Exception) {}
            // Load server config
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
            } catch (_: Exception) {}
            refresh()
        }
    }

    /**
     * Re-sync album manifests from the server and recompute the non-reactive
     * snapshots (smart-album counts/covers, shared albums, Discover sections).
     *
     * The album *list* itself is a reactive Room Flow, but these snapshots were
     * previously computed only once in `init`, so web-created albums and newly
     * synced photos never appeared until the app was restarted (issue #12 —
     * "Albums on Android do not auto-refresh"). The album screen now calls this
     * on every ON_RESUME so re-entering the tab reflects the current state.
     */
    /**
     * One strictly ordered pass: secure ids → server sync → Takeout rebuild →
     * a single count recompute over the settled state.
     *
     * The ordering is the fix, not an aesthetic. This used to be two racing
     * coroutines: one syncing manifests, the other counting from an `albums`
     * snapshot it had taken *before* the sync landed, with a `secureBlobIds` set
     * that was still empty on the first pass. So a resume published a count from
     * pre-sync membership, secure-inclusive, and then published a different one
     * moments later — the "count changes every time it checks in with the
     * server" report. Each input is now settled before it is read.
     */
    fun refresh() {
        viewModelScope.launch {
            // Fails CLOSED (B5): an unavailable set keeps the previous one, so a
            // server hiccup cannot make the smart-album counts start including
            // secured photos. This `catch` used to be unreachable — the
            // repository swallowed the throw and handed back an empty set, which
            // read as "nothing is secured" and silently un-hid everything.
            when (val read = withContext(Dispatchers.IO) { secureGalleryRepository.secureBlobIds() }) {
                is SecureBlobIds.Known -> secureBlobIds = read.ids
                SecureBlobIds.Unavailable -> android.util.Log.w(
                    "AlbumViewModel",
                    "secure id set unavailable — counting with the previous " +
                        "${secureBlobIds.size} id(s) excluded"
                )
            }

            try {
                withContext(Dispatchers.IO) { albumRepository.syncAlbumsFromServer() }
            } catch (e: Exception) {
                android.util.Log.w("AlbumViewModel", "album sync failed: ${e.message}")
            }
            // Takeout albums are captured server-side at import time and rebuilt
            // into local manifests here automatically — no manual "rebuild" step
            // (Issue 2). Idempotent and best-effort; runs after the server sync so
            // it merges on top rather than fighting it. Photos not synced yet are
            // skipped and picked up once the mirror grows.
            runTakeoutReconstruction()

            loadSmartAlbumCounts()
            // Now — and only now — is every input settled: the manifests are
            // synced, the Takeout pass has merged, and the secure set is fresh.
            // `albums.first()` reads the Flow's current value, so this is the
            // post-sync list rather than the pre-sync snapshot it used to be.
            loadCoverPhotos(albums.first())
        }
        loadSharedAlbums()
        loadDiscoverSections()
    }

    /**
     * Rebuild Takeout source albums — but only when the mirror has changed since
     * the last pass that settled. Best-effort throughout: a failure here must
     * never block the album list from rendering.
     */
    private suspend fun runTakeoutReconstruction() {
        val photoCount = try {
            withContext(Dispatchers.IO) { photoRepository.countPhotos() }
        } catch (e: Exception) {
            android.util.Log.w("AlbumViewModel", "mirror count failed: ${e.message}")
            return
        }
        // Settled, and nothing new has synced in since — nothing to do.
        if (photoCount == materializedAtPhotoCount) return

        try {
            val r = withContext(Dispatchers.IO) { albumRepository.recreateAlbumsFromServer() }
            if (AlbumRepository.takeoutSettled(r, lastUnmatched)) {
                materializedAtPhotoCount = photoCount
                try {
                    dataStore.edit { it[KEY_TAKEOUT_MATERIALIZED_AT] = photoCount }
                } catch (e: Exception) {
                    // Non-fatal: we just re-run the (idempotent) pass next launch.
                    android.util.Log.w("AlbumViewModel", "could not persist takeout latch: ${e.message}")
                }
            }
            lastUnmatched = r.photosUnmatched
        } catch (e: Exception) {
            android.util.Log.w("AlbumViewModel", "takeout album materialize failed: ${e.message}")
        }
    }

    private fun loadDiscoverSections() {
        viewModelScope.launch {
            try { peopleClusters = withContext(Dispatchers.IO) { aiRepository.listFaceClusters() } } catch (_: Exception) {}
            try { petClusters = withContext(Dispatchers.IO) { aiRepository.listPetClusters() } } catch (_: Exception) {}
            try { memories = withContext(Dispatchers.IO) { geoRepository.listMemories() } } catch (_: Exception) {}
            try { trips = withContext(Dispatchers.IO) { geoRepository.listTrips() } } catch (_: Exception) {}
        }
    }

    // Suspending, not self-launching: `refresh()` sequences this between the
    // server sync and the per-album count pass, which only works if awaiting it
    // actually waits for it.
    private suspend fun loadSmartAlbumCounts() {
        // The server summary is AUTHORITATIVE for the badge numbers (#42).
        //
        // This used to prefer the local mirror whenever Room held anything, and
        // treat the summary as a cold-start fallback. That is why Android and web
        // disagreed: `getAllPhotos()` is the whole Room table, so it counted
        // device-captured rows the server has never seen, while web counted only
        // rows carrying an encrypted blob. Neither equalled the library.
        //
        // The mirror read below is still needed for cover photos, and still backs
        // the counts when the summary is unavailable. Counts are published in ONE
        // state write from whichever source wins, so the #20 badge-flash (server
        // total written, then overwritten by the local count) cannot recur.
        val allPhotos = try {
            withContext(Dispatchers.IO) {
                photoRepository.getAllPhotos().first()
            }.excludeSecure(secureBlobIds) // secured photos are hidden from the
            // main gallery + smart grids, so they must not be counted here (#16).
        } catch (e: Exception) {
            android.util.Log.w("AlbumViewModel", "local smart-album counts failed: ${e.message}")
            emptyList()
        }

        // Authoritative path: server tile counts. `hasTileCounts` is false when
        // the server predates #42, in which case we fall through to the mirror
        // rather than painting zeros.
        val summary = try {
            withContext(Dispatchers.IO) { photoRepository.apiService.photosSummary() }
                .takeIf { it.hasTileCounts }
        } catch (e: Exception) {
            // Non-fatal. Log so a broken endpoint (stale server) is visible.
            android.util.Log.w("AlbumViewModel", "photos/summary fetch failed: ${e.message}")
            null
        }

        if (summary != null) {
            totalCount = summary.collapsedTotal.toInt()
            recentCount = summary.smartRecent.toInt()
            favoritesCount = summary.smartFavorites.toInt()
            photosCount = summary.smartPhotos.toInt()
            gifsCount = summary.smartGifs.toInt()
            videosCount = summary.smartVideos.toInt()
            audioCount = summary.smartAudio.toInt()
        } else if (allPhotos.isNotEmpty()) {
            // Fallback only. Structurally short by the pending-encryption
            // backlog — those rows are not in Room at all — but at least
            // internally consistent: EVERY category collapses bursts, matching
            // the grids. Previously total/gifs/videos/audio were raw row counts
            // while favorites/photos/recent were collapsed, in this same block.
            totalCount = allPhotos.collapseBursts().size
            // "Recently Added" mirrors the web smart album: capped at 100,
            // with bursts collapsed so a burst counts as one item (the
            // detail list does the same — see AlbumDetailViewModel).
            recentCount = minOf(allPhotos.collapseBursts().size, 100)
            // Counts match the collapsed grids (Favorites/Photos collapse
            // bursts in getAlbumPhotos) so the card count equals the tiles.
            favoritesCount = allPhotos.filter { it.isFavorite }.collapseBursts().size
            photosCount = allPhotos.filter { it.mediaType == "photo" || it.mediaType == "gif" }.collapseBursts().size
            gifsCount = allPhotos.filter { it.mediaType == "gif" }.collapseBursts().size
            videosCount = allPhotos.filter { it.mediaType == "video" }.collapseBursts().size
            audioCount = allPhotos.filter { it.mediaType == "audio" }.collapseBursts().size
        }

        if (allPhotos.isNotEmpty()) {
            // Cover photos always come from the mirror — the summary carries
            // counts only. Independent of which source supplied the numbers.
            val sorted = allPhotos.sortedByDescending { it.takenAt }
            val covers = mutableMapOf<String, PhotoEntity>()
            // "Recently Added" cover = the most recently imported item (by createdAt).
            allPhotos.maxByOrNull { it.createdAt }?.let { covers["smart-recents"] = it }
            sorted.firstOrNull { it.isFavorite }?.let { covers["smart-favorites"] = it }
            sorted.firstOrNull { it.mediaType == "photo" || it.mediaType == "gif" }?.let { covers["smart-photos"] = it }
            sorted.firstOrNull { it.mediaType == "gif" }?.let { covers["smart-gifs"] = it }
            sorted.firstOrNull { it.mediaType == "video" }?.let { covers["smart-videos"] = it }
            sorted.firstOrNull { it.mediaType == "audio" }?.let { covers["smart-audio"] = it }
            smartAlbumCoverPhotos = covers
        }
    }

    /**
     * Load the cover photo + visible member count for each album (call whenever
     * the albums list updates).
     *
     * The count is [AlbumRepository.visibleMemberCount] over the album's stored
     * manifest membership: members still present in the local mirror, minus
     * anything in a secure gallery. That is the same predicate the detail grid
     * resolves with and the same one web's `countRegularAlbum` applies, so a tile
     * badge can neither over-report against the grid it opens (#12/#16) nor
     * disagree with the other platform.
     *
     * Counting from the stored membership rather than from xref rows also means
     * a member this device hasn't synced yet is simply not *visible* — it is not
     * forgotten. It stays in the manifest and returns to the count when it lands.
     */
    suspend fun loadCoverPhotos(albums: List<AlbumEntity>) {
        // One mirror read, reused for every album — no per-album queries.
        val mirror = try {
            withContext(Dispatchers.IO) { photoRepository.getAllPhotos().first() }
        } catch (e: Exception) {
            android.util.Log.w("AlbumViewModel", "album counts: mirror read failed: ${e.message}")
            return
        }
        // Cold start: Room is still empty, so every intersection would be 0.
        // Publishing that would overwrite each album's last known-good count with
        // a zero and show the user an empty library — so leave both the state and
        // the persisted counts alone and let the tiles keep rendering
        // `cachedCount` until the mirror arrives.
        if (mirror.isEmpty()) return

        val photoByBlobId = HashMap<String, PhotoEntity>(mirror.size)
        for (p in mirror) p.serverBlobId?.let { photoByBlobId[it] = p }
        val mirrorBlobIds = photoByBlobId.keys

        val covers = mutableMapOf<String, PhotoEntity>()
        val counts = mutableMapOf<String, Int>()
        for (album in albums) {
            counts[album.localId] = AlbumRepository.visibleMemberCount(
                album.photoBlobIds,
                mirrorBlobIds,
                secureBlobIds,
            )
            album.photoBlobIds
                .firstOrNull { it !in secureBlobIds && it in mirrorBlobIds }
                ?.let { blobId -> photoByBlobId[blobId]?.let { covers[album.localId] = it } }
        }
        // `mutableStateOf` compares structurally, so re-publishing an identical
        // map is already a no-op for Compose — no recomposition, no flicker.
        albumCoverPhotos = covers
        albumCounts = counts

        // Persist so the next cold start opens on the last stable number instead
        // of counting up from 0. Guarded inside the DAO: an unconditional write
        // of an unchanged count would invalidate the albums Flow, re-trigger this
        // very function, and loop.
        withContext(Dispatchers.IO) {
            for (album in albums) {
                counts[album.localId]?.let { albumRepository.setCachedCount(album.localId, it) }
            }
        }
    }

    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { authRepository.logout() }
            } catch (_: Exception) {}
            onLoggedOut()
        }
    }

    fun createAlbum() {
        val name = newAlbumName.trim()
        if (name.isBlank()) return
        viewModelScope.launch {
            try {
                albumRepository.createAlbum(name)
                newAlbumName = ""
                showCreateDialog = false
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun deleteAlbum(album: AlbumEntity) {
        viewModelScope.launch {
            try {
                albumRepository.deleteAlbum(album)
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    // ── Shared album operations ──────────────────────────────────────────

    /** Fetch shared albums from the server. */
    fun loadSharedAlbums() {
        viewModelScope.launch {
            sharedLoading = true
            try {
                sharedAlbums = withContext(Dispatchers.IO) { sharingRepository.listAlbums() }
            } catch (_: Exception) {
                // Non-fatal: shared albums may not be available
            } finally {
                sharedLoading = false
            }
        }
    }

    /** Create a new shared album and refresh the list. */
    fun createSharedAlbum() {
        val name = newSharedAlbumName.trim()
        if (name.isBlank()) return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { sharingRepository.createAlbum(name) }
                newSharedAlbumName = ""
                showCreateSharedDialog = false
                loadSharedAlbums()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    companion object {
        /**
         * Mirror size at the moment Takeout reconstruction last had nothing left
         * to do. Wiped along with every other preference on logout, so a
         * different account never inherits this one's latch.
         */
        private val KEY_TAKEOUT_MATERIALIZED_AT = intPreferencesKey("takeout_materialized_at_photo_count")
    }
}
