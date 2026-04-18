/**
 * ViewModel for the main gallery screen, managing photo loading, sync operations,
 * selection state, and album interactions.
 */
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
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.BackupFolderRepository
import com.simplephotos.data.repository.DiagnosticRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SecureGalleryRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_DIAGNOSTIC_LOGGING
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_THUMBNAIL_SIZE
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
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
    private val diagnosticRepository: DiagnosticRepository,
    private val albumRepository: AlbumRepository,
    private val backupFolderRepository: BackupFolderRepository,
    private val secureGalleryRepository: SecureGalleryRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    val photos = photoRepository.getAllPhotos()
    /** Exposed for banner composables that need to poll the server. */
    val apiService get() = photoRepository.apiService
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
    var username by mutableStateOf("")
        private set

    // Thumbnail size preference ("normal" or "large")
    var thumbnailSize by mutableStateOf("normal")
        private set

    // Blob IDs that belong to secure galleries — filtered from the main gallery
    var secureBlobIds by mutableStateOf(emptySet<String>())
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
                val prefs = dataStore.data.first()
                serverBaseUrl = url
                username = prefs[KEY_USERNAME] ?: ""
                thumbnailSize = prefs[KEY_THUMBNAIL_SIZE] ?: "normal"
            } catch (e: Exception) {
                error = "Init failed: ${e.message}"
            }
        }
        // Start periodic polling for secure gallery updates
        startActivityPolling()
    }

    /** Poll the server every 3 seconds for secure gallery updates. */
    private fun startActivityPolling() {
        viewModelScope.launch {
            while (isActive) {
                try {
                    // Refresh secure gallery blob IDs so photos moved to/from
                    // secure galleries on other devices are hidden/shown promptly.
                    try {
                        val freshIds = withContext(Dispatchers.IO) { secureGalleryRepository.getSecureBlobIds() }
                        if (freshIds != secureBlobIds) {
                            secureBlobIds = freshIds
                        }
                    } catch (_: Exception) { /* endpoint unavailable — keep existing set */ }
                } catch (_: Exception) { /* server unreachable — skip this tick */ }
                delay(3_000)
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
                android.util.Log.i("GalleryViewModel", "deleteSelectedPhotos: deleting ${toDelete.size} items")
                for (p in toDelete) {
                    android.util.Log.d("GalleryViewModel", "deleteSelectedPhotos: deleting '${p.filename}' " +
                            "(localId=${p.localId}, serverBlobId=${p.serverBlobId})")
                    withContext(Dispatchers.IO) { photoRepository.deletePhoto(p) }
                }
                android.util.Log.i("GalleryViewModel", "deleteSelectedPhotos: completed successfully")
                clearSelection()
            } catch (e: Exception) {
                android.util.Log.e("GalleryViewModel", "deleteSelectedPhotos: failed: ${e.message}", e)
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
                val diag = diagnosticRepository.createLogger(loggingEnabled)
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

                val pendingCount = withContext(Dispatchers.IO) { photoRepository.getPhotoCountByStatus(SyncStatus.PENDING) }
                val failedCount = withContext(Dispatchers.IO) { photoRepository.getPhotoCountByStatus(SyncStatus.FAILED) }
                val uploadingCount = withContext(Dispatchers.IO) { photoRepository.getPhotoCountByStatus(SyncStatus.UPLOADING) }
                val syncedCount = withContext(Dispatchers.IO) { photoRepository.getPhotoCountByStatus(SyncStatus.SYNCED) }
                diag.info("AppDiagnostic", "Local DB photo status", mapOf(
                    "pending" to pendingCount.toString(), "failed" to failedCount.toString(),
                    "uploading" to uploadingCount.toString(), "synced" to syncedCount.toString()
                ))

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
                val enabledFolders = withContext(Dispatchers.IO) { backupFolderRepository.getEnabledFolders() }
                val allSavedFolders = withContext(Dispatchers.IO) { backupFolderRepository.getFolderCount() }
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
                    val health = withContext(Dispatchers.IO) { diagnosticRepository.getHealthInfo() }
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
                serverBaseUrl = url
                val imported = withContext(Dispatchers.IO) { photoRepository.syncFromServer() }
                // Also sync albums from server (downloads manifests created on web)
                try { withContext(Dispatchers.IO) { albumRepository.syncAlbumsFromServer() } } catch (_: Exception) {}
                // Fetch blob IDs in secure galleries so they can be hidden from the main grid
                try {
                    secureBlobIds = withContext(Dispatchers.IO) { secureGalleryRepository.getSecureBlobIds() }
                } catch (_: Exception) {}
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
                        photoRepository.getSyncedByHash(contentHash)
                    }
                    if (existingByHash != null) continue

                    val thumbBytes = if (mediaType != "video") {
                        withContext(Dispatchers.IO) {
                            var bitmap = android.graphics.BitmapFactory.decodeByteArray(data, 0, data.size)
                            // Apply EXIF rotation so portrait thumbnails are correct
                            try {
                                val exif = androidx.exifinterface.media.ExifInterface(java.io.ByteArrayInputStream(data))
                                val orient = exif.getAttributeInt(
                                    androidx.exifinterface.media.ExifInterface.TAG_ORIENTATION,
                                    androidx.exifinterface.media.ExifInterface.ORIENTATION_NORMAL
                                )
                                val matrix = android.graphics.Matrix()
                                when (orient) {
                                    androidx.exifinterface.media.ExifInterface.ORIENTATION_ROTATE_90 -> matrix.setRotate(90f)
                                    androidx.exifinterface.media.ExifInterface.ORIENTATION_ROTATE_180 -> matrix.setRotate(180f)
                                    androidx.exifinterface.media.ExifInterface.ORIENTATION_ROTATE_270 -> matrix.setRotate(270f)
                                    androidx.exifinterface.media.ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> matrix.setScale(-1f, 1f)
                                    androidx.exifinterface.media.ExifInterface.ORIENTATION_FLIP_VERTICAL -> matrix.setScale(1f, -1f)
                                    androidx.exifinterface.media.ExifInterface.ORIENTATION_TRANSPOSE -> { matrix.setRotate(90f); matrix.postScale(-1f, 1f) }
                                    androidx.exifinterface.media.ExifInterface.ORIENTATION_TRANSVERSE -> { matrix.setRotate(270f); matrix.postScale(-1f, 1f) }
                                    else -> null
                                }?.let {
                                    val rotated = android.graphics.Bitmap.createBitmap(bitmap!!, 0, 0, bitmap!!.width, bitmap!!.height, matrix, true)
                                    if (rotated !== bitmap) bitmap?.recycle()
                                    bitmap = rotated
                                }
                            } catch (_: Exception) {}
                            // Scale to 512px max edge (matching BackupWorker quality)
                            bitmap?.let { bmp ->
                                val w = bmp.width; val h = bmp.height
                                if (w > 0 && h > 0) {
                                    val scale = 512f / maxOf(w, h)
                                    if (scale < 1f) {
                                        val tw = (w * scale).toInt().coerceAtLeast(1)
                                        val th = (h * scale).toInt().coerceAtLeast(1)
                                        val scaled = android.graphics.Bitmap.createScaledBitmap(bmp, tw, th, true)
                                        if (scaled !== bmp) bmp.recycle()
                                        bitmap = scaled
                                    }
                                }
                            }
                            val stream = java.io.ByteArrayOutputStream()
                            bitmap?.compress(android.graphics.Bitmap.CompressFormat.JPEG, 80, stream)
                            bitmap?.recycle()
                            stream.toByteArray()
                        }
                    } else { ByteArray(0) }

                    val localId = java.util.UUID.randomUUID().toString()
                    val filename = uri.lastPathSegment ?: "import_$localId"

                    // Extract correct display dimensions with EXIF orientation
                    var imgWidth = 0
                    var imgHeight = 0
                    if (mediaType == "photo" || mediaType == "gif") {
                        try {
                            val boundsOpts = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
                            android.graphics.BitmapFactory.decodeByteArray(data, 0, data.size, boundsOpts)
                            imgWidth = boundsOpts.outWidth
                            imgHeight = boundsOpts.outHeight
                            val exifDims = androidx.exifinterface.media.ExifInterface(java.io.ByteArrayInputStream(data))
                            val orientDims = exifDims.getAttributeInt(
                                androidx.exifinterface.media.ExifInterface.TAG_ORIENTATION,
                                androidx.exifinterface.media.ExifInterface.ORIENTATION_NORMAL
                            )
                            val needsSwap = orientDims == androidx.exifinterface.media.ExifInterface.ORIENTATION_ROTATE_90
                                    || orientDims == androidx.exifinterface.media.ExifInterface.ORIENTATION_ROTATE_270
                                    || orientDims == androidx.exifinterface.media.ExifInterface.ORIENTATION_TRANSPOSE
                                    || orientDims == androidx.exifinterface.media.ExifInterface.ORIENTATION_TRANSVERSE
                            if (needsSwap && imgWidth > 0 && imgHeight > 0) {
                                val tmp = imgWidth; imgWidth = imgHeight; imgHeight = tmp
                            }
                        } catch (_: Exception) {}
                    }

                    val photo = PhotoEntity(
                        localId = localId, filename = filename, takenAt = System.currentTimeMillis(),
                        mimeType = mimeType, mediaType = mediaType, width = imgWidth, height = imgHeight,
                        localPath = uri.toString(), syncStatus = SyncStatus.PENDING,
                        photoHash = contentHash
                    )
                    photoRepository.insertPhoto(photo)
                    if (thumbBytes.isNotEmpty()) {
                        val thumbPath = photoRepository.saveThumbnailToDisk(localId, thumbBytes)
                        photoRepository.updateThumbnailPath(localId, thumbPath)
                    }
                    try {
                        withContext(Dispatchers.IO) {
                            photoRepository.uploadPhoto(photo, data, thumbBytes.takeIf { it.isNotEmpty() })
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
