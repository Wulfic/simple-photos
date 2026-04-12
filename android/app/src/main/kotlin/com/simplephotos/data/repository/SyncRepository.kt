/**
 * Repository coordinating photo synchronisation between the local device
 * and the remote server, including blob upload and download.
 */
package com.simplephotos.data.repository

import android.content.ContentUris
import android.content.Context
import android.provider.MediaStore
import android.util.Log
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.longPreferencesKey
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.BlobQueueEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.first
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Coordinates sync between device MediaStore and the server.
 * Scans for new photos/videos **only from user-selected folders**,
 * inserts them into Room, and enqueues them for upload via the BlobQueue.
 *
 * The default folder is DCIM/Camera (the standard camera roll).
 * Users can configure additional folders from Settings → Backup Folders.
 */
@Singleton
class SyncRepository @Inject constructor(
    @ApplicationContext private val context: Context,
    private val db: AppDatabase,
    private val dataStore: DataStore<Preferences>,
    private val backupFolderRepository: BackupFolderRepository
) {
    companion object {
        private const val TAG = "SyncRepository"
        val KEY_LAST_SYNC_TIMESTAMP = longPreferencesKey("last_sync_timestamp")
    }

    // Mutex to prevent concurrent scans from inserting duplicate entries
    private val scanMutex = kotlinx.coroutines.sync.Mutex()

    /**
     * Scan device MediaStore for photos and videos added since the last sync,
     * filtered to only include media from folders the user has selected for backup.
     * Inserts new items as PENDING in Room and enqueues BlobQueueEntity entries.
     * Uses a mutex to ensure only one scan runs at a time.
     */
    suspend fun scanForNewMedia() {
        if (!scanMutex.tryLock()) {
            Log.i(TAG, "scanForNewMedia: another scan already in progress, skipping")
            return
        }
        try {
            scanForNewMediaInternal()
        } finally {
            scanMutex.unlock()
        }
    }

    private suspend fun scanForNewMediaInternal() {
        Log.i(TAG, "scanForNewMedia: starting scan")
        // Ensure defaults are set on first run
        backupFolderRepository.initializeDefaultsIfNeeded()

        val prefs = dataStore.data.first()
        val lastSync = prefs[KEY_LAST_SYNC_TIMESTAMP] ?: 0L

        // Get the list of enabled bucket IDs for filtering
        val enabledBucketIds = backupFolderRepository.getEnabledBucketIds()
        Log.i(TAG, "scanForNewMedia: lastSync=$lastSync, enabledBucketIds=$enabledBucketIds (count=${enabledBucketIds.size})")
        if (enabledBucketIds.isEmpty()) {
            Log.w(TAG, "scanForNewMedia: NO enabled bucket IDs — skipping scan entirely")
            return // No folders selected
        }

        // Also log the enabled folder details for context
        val enabledFolders = backupFolderRepository.getEnabledFolders()
        for (f in enabledFolders) {
            Log.i(TAG, "scanForNewMedia: enabled folder: name='${f.bucketName}' path='${f.relativePath}' bucketId=${f.bucketId}")
        }

        // Log permission state at scan time
        val permSnapshot = backupFolderRepository.getPermissionSnapshot()
        Log.i(TAG, "scanForNewMedia: permissions=$permSnapshot")

        scanImages(lastSync, enabledBucketIds)
        scanVideos(lastSync, enabledBucketIds)

        // Update last sync timestamp
        dataStore.edit { it[KEY_LAST_SYNC_TIMESTAMP] = System.currentTimeMillis() / 1000 }
        Log.i(TAG, "scanForNewMedia: scan complete")
    }

    /**
     * Full scan for specific bucket IDs with NO timestamp filter.
     * Used when a user enables a new folder — picks up ALL existing photos.
     * Deduplicates by localPath to avoid re-importing already-known media.
     */
    suspend fun fullScanForBuckets(bucketIds: List<Long>) {
        if (bucketIds.isEmpty()) return
        scanImages(0L, bucketIds)
        scanVideos(0L, bucketIds)
    }

    private suspend fun scanImages(lastSyncSecs: Long, enabledBucketIds: List<Long>) {
        val projection = arrayOf(
            MediaStore.Images.Media._ID,
            MediaStore.Images.Media.DISPLAY_NAME,
            MediaStore.Images.Media.MIME_TYPE,
            MediaStore.Images.Media.WIDTH,
            MediaStore.Images.Media.HEIGHT,
            MediaStore.Images.Media.DATE_TAKEN,
            MediaStore.Images.Media.BUCKET_ID,
            MediaStore.Images.Media.ORIENTATION,
        )

        val bucketPlaceholders = enabledBucketIds.joinToString(",") { "?" }
        val selection = "${MediaStore.Images.Media.DATE_ADDED} > ? AND ${MediaStore.Images.Media.BUCKET_ID} IN ($bucketPlaceholders)"
        val selectionArgs = arrayOf(lastSyncSecs.toString()) + enabledBucketIds.map { it.toString() }.toTypedArray()
        val sortOrder = "${MediaStore.Images.Media.DATE_ADDED} ASC"

        Log.d(TAG, "scanImages: selection='$selection' args=${selectionArgs.toList()} sort='$sortOrder'")

        val cursor = context.contentResolver.query(
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            projection, selection, selectionArgs, sortOrder
        )

        if (cursor == null) {
            Log.e(TAG, "scanImages: ContentResolver returned NULL cursor — likely missing permissions")
            return
        }

        cursor.use { c ->
            Log.i(TAG, "scanImages: cursor has ${c.count} rows")
            val idCol = c.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
            val nameCol = c.getColumnIndexOrThrow(MediaStore.Images.Media.DISPLAY_NAME)
            val mimeCol = c.getColumnIndexOrThrow(MediaStore.Images.Media.MIME_TYPE)
            val widthCol = c.getColumnIndexOrThrow(MediaStore.Images.Media.WIDTH)
            val heightCol = c.getColumnIndexOrThrow(MediaStore.Images.Media.HEIGHT)
            val dateCol = c.getColumnIndexOrThrow(MediaStore.Images.Media.DATE_TAKEN)
            val orientCol = c.getColumnIndexOrThrow(MediaStore.Images.Media.ORIENTATION)

            var newCount = 0
            var skipCount = 0
            while (c.moveToNext()) {
                val mediaId = c.getLong(idCol)
                val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, mediaId)

                // Skip if we already have this file in the database
                if (db.photoDao().getByLocalPath(uri.toString()) != null) {
                    skipCount++
                    continue
                }

                val filename = c.getString(nameCol) ?: "unknown.jpg"
                val mimeType = c.getString(mimeCol) ?: "image/jpeg"
                var width = c.getInt(widthCol)
                var height = c.getInt(heightCol)
                val dateTaken = c.getLong(dateCol)
                val orientation = c.getInt(orientCol)

                // MediaStore returns raw pixel dimensions; swap for 90°/270° EXIF rotation
                if ((orientation == 90 || orientation == 270) && width > 0 && height > 0) {
                    val tmp = width; width = height; height = tmp
                }

                val mediaType = if (mimeType == "image/gif") "gif" else "photo"

                val localId = UUID.randomUUID().toString()
                val photo = PhotoEntity(
                    localId = localId,
                    filename = filename,
                    takenAt = dateTaken,
                    mimeType = mimeType,
                    mediaType = mediaType,
                    width = width,
                    height = height,
                    localPath = uri.toString(),
                    syncStatus = SyncStatus.PENDING
                )
                db.photoDao().insert(photo)
                Log.d(TAG, "scanImages: new photo '$filename' takenAt=$dateTaken localId=$localId")
                newCount++

                // Enqueue for upload: thumbnail first (priority 0), then media (priority 1)
                db.blobQueueDao().insert(BlobQueueEntity(
                    id = UUID.randomUUID().toString(),
                    photoLocalId = localId,
                    blobType = "thumbnail",
                    priority = 0
                ))
                db.blobQueueDao().insert(BlobQueueEntity(
                    id = UUID.randomUUID().toString(),
                    photoLocalId = localId,
                    blobType = mediaType,
                    priority = 1
                ))
            }
            Log.i(TAG, "scanImages: done — $newCount new, $skipCount already known")
        }
    }

    private suspend fun scanVideos(lastSyncSecs: Long, enabledBucketIds: List<Long>) {
        val projection = arrayOf(
            MediaStore.Video.Media._ID,
            MediaStore.Video.Media.DISPLAY_NAME,
            MediaStore.Video.Media.MIME_TYPE,
            MediaStore.Video.Media.WIDTH,
            MediaStore.Video.Media.HEIGHT,
            MediaStore.Video.Media.DURATION,
            MediaStore.Video.Media.DATE_TAKEN,
            MediaStore.Video.Media.BUCKET_ID,
        )

        val bucketPlaceholders = enabledBucketIds.joinToString(",") { "?" }
        val selection = "${MediaStore.Video.Media.DATE_ADDED} > ? AND ${MediaStore.Video.Media.BUCKET_ID} IN ($bucketPlaceholders)"
        val selectionArgs = arrayOf(lastSyncSecs.toString()) + enabledBucketIds.map { it.toString() }.toTypedArray()
        val sortOrder = "${MediaStore.Video.Media.DATE_ADDED} ASC"

        Log.d(TAG, "scanVideos: selection='$selection' args=${selectionArgs.toList()} sort='$sortOrder'")

        val cursor = context.contentResolver.query(
            MediaStore.Video.Media.EXTERNAL_CONTENT_URI,
            projection, selection, selectionArgs, sortOrder
        )

        if (cursor == null) {
            Log.e(TAG, "scanVideos: ContentResolver returned NULL cursor — likely missing permissions")
            return
        }

        cursor.use { c ->
            Log.i(TAG, "scanVideos: cursor has ${c.count} rows")
            val idCol = c.getColumnIndexOrThrow(MediaStore.Video.Media._ID)
            val nameCol = c.getColumnIndexOrThrow(MediaStore.Video.Media.DISPLAY_NAME)
            val mimeCol = c.getColumnIndexOrThrow(MediaStore.Video.Media.MIME_TYPE)
            val widthCol = c.getColumnIndexOrThrow(MediaStore.Video.Media.WIDTH)
            val heightCol = c.getColumnIndexOrThrow(MediaStore.Video.Media.HEIGHT)
            val durationCol = c.getColumnIndexOrThrow(MediaStore.Video.Media.DURATION)
            val dateCol = c.getColumnIndexOrThrow(MediaStore.Video.Media.DATE_TAKEN)

            var newCount = 0
            var skipCount = 0
            while (c.moveToNext()) {
                val mediaId = c.getLong(idCol)
                val uri = ContentUris.withAppendedId(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, mediaId)

                // Skip if we already have this file in the database
                if (db.photoDao().getByLocalPath(uri.toString()) != null) {
                    skipCount++
                    continue
                }

                val filename = c.getString(nameCol) ?: "unknown.mp4"
                val mimeType = c.getString(mimeCol) ?: "video/mp4"
                var width = c.getInt(widthCol)
                var height = c.getInt(heightCol)
                val durationMs = c.getLong(durationCol)
                val dateTaken = c.getLong(dateCol)

                // MediaStore returns coded pixel dimensions for videos; swap for
                // portrait-recorded videos that have a 90°/270° rotation tag.
                if (width > 0 && height > 0) {
                    try {
                        val retriever = android.media.MediaMetadataRetriever()
                        retriever.setDataSource(context, uri)
                        val rotation = retriever.extractMetadata(
                            android.media.MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION
                        )?.toIntOrNull() ?: 0
                        retriever.release()
                        if (rotation == 90 || rotation == 270) {
                            val tmp = width; width = height; height = tmp
                        }
                    } catch (_: Exception) { /* ignore — keep coded dimensions */ }
                }

                val localId = UUID.randomUUID().toString()
                val photo = PhotoEntity(
                    localId = localId,
                    filename = filename,
                    takenAt = dateTaken,
                    mimeType = mimeType,
                    mediaType = "video",
                    width = width,
                    height = height,
                    durationSecs = durationMs / 1000f,
                    localPath = uri.toString(),
                    syncStatus = SyncStatus.PENDING
                )
                db.photoDao().insert(photo)
                Log.d(TAG, "scanVideos: new video '$filename' takenAt=$dateTaken localId=$localId")
                newCount++

                db.blobQueueDao().insert(BlobQueueEntity(
                    id = UUID.randomUUID().toString(),
                    photoLocalId = localId,
                    blobType = "video_thumbnail",
                    priority = 0
                ))
                db.blobQueueDao().insert(BlobQueueEntity(
                    id = UUID.randomUUID().toString(),
                    photoLocalId = localId,
                    blobType = "video",
                    priority = 1
                ))
            }
            Log.i(TAG, "scanVideos: done — $newCount new, $skipCount already known")
        }
    }
}
