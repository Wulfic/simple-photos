package com.simplephotos.data.repository

import android.content.Context
import android.os.Environment
import android.provider.MediaStore
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.BackupFolderEntity
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Data class representing a folder found on the device that contains photos or videos.
 */
data class DeviceFolder(
    val bucketId: Long,
    val bucketName: String,
    val relativePath: String,
    val mediaCount: Int
)

/**
 * Manages which device folders are selected for automatic backup.
 *
 * On first launch, the default selection is DCIM/Camera only.
 * Users can select additional folders via the folder-selection screen.
 */
@Singleton
class BackupFolderRepository @Inject constructor(
    @ApplicationContext private val context: Context,
    private val db: AppDatabase
) {
    companion object {
        /** Default folder: the standard camera roll path. */
        private const val DEFAULT_CAMERA_PATH = "DCIM/Camera"
        private const val DEFAULT_CAMERA_NAME = "Camera"
    }

    fun getSelectedFolders(): Flow<List<BackupFolderEntity>> =
        db.backupFolderDao().getAllFolders()

    suspend fun getEnabledFolders(): List<BackupFolderEntity> =
        db.backupFolderDao().getEnabledFolders()

    suspend fun getEnabledBucketIds(): List<Long> =
        db.backupFolderDao().getEnabledBucketIds()

    /**
     * Initialize default folders on first launch.
     * Sets DCIM/Camera as the default backup folder.
     */
    suspend fun initializeDefaultsIfNeeded() {
        if (db.backupFolderDao().count() > 0) return

        // Find the Camera bucket from MediaStore
        val cameraFolder = scanDeviceFolders().find { folder ->
            folder.relativePath.contains("DCIM/Camera", ignoreCase = true) ||
            folder.bucketName.equals("Camera", ignoreCase = true)
        }

        if (cameraFolder != null) {
            db.backupFolderDao().insert(
                BackupFolderEntity(
                    bucketId = cameraFolder.bucketId,
                    bucketName = cameraFolder.bucketName,
                    relativePath = cameraFolder.relativePath,
                    enabled = true
                )
            )
        } else {
            // Camera folder not found (empty device?), insert a placeholder
            // that will match once photos are taken
            db.backupFolderDao().insert(
                BackupFolderEntity(
                    bucketId = DEFAULT_CAMERA_PATH.hashCode().toLong(),
                    bucketName = DEFAULT_CAMERA_NAME,
                    relativePath = DEFAULT_CAMERA_PATH,
                    enabled = true
                )
            )
        }
    }

    /**
     * Scan the device MediaStore for all folders that contain images or videos.
     * Returns unique folders with media counts.
     */
    suspend fun scanDeviceFolders(): List<DeviceFolder> {
        val folders = mutableMapOf<Long, DeviceFolder>()

        // Scan image folders
        scanMediaFolders(
            uri = MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            bucketIdColumn = MediaStore.Images.Media.BUCKET_ID,
            bucketNameColumn = MediaStore.Images.Media.BUCKET_DISPLAY_NAME,
            relativePathColumn = MediaStore.Images.Media.RELATIVE_PATH,
            folders = folders
        )

        // Scan video folders
        scanMediaFolders(
            uri = MediaStore.Video.Media.EXTERNAL_CONTENT_URI,
            bucketIdColumn = MediaStore.Video.Media.BUCKET_ID,
            bucketNameColumn = MediaStore.Video.Media.BUCKET_DISPLAY_NAME,
            relativePathColumn = MediaStore.Video.Media.RELATIVE_PATH,
            folders = folders
        )

        return folders.values
            .sortedWith(compareByDescending<DeviceFolder> {
                // Camera first, then by count
                it.relativePath.contains("DCIM/Camera", ignoreCase = true)
            }.thenByDescending { it.mediaCount })
    }

    private fun scanMediaFolders(
        uri: android.net.Uri,
        bucketIdColumn: String,
        bucketNameColumn: String,
        relativePathColumn: String,
        folders: MutableMap<Long, DeviceFolder>
    ) {
        val projection = arrayOf(
            bucketIdColumn,
            bucketNameColumn,
            relativePathColumn
        )

        context.contentResolver.query(
            uri, projection, null, null, null
        )?.use { cursor ->
            val bucketIdIdx = cursor.getColumnIndexOrThrow(bucketIdColumn)
            val bucketNameIdx = cursor.getColumnIndexOrThrow(bucketNameColumn)
            val relativePathIdx = cursor.getColumnIndexOrThrow(relativePathColumn)

            while (cursor.moveToNext()) {
                val bucketId = cursor.getLong(bucketIdIdx)
                val bucketName = cursor.getString(bucketNameIdx) ?: "Unknown"
                val relativePath = cursor.getString(relativePathIdx) ?: ""

                val existing = folders[bucketId]
                if (existing != null) {
                    folders[bucketId] = existing.copy(mediaCount = existing.mediaCount + 1)
                } else {
                    folders[bucketId] = DeviceFolder(
                        bucketId = bucketId,
                        bucketName = bucketName,
                        relativePath = relativePath.trimEnd('/'),
                        mediaCount = 1
                    )
                }
            }
        }
    }

    /**
     * Enable or disable a folder for backup.
     * If the folder isn't in the DB yet, insert it.
     */
    suspend fun setFolderEnabled(folder: DeviceFolder, enabled: Boolean) {
        val entity = BackupFolderEntity(
            bucketId = folder.bucketId,
            bucketName = folder.bucketName,
            relativePath = folder.relativePath,
            enabled = enabled
        )
        db.backupFolderDao().insert(entity)
    }

    /**
     * Toggle a folder's enabled state.
     */
    suspend fun toggleFolder(bucketId: Long, enabled: Boolean) {
        db.backupFolderDao().setEnabled(bucketId, enabled)
    }
}
