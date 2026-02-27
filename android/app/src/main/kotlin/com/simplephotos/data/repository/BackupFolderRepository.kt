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

        /** Sentinel bucket ID used when no camera photos exist yet. */
        val PLACEHOLDER_BUCKET_ID = DEFAULT_CAMERA_PATH.hashCode().toLong()
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
        if (db.backupFolderDao().count() > 0) {
            // Already initialized — but check if we have a placeholder bucket ID
            // that needs to be resolved to a real MediaStore bucket ID.
            resolvePlaceholderBucket()
            return
        }

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
                    bucketId = PLACEHOLDER_BUCKET_ID,
                    bucketName = DEFAULT_CAMERA_NAME,
                    relativePath = DEFAULT_CAMERA_PATH,
                    enabled = true
                )
            )
        }
    }

    /**
     * If we inserted a placeholder bucket ID at first launch (because the
     * Camera folder didn't exist yet), check whether photos now exist in
     * DCIM/Camera and replace the placeholder with the real MediaStore
     * bucket ID so the scan filter actually matches.
     */
    private suspend fun resolvePlaceholderBucket() {
        val enabledFolders = db.backupFolderDao().getEnabledFolders()
        val placeholder = enabledFolders.find { it.bucketId == PLACEHOLDER_BUCKET_ID }
            ?: return // no placeholder — nothing to fix

        // Re-scan device to find the real Camera bucket
        val cameraFolder = scanDeviceFolders().find { folder ->
            folder.relativePath.contains("DCIM/Camera", ignoreCase = true) ||
            folder.bucketName.equals("Camera", ignoreCase = true)
        } ?: return // still no camera folder on device

        // Replace the placeholder with the real bucket ID
        db.backupFolderDao().delete(placeholder)
        db.backupFolderDao().insert(
            BackupFolderEntity(
                bucketId = cameraFolder.bucketId,
                bucketName = cameraFolder.bucketName,
                relativePath = cameraFolder.relativePath,
                enabled = true
            )
        )
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
