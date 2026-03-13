package com.simplephotos.ui.screens.gallery

import android.net.Uri
import android.os.Build
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.BackupFolderRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.sync.DiagnosticLogger
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_DIAGNOSTIC_LOGGING
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_THUMBNAIL_SIZE
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.*
import javax.inject.Inject

// ── Day grouping helper ─────────────────────────────────────────────────────

internal fun groupPhotosByDay(photos: List<PhotoEntity>): List<Pair<String, List<PhotoEntity>>> {
    val fmt = SimpleDateFormat("EEEE, MMMM d, yyyy", Locale.getDefault())
    return photos
        // Primary: newest first. Secondary: filename ASC for deterministic
        // ordering when multiple photos share the exact same takenAt.
        // This matches the server's ORDER BY COALESCE(taken_at, created_at) DESC, filename ASC
        .sortedWith(compareByDescending<PhotoEntity> { it.takenAt }.thenBy { it.filename })
        .groupBy { fmt.format(Date(it.takenAt)) }
        .toList()
}

// Sealed class for grid items (headers vs photos)
internal sealed class GalleryGridItem {
    data class Header(val dateLabel: String, val photoIds: Set<String>) : GalleryGridItem()
    data class Photo(val photo: PhotoEntity) : GalleryGridItem()
}

internal fun buildGridItems(dayGroups: List<Pair<String, List<PhotoEntity>>>): List<GalleryGridItem> {
    val items = mutableListOf<GalleryGridItem>()
    for ((dateLabel, photos) in dayGroups) {
        items.add(GalleryGridItem.Header(dateLabel, photos.map { it.localId }.toSet()))
        for (photo in photos) {
            items.add(GalleryGridItem.Photo(photo))
        }
    }
    return items
}

// ── ViewModel ───────────────────────────────────────────────────────────────

/**
 * Drives the main gallery screen: server sync, multi-select operations,
 * photo import from content URIs, album management, and diagnostic reporting.
 */
