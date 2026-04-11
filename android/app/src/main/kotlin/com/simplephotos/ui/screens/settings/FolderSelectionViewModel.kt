package com.simplephotos.ui.screens.settings

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.repository.BackupFolderRepository
import com.simplephotos.data.repository.DeviceFolder
import com.simplephotos.data.repository.PermissionDiagnostics
import com.simplephotos.data.repository.SyncRepository
import com.simplephotos.sync.SyncScheduler
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * ViewModel for the folder selection screen.
 * Scans the device for folders containing media and shows which ones
 * are enabled for automatic backup.
 *
 * Uses direct ContextCompat.checkSelfPermission() instead of Accompanist
 * to avoid the Android 14+ Accompanist permission-reporting bug.
 */
@HiltViewModel
class FolderSelectionViewModel @Inject constructor(
    private val backupFolderRepository: BackupFolderRepository,
    private val syncRepository: SyncRepository,
    @dagger.hilt.android.qualifiers.ApplicationContext private val appContext: android.content.Context
) : ViewModel() {

    var deviceFolders by mutableStateOf<List<DeviceFolder>>(emptyList())
        private set
    var enabledBucketIds by mutableStateOf<Set<Long>>(emptySet())
        private set
    /** Snapshot of enabled IDs when the screen was loaded — used to diff
     *  changes on exit and only persist the delta. */
    private var originalEnabledBucketIds: Set<Long> = emptySet()
    /** True while the deferred save is being written. */
    var saving by mutableStateOf(false)
        private set
    var loading by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var diagnostics by mutableStateOf<PermissionDiagnostics?>(null)
        private set
    var showDiagnosticPanel by mutableStateOf(false)
        private set
    /** True when the post-scan heuristic detects probable partial access
     *  despite checkSelfPermission reporting full access (Android 14 bug). */
    var likelyPartialAccess by mutableStateOf(false)
        private set

    // Observe saved folders from the database
    val savedFolders = backupFolderRepository.getSelectedFolders()

    init {
        // Don't auto-scan — permissions may not be granted yet.
        // The screen will call scanFolders() after permissions are confirmed.
    }

    /** Refresh the diagnostic snapshot (call on resume). */
    fun refreshDiagnostics() {
        diagnostics = backupFolderRepository.getPermissionDiagnostics()
        android.util.Log.i("FolderSelection", "refreshDiagnostics: ${diagnostics?.toLogString()}")
    }

    /** Called by the Screen once permissions have been confirmed via direct system check. */
    fun scanFolders() {
        if (loading) return           // already scanning
        viewModelScope.launch {
            loading = true
            error = null
            try {
                // Refresh diagnostics
                diagnostics = backupFolderRepository.getPermissionDiagnostics()
                android.util.Log.i("FolderSelection", "scanFolders: ${diagnostics?.toLogString()}")

                // Initialize defaults if first launch
                backupFolderRepository.initializeDefaultsIfNeeded()

                // Scan device for available folders
                deviceFolders = backupFolderRepository.scanDeviceFolders()
                android.util.Log.i("FolderSelection", "scanFolders: found ${deviceFolders.size} folders")

                // Get currently enabled bucket IDs
                enabledBucketIds = backupFolderRepository.getEnabledBucketIds().toSet()
                originalEnabledBucketIds = enabledBucketIds
                android.util.Log.i("FolderSelection", "scanFolders: ${enabledBucketIds.size} folders enabled, bucketIds=$enabledBucketIds")

                // Post-scan heuristic: detect the Android 14 platform bug
                // where checkSelfPermission(READ_MEDIA_IMAGES) returns GRANTED
                // even under partial ("Select photos") access (b/308531058).
                likelyPartialAccess = BackupFolderRepository.likelyPartialAccessDespitePermissions(
                    appContext, deviceFolders.size
                )
                if (likelyPartialAccess) {
                    android.util.Log.w("FolderSelection",
                        "scanFolders: HEURISTIC — only ${deviceFolders.size} folder(s) found " +
                        "with VISUAL_USER_SELECTED granted on API 34+. " +
                        "Probable partial access despite checkSelfPermission reporting full access.")
                } else if (deviceFolders.size <= 1) {
                    android.util.Log.w("FolderSelection", "scanFolders: WARNING — only ${deviceFolders.size} folder(s) found. Likely a permission issue.")
                    // Auto-show diagnostic panel when results look wrong
                    showDiagnosticPanel = true
                }
            } catch (e: Exception) {
                android.util.Log.e("FolderSelection", "scanFolders failed", e)
                error = "Failed to scan folders: ${e.message}"
            } finally {
                loading = false
            }
        }
    }

    /**
     * Wipe the folder DB and rescan from scratch.
     * Use when folder discovery appears broken (e.g. after permission changes).
     */
    fun resetAndRescan() {
        viewModelScope.launch {
            loading = true
            error = null
            showDiagnosticPanel = false
            try {
                diagnostics = backupFolderRepository.getPermissionDiagnostics()
                android.util.Log.w("FolderSelection", "resetAndRescan: ${diagnostics?.toLogString()}")

                deviceFolders = backupFolderRepository.resetAndRescan()
                enabledBucketIds = backupFolderRepository.getEnabledBucketIds().toSet()
                originalEnabledBucketIds = enabledBucketIds

                android.util.Log.i("FolderSelection", "resetAndRescan: found ${deviceFolders.size} folders, ${enabledBucketIds.size} enabled")

                likelyPartialAccess = BackupFolderRepository.likelyPartialAccessDespitePermissions(
                    appContext, deviceFolders.size
                )
                if (likelyPartialAccess) {
                    android.util.Log.w("FolderSelection",
                        "resetAndRescan: HEURISTIC — probable partial access despite permissions (${deviceFolders.size} folder(s))")
                } else if (deviceFolders.size <= 1) {
                    showDiagnosticPanel = true
                }
            } catch (e: Exception) {
                android.util.Log.e("FolderSelection", "resetAndRescan failed", e)
                error = "Failed to reset & rescan: ${e.message}"
            } finally {
                loading = false
            }
        }
    }

    fun toggleDiagnosticPanel() {
        showDiagnosticPanel = !showDiagnosticPanel
    }

    fun toggleFolder(folder: DeviceFolder) {
        // Only update in-memory state — changes are persisted when
        // the user leaves the screen via applyPendingChanges().
        val isCurrentlyEnabled = folder.bucketId in enabledBucketIds
        enabledBucketIds = if (isCurrentlyEnabled) {
            enabledBucketIds - folder.bucketId
        } else {
            enabledBucketIds + folder.bucketId
        }
    }

    /**
     * Persist any folder toggle changes to the DB and trigger sync for
     * newly-enabled folders. Called when the user navigates away.
     *
     * Uses NonCancellable to ensure the coroutine completes even when
     * the ViewModel scope is cancelled as the screen is popped.
     */
    fun applyPendingChanges() {
        val newlyEnabled = enabledBucketIds - originalEnabledBucketIds
        val newlyDisabled = originalEnabledBucketIds - enabledBucketIds
        if (newlyEnabled.isEmpty() && newlyDisabled.isEmpty()) return

        saving = true
        viewModelScope.launch(kotlinx.coroutines.NonCancellable) {
            try {
                // Persist all changes to the DB
                for (folder in deviceFolders) {
                    if (folder.bucketId in newlyEnabled || folder.bucketId in newlyDisabled) {
                        val enabled = folder.bucketId in enabledBucketIds
                        backupFolderRepository.setFolderEnabled(folder, enabled)
                    }
                }

                // Scan and trigger backup for newly-enabled folders
                if (newlyEnabled.isNotEmpty()) {
                    syncRepository.fullScanForBuckets(newlyEnabled.toList())
                    SyncScheduler.triggerNow(appContext)
                }

                originalEnabledBucketIds = enabledBucketIds
                android.util.Log.i("FolderSelection",
                    "applyPendingChanges: enabled=${newlyEnabled.size}, disabled=${newlyDisabled.size}")
            } catch (e: Exception) {
                error = "Failed to save folder changes: ${e.message}"
            } finally {
                saving = false
            }
        }
    }
}
