/**
 * Repository for discovering device media folders and managing which
 * folders are selected for backup.
 */
package com.simplephotos.data.repository

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import androidx.core.content.ContextCompat
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
 * Detailed permission diagnostics for debugging folder-discovery issues.
 * Shown in the folder-selection screen's diagnostic panel.
 */
data class PermissionDiagnostics(
    val apiLevel: Int,
    val readMediaImages: Boolean?,
    val readMediaVideo: Boolean?,
    val readMediaVisualUserSelected: Boolean?,
    val readExternalStorage: Boolean?,
    val hasFullAccess: Boolean,
    val hasPartialAccess: Boolean
) {
    fun toLogString(): String = buildString {
        append("API=$apiLevel")
        readMediaImages?.let { append(" READ_MEDIA_IMAGES=$it") }
        readMediaVideo?.let { append(" READ_MEDIA_VIDEO=$it") }
        readMediaVisualUserSelected?.let { append(" VISUAL_USER_SELECTED=$it") }
        readExternalStorage?.let { append(" READ_EXTERNAL_STORAGE=$it") }
        append(" fullAccess=$hasFullAccess partialAccess=$hasPartialAccess")
    }
}

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
        private const val TAG = "BackupFolderRepo"

        /** Default folder: the standard camera roll path. */
        private const val DEFAULT_CAMERA_PATH = "DCIM/Camera"
        private const val DEFAULT_CAMERA_NAME = "Camera"

        /** Sentinel bucket ID used when no camera photos exist yet. */
        val PLACEHOLDER_BUCKET_ID = DEFAULT_CAMERA_PATH.hashCode().toLong()

        /** Patterns for folders that should be auto-enabled on fresh install.
         *  Only the camera roll — everything else the user opts into. */
        private val AUTO_ENABLE_PATTERNS = listOf(
            "DCIM/Camera"
        )

        /**
         * Direct system check for full media access.
         * Does NOT rely on Accompanist — uses ContextCompat.checkSelfPermission().
         *
         * On Android 14+ with READ_MEDIA_VISUAL_USER_SELECTED declared in the
         * manifest, Accompanist can incorrectly report allPermissionsGranted=true
         * when only partial ("Select photos") access was granted.  This function
         * uses the system check, but note: on some Android 14 builds
         * (b/308531058), checkSelfPermission itself can also return GRANTED
         * for READ_MEDIA_IMAGES under partial access.  Use
         * [likelyPartialAccessDespitePermissions] after scanning as a
         * secondary heuristic.
         */
        fun hasFullMediaAccess(context: Context): Boolean {
            return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                ContextCompat.checkSelfPermission(
                    context, Manifest.permission.READ_MEDIA_IMAGES
                ) == PackageManager.PERMISSION_GRANTED &&
                ContextCompat.checkSelfPermission(
                    context, Manifest.permission.READ_MEDIA_VIDEO
                ) == PackageManager.PERMISSION_GRANTED
            } else {
                ContextCompat.checkSelfPermission(
                    context, Manifest.permission.READ_EXTERNAL_STORAGE
                ) == PackageManager.PERMISSION_GRANTED
            }
        }

        /**
         * Detect Android 14+ partial access: the user chose "Select photos"
         * instead of "Allow all", so READ_MEDIA_VISUAL_USER_SELECTED is
         * granted but READ_MEDIA_IMAGES is not.
         */
        fun hasPartialMediaAccess(context: Context): Boolean {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return false
            if (hasFullMediaAccess(context)) return false
            return ContextCompat.checkSelfPermission(
                context, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED
            ) == PackageManager.PERMISSION_GRANTED
        }

        /**
         * Post-scan heuristic: on Android 14+ (API 34), there is a platform
         * bug (b/308531058) where [checkSelfPermission] for READ_MEDIA_IMAGES
         * can return GRANTED even when the user only chose "Select photos".
         * In that case [hasFullMediaAccess] returns true, but MediaStore only
         * exposes the user-selected subset — causing folder discovery to show
         * very few folders.
         *
         * Call this AFTER [scanDeviceFolders]: if we're on API 34+,
         * VISUAL_USER_SELECTED is granted, and the scan returned <= [threshold]
         * folders, we almost certainly have partial access despite what the
         * permission checks report.
         */
        fun likelyPartialAccessDespitePermissions(
            context: Context,
            discoveredFolderCount: Int,
            threshold: Int = 1
        ): Boolean {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return false
            val visualUserSelected = ContextCompat.checkSelfPermission(
                context, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED
            ) == PackageManager.PERMISSION_GRANTED
            if (!visualUserSelected) return false
            return discoveredFolderCount <= threshold
        }
    }

    /**
     * Snapshot the current permission state for diagnostic logging.
     * Returns a map of permission name → granted status.
     */
    fun getPermissionSnapshot(): Map<String, Boolean> {
        val perms = mutableMapOf<String, Boolean>()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            perms["READ_MEDIA_IMAGES"] = ContextCompat.checkSelfPermission(
                context, Manifest.permission.READ_MEDIA_IMAGES
            ) == PackageManager.PERMISSION_GRANTED
            perms["READ_MEDIA_VIDEO"] = ContextCompat.checkSelfPermission(
                context, Manifest.permission.READ_MEDIA_VIDEO
            ) == PackageManager.PERMISSION_GRANTED
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            perms["READ_MEDIA_VISUAL_USER_SELECTED"] = ContextCompat.checkSelfPermission(
                context, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED
            ) == PackageManager.PERMISSION_GRANTED
        }
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            perms["READ_EXTERNAL_STORAGE"] = ContextCompat.checkSelfPermission(
                context, Manifest.permission.READ_EXTERNAL_STORAGE
            ) == PackageManager.PERMISSION_GRANTED
        }
        return perms
    }

    /**
     * Full permission diagnostics for the folder selection UI diagnostic panel.
     */
    fun getPermissionDiagnostics(): PermissionDiagnostics {
        val readImages = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_IMAGES) == PackageManager.PERMISSION_GRANTED
        } else null
        val readVideo = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_VIDEO) == PackageManager.PERMISSION_GRANTED
        } else null
        val visualUserSelected = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED) == PackageManager.PERMISSION_GRANTED
        } else null
        val readExternal = if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED
        } else null

        val fullAccess = hasFullMediaAccess(context)
        val partialAccess = hasPartialMediaAccess(context)

        return PermissionDiagnostics(
            apiLevel = Build.VERSION.SDK_INT,
            readMediaImages = readImages,
            readMediaVideo = readVideo,
            readMediaVisualUserSelected = visualUserSelected,
            readExternalStorage = readExternal,
            hasFullAccess = fullAccess,
            hasPartialAccess = partialAccess
        )
    }

    fun getSelectedFolders(): Flow<List<BackupFolderEntity>> =
        db.backupFolderDao().getAllFolders()

    suspend fun getEnabledFolders(): List<BackupFolderEntity> =
        db.backupFolderDao().getEnabledFolders()

    /** Total number of folders saved in the DB (enabled + disabled). */
    suspend fun getFolderCount(): Int =
        db.backupFolderDao().count()

    suspend fun getEnabledBucketIds(): List<Long> =
        db.backupFolderDao().getEnabledBucketIds()

    /**
     * Initialize default folders on first launch.
     * Auto-enables all well-known media folders (Camera, Pictures, Screenshots,
     * Download, Movies) so the user gets comprehensive backup out of the box.
     */
    suspend fun initializeDefaultsIfNeeded() {
        val permSnapshot = getPermissionSnapshot()
        Log.i(TAG, "initializeDefaultsIfNeeded: API=${Build.VERSION.SDK_INT}, permissions=$permSnapshot")

        val existingCount = db.backupFolderDao().count()
        if (existingCount > 0) {
            Log.i(TAG, "initializeDefaultsIfNeeded: already have $existingCount folders in DB")
            // Already initialized — but check if we have a placeholder bucket ID
            // that needs to be resolved to a real MediaStore bucket ID.
            resolvePlaceholderBucket()

            // Merge in any newly-visible folders (e.g. after a permission upgrade
            // from partial to full media access).
            mergeNewDeviceFolders()
            return
        }

        Log.i(TAG, "initializeDefaultsIfNeeded: fresh install — scanning device folders")
        // Fresh install / DB was wiped — discover all device folders and
        // auto-enable the well-known ones.
        val allFolders = scanDeviceFolders()

        if (allFolders.isEmpty()) {
            Log.w(TAG, "initializeDefaultsIfNeeded: no media folders found — inserting placeholder")
            // No media at all — insert a Camera placeholder so the user
            // sees something once photos are taken.
            db.backupFolderDao().insert(
                BackupFolderEntity(
                    bucketId = PLACEHOLDER_BUCKET_ID,
                    bucketName = DEFAULT_CAMERA_NAME,
                    relativePath = DEFAULT_CAMERA_PATH,
                    enabled = true
                )
            )
            return
        }

        val entities = allFolders.map { folder ->
            val shouldEnable = AUTO_ENABLE_PATTERNS.any { pattern ->
                folder.relativePath.contains(pattern, ignoreCase = true) ||
                folder.bucketName.equals(pattern, ignoreCase = true)
            }
            Log.i(TAG, "initializeDefaultsIfNeeded: folder '${folder.bucketName}' path='${folder.relativePath}' bucketId=${folder.bucketId} count=${folder.mediaCount} autoEnable=$shouldEnable")
            BackupFolderEntity(
                bucketId = folder.bucketId,
                bucketName = folder.bucketName,
                relativePath = folder.relativePath,
                enabled = shouldEnable
            )
        }
        db.backupFolderDao().insertAll(entities)
        Log.i(TAG, "initializeDefaultsIfNeeded: inserted ${entities.size} folders, ${entities.count { it.enabled }} enabled")
    }

    /**
     * After a permission upgrade (e.g. from partial READ_MEDIA_VISUAL_USER_SELECTED
     * to full READ_MEDIA_IMAGES), the device may expose folders that were previously
     * invisible.  Insert any newly-discovered folders into Room so they appear on the
     * folder-selection screen.  New folders are always disabled — the user must
     * explicitly opt-in to back them up.
     */
    private suspend fun mergeNewDeviceFolders() {
        val allDeviceFolders = scanDeviceFolders()
        val knownBucketIds = db.backupFolderDao().getAllBucketIds().toSet()

        val newFolders = allDeviceFolders.filter { it.bucketId !in knownBucketIds }
        Log.i(TAG, "mergeNewDeviceFolders: device=${allDeviceFolders.size} known=${knownBucketIds.size} new=${newFolders.size}")
        if (newFolders.isEmpty()) return

        val entities = newFolders.map { folder ->
            Log.i(TAG, "mergeNewDeviceFolders: new folder '${folder.bucketName}' path='${folder.relativePath}' bucketId=${folder.bucketId} count=${folder.mediaCount} enabled=false")
            BackupFolderEntity(
                bucketId = folder.bucketId,
                bucketName = folder.bucketName,
                relativePath = folder.relativePath,
                enabled = false
            )
        }
        db.backupFolderDao().insertAll(entities)
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
        val permSnapshot = getPermissionSnapshot()
        Log.i(TAG, "scanDeviceFolders: starting scan, API=${Build.VERSION.SDK_INT}, permissions=$permSnapshot")

        val folders = mutableMapOf<Long, DeviceFolder>()

        // Scan image folders
        Log.d(TAG, "scanDeviceFolders: querying Images MediaStore...")
        scanMediaFolders(
            uri = MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            bucketIdColumn = MediaStore.Images.Media.BUCKET_ID,
            bucketNameColumn = MediaStore.Images.Media.BUCKET_DISPLAY_NAME,
            relativePathColumn = MediaStore.Images.Media.RELATIVE_PATH,
            mediaType = "images",
            folders = folders
        )
        Log.i(TAG, "scanDeviceFolders: after images scan — ${folders.size} unique folders found")

        // Scan video folders
        Log.d(TAG, "scanDeviceFolders: querying Videos MediaStore...")
        val foldersBefore = folders.size
        scanMediaFolders(
            uri = MediaStore.Video.Media.EXTERNAL_CONTENT_URI,
            bucketIdColumn = MediaStore.Video.Media.BUCKET_ID,
            bucketNameColumn = MediaStore.Video.Media.BUCKET_DISPLAY_NAME,
            relativePathColumn = MediaStore.Video.Media.RELATIVE_PATH,
            mediaType = "videos",
            folders = folders
        )
        Log.i(TAG, "scanDeviceFolders: after videos scan — ${folders.size} unique folders (${folders.size - foldersBefore} new from videos)")

        val result = folders.values
            .sortedWith(compareByDescending<DeviceFolder> {
                // Camera first, then by count
                it.relativePath.contains("DCIM/Camera", ignoreCase = true)
            }.thenByDescending { it.mediaCount })

        // Log every discovered folder for diagnostics
        for (folder in result) {
            Log.i(TAG, "  folder: name='${folder.bucketName}' path='${folder.relativePath}' bucketId=${folder.bucketId} count=${folder.mediaCount}")
        }
        Log.i(TAG, "scanDeviceFolders: returning ${result.size} total folders")
        return result
    }

    private fun scanMediaFolders(
        uri: android.net.Uri,
        bucketIdColumn: String,
        bucketNameColumn: String,
        relativePathColumn: String,
        mediaType: String,
        folders: MutableMap<Long, DeviceFolder>
    ) {
        val projection = arrayOf(
            bucketIdColumn,
            bucketNameColumn,
            relativePathColumn
        )

        val cursor = context.contentResolver.query(
            uri, projection, null, null, null
        )

        if (cursor == null) {
            Log.e(TAG, "scanMediaFolders($mediaType): ContentResolver.query returned NULL — this usually means missing permissions")
            return
        }

        cursor.use { c ->
            val totalRows = c.count
            Log.i(TAG, "scanMediaFolders($mediaType): cursor has $totalRows rows for URI=$uri")

            if (totalRows == 0) {
                Log.w(TAG, "scanMediaFolders($mediaType): 0 rows returned — either no media or permissions restrict visibility")
                return
            }

            val bucketIdIdx = c.getColumnIndexOrThrow(bucketIdColumn)
            val bucketNameIdx = c.getColumnIndexOrThrow(bucketNameColumn)
            val relativePathIdx = c.getColumnIndexOrThrow(relativePathColumn)

            var rowCount = 0
            while (c.moveToNext()) {
                val bucketId = c.getLong(bucketIdIdx)
                val bucketName = c.getString(bucketNameIdx) ?: "Unknown"
                val relativePath = c.getString(relativePathIdx) ?: ""

                // Log first 5 rows individually for debugging
                if (rowCount < 5) {
                    Log.d(TAG, "scanMediaFolders($mediaType): row[$rowCount] bucketId=$bucketId name='$bucketName' path='$relativePath'")
                }
                rowCount++

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
            Log.i(TAG, "scanMediaFolders($mediaType): processed $rowCount rows → ${folders.size} unique folders so far")
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

    /**
     * Wipe the folder database and reinitialize from a fresh device scan.
     * Use after a permission change (e.g. partial → full access) or when
     * folder discovery appears broken.
     *
     * Returns the freshly-discovered device folders.
     */
    suspend fun resetAndRescan(): List<DeviceFolder> {
        val diag = getPermissionDiagnostics()
        Log.w(TAG, "resetAndRescan: WIPING folder database — ${diag.toLogString()}")
        db.backupFolderDao().deleteAll()
        Log.i(TAG, "resetAndRescan: folder table cleared, running initializeDefaultsIfNeeded")
        initializeDefaultsIfNeeded()
        val folders = scanDeviceFolders()
        Log.i(TAG, "resetAndRescan: complete — found ${folders.size} folders")
        return folders
    }
}
