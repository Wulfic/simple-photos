package com.simplephotos.data.repository

import android.content.ContentUris
import android.content.Context
import android.provider.MediaStore
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
        val KEY_LAST_SYNC_TIMESTAMP = longPreferencesKey("last_sync_timestamp")
    }

    /**
     * Scan device MediaStore for photos and videos added since the last sync,
     * filtered to only include media from folders the user has selected for backup.
     * Inserts new items as PENDING in Room and enqueues BlobQueueEntity entries.
     */
    suspend fun scanForNewMedia() {
        // Ensure defaults are set on first run
        backupFolderRepository.initializeDefaultsIfNeeded()

        val prefs = dataStore.data.first()
        val lastSync = prefs[KEY_LAST_SYNC_TIMESTAMP] ?: 0L

        // Get the list of enabled bucket IDs for filtering
        val enabledBucketIds = backupFolderRepository.getEnabledBucketIds()
        if (enabledBucketIds.isEmpty()) return // No folders selected

        scanImages(lastSync, enabledBucketIds)
        scanVideos(lastSync, enabledBucketIds)

        // Update last sync timestamp
        dataStore.edit { it[KEY_LAST_SYNC_TIMESTAMP] = System.currentTimeMillis() / 1000 }
    }

    /**
     * Build a SQL selection clause that filters by BUCKET_ID.
     * Example: "date_added > ? AND bucket_id IN (123, 456, 789)"
     */
    private fun buildBucketFilter(
        dateColumn: String,
        bucketIdColumn: String,
        bucketIds: List<Long>
    ): Pair<String, Array<String>> {
        val placeholders = bucketIds.joinToString(",") { "?" }
        val selection = "$dateColumn > ? AND $bucketIdColumn IN ($placeholders)"
        val args = mutableListOf<String>()
        // The lastSync arg is added by the caller; bucket IDs follow
        return selection to bucketIds.map { it.toString() }.toTypedArray()
    }

    private suspend fun scanImages(lastSyncSecs: Long, enabledBucketIds: List<Long>) {
        val projection = arrayOf(
            MediaStore.Images.Media._ID,
            MediaStore.Images.Media.DISPLAY_NAME,
            MediaStore.Images.Media.MIME_TYPE,
            MediaStore.Images.Media.WIDTH,
            MediaStore.Images.Media.HEIGHT,
            MediaStore.Images.Media.DATE_TAKEN,
            MediaStore.Images.Media.LATITUDE,
            MediaStore.Images.Media.LONGITUDE,
            MediaStore.Images.Media.BUCKET_ID,
        )

        val bucketPlaceholders = enabledBucketIds.joinToString(",") { "?" }
        val selection = "${MediaStore.Images.Media.DATE_ADDED} > ? AND ${MediaStore.Images.Media.BUCKET_ID} IN ($bucketPlaceholders)"
        val selectionArgs = arrayOf(lastSyncSecs.toString()) + enabledBucketIds.map { it.toString() }.toTypedArray()
        val sortOrder = "${MediaStore.Images.Media.DATE_ADDED} ASC"

        context.contentResolver.query(
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            projection, selection, selectionArgs, sortOrder
        )?.use { cursor ->
            val idCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
            val nameCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DISPLAY_NAME)
            val mimeCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.MIME_TYPE)
            val widthCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.WIDTH)
            val heightCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.HEIGHT)
            val dateCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DATE_TAKEN)

            while (cursor.moveToNext()) {
                val mediaId = cursor.getLong(idCol)
                val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, mediaId)
                val filename = cursor.getString(nameCol) ?: "unknown.jpg"
                val mimeType = cursor.getString(mimeCol) ?: "image/jpeg"
                val width = cursor.getInt(widthCol)
                val height = cursor.getInt(heightCol)
                val dateTaken = cursor.getLong(dateCol)

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

        context.contentResolver.query(
            MediaStore.Video.Media.EXTERNAL_CONTENT_URI,
            projection, selection, selectionArgs, sortOrder
        )?.use { cursor ->
            val idCol = cursor.getColumnIndexOrThrow(MediaStore.Video.Media._ID)
            val nameCol = cursor.getColumnIndexOrThrow(MediaStore.Video.Media.DISPLAY_NAME)
            val mimeCol = cursor.getColumnIndexOrThrow(MediaStore.Video.Media.MIME_TYPE)
            val widthCol = cursor.getColumnIndexOrThrow(MediaStore.Video.Media.WIDTH)
            val heightCol = cursor.getColumnIndexOrThrow(MediaStore.Video.Media.HEIGHT)
            val durationCol = cursor.getColumnIndexOrThrow(MediaStore.Video.Media.DURATION)
            val dateCol = cursor.getColumnIndexOrThrow(MediaStore.Video.Media.DATE_TAKEN)

            while (cursor.moveToNext()) {
                val mediaId = cursor.getLong(idCol)
                val uri = ContentUris.withAppendedId(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, mediaId)
                val filename = cursor.getString(nameCol) ?: "unknown.mp4"
                val mimeType = cursor.getString(mimeCol) ?: "video/mp4"
                val width = cursor.getInt(widthCol)
                val height = cursor.getInt(heightCol)
                val durationMs = cursor.getLong(durationCol)
                val dateTaken = cursor.getLong(dateCol)

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
        }
    }
}