@HiltViewModel
class GalleryViewModel @Inject constructor(
    private val photoRepository: PhotoRepository,
    private val authRepository: AuthRepository,
    private val api: ApiService,
    private val db: AppDatabase,
    private val albumRepository: AlbumRepository,
    private val backupFolderRepository: BackupFolderRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    val photos = photoRepository.getAllPhotos()
    var error by mutableStateOf<String?>(null)
    var isSyncing by mutableStateOf(false)
        private set
    var isImporting by mutableStateOf(false)
        private set
    var lastSyncResult by mutableStateOf<String?>(null)
        private set

    /** True once the first server sync has completed.
     *  Until then, the Gallery shows a loading indicator instead of
     *  Room-cached photos — prevents flashing stale data from a
     *  previous user's session. */
    var dataReady by mutableStateOf(false)
        private set

    var serverBaseUrl by mutableStateOf("")
        private set
    var encryptionMode by mutableStateOf("plain")
        private set
    var username by mutableStateOf("")
        private set

    // Thumbnail size preference ("normal" or "large")
    var thumbnailSize by mutableStateOf("normal")
        private set

    // ── Multi-select state ────────────────────────────────────────
    var selectedIds by mutableStateOf(emptySet<String>())
        private set
    var isSelectionMode by mutableStateOf(false)
        private set

    // ── Album state for picker ────────────────────────────────────
    val albums = albumRepository.getAllAlbums()

    init {
        viewModelScope.launch {
            try {
                val url = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val mode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }
                val prefs = dataStore.data.first()
                serverBaseUrl = url
                encryptionMode = mode
                username = prefs[KEY_USERNAME] ?: ""
                thumbnailSize = prefs[KEY_THUMBNAIL_SIZE] ?: "normal"
            } catch (e: Exception) {
                error = "Init failed: ${e.message}"
            }
        }
    }

    fun enterSelectionMode(id: String) {
        isSelectionMode = true
        selectedIds = setOf(id)
    }

    fun toggleSelect(id: String) {
        if (!isSelectionMode) return
        selectedIds = if (id in selectedIds) selectedIds - id else selectedIds + id
        if (selectedIds.isEmpty()) isSelectionMode = false
    }

    fun selectAll(allPhotos: List<PhotoEntity>) {
        isSelectionMode = true
        selectedIds = allPhotos.map { it.localId }.toSet()
    }

    fun selectDay(dayPhotoIds: Set<String>) {
        isSelectionMode = true
        selectedIds = selectedIds + dayPhotoIds
    }

    fun clearSelection() {
        selectedIds = emptySet()
        isSelectionMode = false
    }

    fun deleteSelectedPhotos(allPhotos: List<PhotoEntity>) {
        viewModelScope.launch {
            try {
                val toDelete = allPhotos.filter { it.localId in selectedIds }
                for (p in toDelete) {
                    withContext(Dispatchers.IO) { photoRepository.deletePhoto(p) }
                }
                clearSelection()
            } catch (e: Exception) {
                error = "Delete failed: ${e.message}"
            }
        }
    }

    fun addSelectedToAlbum(albumId: String) {
        viewModelScope.launch {
            try {
                for (id in selectedIds) {
                    withContext(Dispatchers.IO) { albumRepository.addPhotoToAlbum(id, albumId) }
                }
                clearSelection()
            } catch (e: Exception) {
                error = "Add to album failed: ${e.message}"
            }
        }
    }

    fun createAlbumAndAddSelected(name: String) {
        viewModelScope.launch {
            try {
                val album = withContext(Dispatchers.IO) { albumRepository.createAlbum(name) }
                for (id in selectedIds) {
                    withContext(Dispatchers.IO) { albumRepository.addPhotoToAlbum(id, album.localId) }
                }
                clearSelection()
            } catch (e: Exception) {
                error = "Create album failed: ${e.message}"
            }
        }
    }

    fun sendDiagnosticReport(context: android.content.Context) {
        viewModelScope.launch {
            try {
                val prefs = dataStore.data.first()
                val loggingEnabled = prefs[KEY_DIAGNOSTIC_LOGGING] ?: false
                val diag = DiagnosticLogger(api, loggingEnabled)
                diag.info("AppDiagnostic", "Gallery opened — sending diagnostic report")

                // ── Device info ─────────────────────────────────────────
                diag.info("AppDiagnostic", "Device info", mapOf(
                    "apiLevel" to Build.VERSION.SDK_INT.toString(),
                    "release" to Build.VERSION.RELEASE,
                    "manufacturer" to Build.MANUFACTURER,
                    "model" to Build.MODEL,
                    "securityPatch" to (Build.VERSION.SECURITY_PATCH ?: "unknown")
                ))

                // ── Permission state ────────────────────────────────────
                val permSnapshot = withContext(Dispatchers.IO) { backupFolderRepository.getPermissionSnapshot() }
                diag.info("AppDiagnostic", "Permission state", permSnapshot.mapValues { it.value.toString() })

                val pendingCount = withContext(Dispatchers.IO) { db.photoDao().getByStatus(SyncStatus.PENDING).size }
                val failedCount = withContext(Dispatchers.IO) { db.photoDao().getByStatus(SyncStatus.FAILED).size }
                val uploadingCount = withContext(Dispatchers.IO) { db.photoDao().getByStatus(SyncStatus.UPLOADING).size }
                val syncedCount = withContext(Dispatchers.IO) { db.photoDao().getByStatus(SyncStatus.SYNCED).size }
                diag.info("AppDiagnostic", "Local DB photo status", mapOf(
                    "pending" to pendingCount.toString(), "failed" to failedCount.toString(),
                    "uploading" to uploadingCount.toString(), "synced" to syncedCount.toString()
                ))

                val mode = try { withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() } } catch (e: Exception) { "error: ${e.message}" }
                diag.info("AppDiagnostic", "Encryption mode: $mode")

                // ── Photo ordering diagnostic ───────────────────────────
                // Log the first 10 photos in Room order so we can compare with web/server
                try {
                    val allPhotos = withContext(Dispatchers.IO) { photos.first() }
                    diag.info("AppDiagnostic", "Photo ordering — first 10 from Room (takenAt DESC, filename ASC)", mapOf(
                        "totalPhotos" to allPhotos.size.toString()
                    ))
                    for ((i, p) in allPhotos.take(10).withIndex()) {
                        diag.info("AppDiagnostic", "Photo[$i]: filename='${p.filename}' takenAt=${p.takenAt} (${java.util.Date(p.takenAt)}) localId=${p.localId}", mapOf(
                            "index" to i.toString(),
                            "filename" to p.filename,
                            "takenAt" to p.takenAt.toString(),
                            "takenAtDate" to java.util.Date(p.takenAt).toString(),
                            "localId" to p.localId,
                            "serverPhotoId" to (p.serverPhotoId ?: "null"),
                            "serverBlobId" to (p.serverBlobId ?: "null")
                        ))
                    }
                    // Log photos with duplicate takenAt timestamps — potential ordering issues
                    val dupeTimestamps = allPhotos.groupBy { it.takenAt }.filter { it.value.size > 1 }
                    if (dupeTimestamps.isNotEmpty()) {
                        diag.warn("AppDiagnostic", "Photos with DUPLICATE takenAt timestamps (order may be non-deterministic without tiebreaker)", mapOf(
                            "duplicateTimestampCount" to dupeTimestamps.size.toString(),
                            "affectedPhotoCount" to dupeTimestamps.values.sumOf { it.size }.toString()
                        ))
                        for ((ts, photos) in dupeTimestamps.entries.take(5)) {
                            val names = photos.map { it.filename }.sorted()
                            diag.info("AppDiagnostic", "Duplicate takenAt=$ts: ${names.joinToString(", ")}")
                        }
                    }
                } catch (e: Exception) {
                    diag.error("AppDiagnostic", "Photo ordering diagnostic failed: ${e.message}")
                }

                // ── Backup folder diagnostic ────────────────────────────
                val enabledFolders = withContext(Dispatchers.IO) { db.backupFolderDao().getEnabledFolders() }
                val allSavedFolders = withContext(Dispatchers.IO) { db.backupFolderDao().count() }
                diag.info("AppDiagnostic", "Backup folders in DB", mapOf(
                    "totalInDB" to allSavedFolders.toString(),
                    "enabledCount" to enabledFolders.size.toString(),
                    "folders" to enabledFolders.joinToString(", ") { "${it.bucketName}(path='${it.relativePath}', bucketId=${it.bucketId}, enabled=${it.enabled})" }
                ))

                // Perform a live scan of device folders for comparison
                try {
                    val deviceFolders = withContext(Dispatchers.IO) { backupFolderRepository.scanDeviceFolders() }
                    diag.info("AppDiagnostic", "Live device folder scan", mapOf(
                        "totalFoldersFound" to deviceFolders.size.toString()
                    ))
                    for (f in deviceFolders) {
                        diag.info("AppDiagnostic", "Device folder: name='${f.bucketName}' path='${f.relativePath}' bucketId=${f.bucketId} count=${f.mediaCount}", mapOf(
                            "bucketName" to f.bucketName,
                            "relativePath" to f.relativePath,
                            "bucketId" to f.bucketId.toString(),
                            "mediaCount" to f.mediaCount.toString()
                        ))
                    }
                    if (deviceFolders.size <= 1) {
                        diag.warn("AppDiagnostic", "Only ${deviceFolders.size} folder(s) visible — possible permission issue. Expected multiple folders (Camera, Screenshots, etc.) with full media access.")
                    }
                } catch (e: Exception) {
                    diag.error("AppDiagnostic", "Device folder scan failed: ${e.message}", mapOf("exception" to (e::class.simpleName ?: "Unknown")))
                }

                try {
                    val wm = androidx.work.WorkManager.getInstance(context)
                    val periodicInfos = wm.getWorkInfosForUniqueWork("photo_backup").get()
                    val reactiveInfos = wm.getWorkInfosForUniqueWork("photo_backup_reactive").get()
                    diag.info("AppDiagnostic", "WorkManager periodic tasks", mapOf(
                        "count" to periodicInfos.size.toString(),
                        "states" to periodicInfos.joinToString(", ") { "${it.state}(attempt=${it.runAttemptCount})" }
                    ))
                    diag.info("AppDiagnostic", "WorkManager reactive tasks", mapOf(
                        "count" to reactiveInfos.size.toString(),
                        "states" to reactiveInfos.joinToString(", ") { "${it.state}(attempt=${it.runAttemptCount})" }
                    ))
                } catch (e: Exception) {
                    diag.error("AppDiagnostic", "Failed to query WorkManager: ${e.message}")
                }

                try {
                    val health = withContext(Dispatchers.IO) { api.health() }
                    diag.info("AppDiagnostic", "Server health OK", health.mapValues { it.value })
                } catch (e: Exception) {
                    diag.error("AppDiagnostic", "Server health check failed: ${e.message}", mapOf("exception" to (e::class.simpleName ?: "Unknown")))
                }

                withContext(Dispatchers.IO) { diag.flush() }
            } catch (e: Exception) {
                android.util.Log.e("AppDiagnostic", "Failed to send diagnostic report", e)
            }
        }
    }

    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            try { withContext(Dispatchers.IO) { authRepository.logout() } ; onLoggedOut() } catch (_: Exception) { onLoggedOut() }
        }
    }

    fun deletePhoto(photo: PhotoEntity) {
        viewModelScope.launch {
            try { withContext(Dispatchers.IO) { photoRepository.deletePhoto(photo) } } catch (e: Exception) { error = e.message }
        }
    }

    fun syncFromServer() {
        if (isSyncing) return
        viewModelScope.launch {
            isSyncing = true; error = null; lastSyncResult = null
            try {
                val url = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val mode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }
                serverBaseUrl = url; encryptionMode = mode
                val imported = withContext(Dispatchers.IO) { photoRepository.syncFromServer() }
                // Also sync albums from server (downloads manifests created on web)
                try { withContext(Dispatchers.IO) { albumRepository.syncAlbumsFromServer() } } catch (_: Exception) {}
                lastSyncResult = if (imported > 0) "Synced $imported new items" else "Up to date"
            } catch (e: Exception) { error = "Sync failed: ${e.message}" } finally { isSyncing = false; dataReady = true }
        }
    }

    fun importFromUris(uris: List<Uri>, context: android.content.Context) {
        if (uris.isEmpty()) return
        viewModelScope.launch {
            isImporting = true; error = null; var count = 0
            for (uri in uris) {
                try {
                    val resolver = context.contentResolver
                    val mimeType = resolver.getType(uri) ?: "image/jpeg"
                    val mediaType = when {
                        mimeType.startsWith("video/") -> "video"
                        mimeType.startsWith("audio/") -> "audio"
                        mimeType == "image/gif" -> "gif"
                        else -> "photo"
                    }
                    val data = withContext(Dispatchers.IO) { resolver.openInputStream(uri)?.use { it.readBytes() } } ?: continue

                    // Content hash dedup — skip if identical content already exists in DB
                    val contentHash = withContext(Dispatchers.IO) {
                        java.security.MessageDigest.getInstance("SHA-256")
                            .digest(data)
                            .take(6)
                            .joinToString("") { "%02x".format(it) }
                    }
                    val existingByHash = withContext(Dispatchers.IO) {
                        db.photoDao().getSyncedByHash(contentHash)
                    }
                    if (existingByHash != null) continue

                    val thumbBytes = if (mediaType != "video") {
                        withContext(Dispatchers.IO) {
                            val opts = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
                            android.graphics.BitmapFactory.decodeByteArray(data, 0, data.size, opts)
                            val scaleFactor = maxOf(opts.outWidth, opts.outHeight) / 256
                            val opts2 = android.graphics.BitmapFactory.Options().apply { inSampleSize = maxOf(1, scaleFactor) }
                            val bitmap = android.graphics.BitmapFactory.decodeByteArray(data, 0, data.size, opts2)
                            val stream = java.io.ByteArrayOutputStream()
                            bitmap?.compress(android.graphics.Bitmap.CompressFormat.JPEG, 80, stream)
                            bitmap?.recycle()
                            stream.toByteArray()
                        }
                    } else { ByteArray(0) }

                    val localId = java.util.UUID.randomUUID().toString()
                    val filename = uri.lastPathSegment ?: "import_$localId"
                    val photo = PhotoEntity(
                        localId = localId, filename = filename, takenAt = System.currentTimeMillis(),
                        mimeType = mimeType, mediaType = mediaType, width = 0, height = 0,
                        localPath = uri.toString(), syncStatus = SyncStatus.PENDING,
                        photoHash = contentHash
                    )
                    photoRepository.insertPhoto(photo)
                    if (thumbBytes.isNotEmpty()) {
                        val thumbPath = photoRepository.saveThumbnailToDisk(localId, thumbBytes)
                        photoRepository.insertPhoto(photo.copy(thumbnailPath = thumbPath))
                    }
                    try {
                        withContext(Dispatchers.IO) {
                            if (encryptionMode == "plain") photoRepository.uploadPhotoPlain(photo, data)
                            else photoRepository.uploadPhoto(photo, data, thumbBytes.takeIf { it.isNotEmpty() })
                        }
                    } catch (_: Exception) {}
                    count++
                } catch (e: Exception) { error = "Import error: ${e.message}" }
            }
            lastSyncResult = "Imported $count photo${if (count != 1) "s" else ""}"
            isImporting = false
        }
    }
}
