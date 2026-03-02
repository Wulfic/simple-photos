package com.simplephotos.ui.screens.settings

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CameraAlt
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Screenshot
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.BackupFolderEntity
import com.simplephotos.data.repository.BackupFolderRepository
import com.simplephotos.data.repository.DeviceFolder
import com.simplephotos.data.repository.PermissionDiagnostics
import com.simplephotos.data.repository.SyncRepository
import com.simplephotos.sync.SyncScheduler
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.launch
import javax.inject.Inject
import com.simplephotos.R
import androidx.compose.ui.res.painterResource

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
        val isCurrentlyEnabled = folder.bucketId in enabledBucketIds
        val newEnabled = !isCurrentlyEnabled

        viewModelScope.launch {
            try {
                backupFolderRepository.setFolderEnabled(folder, newEnabled)
                enabledBucketIds = if (newEnabled) {
                    enabledBucketIds + folder.bucketId
                } else {
                    enabledBucketIds - folder.bucketId
                }

                // When a folder is newly enabled, scan ALL its existing photos
                // and immediately trigger a backup so they get uploaded.
                if (newEnabled) {
                    syncRepository.fullScanForBuckets(listOf(folder.bucketId))
                    SyncScheduler.triggerNow(appContext)
                }
            } catch (e: Exception) {
                error = "Failed to update folder: ${e.message}"
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FolderSelectionScreen(
    onBack: () -> Unit,
    viewModel: FolderSelectionViewModel = hiltViewModel()
) {
    val context = LocalContext.current

    // ── Direct system permission checks (NOT Accompanist) ──────────────
    // This avoids the Accompanist 0.34.0 bug on Android 14+ where
    // allPermissionsGranted can be true with only partial access.
    var permCheckTrigger by remember { mutableIntStateOf(0) }

    val hasFullAccess = remember(permCheckTrigger) {
        BackupFolderRepository.hasFullMediaAccess(context)
    }

    val hasPartialAccess = remember(permCheckTrigger, hasFullAccess) {
        BackupFolderRepository.hasPartialMediaAccess(context)
    }

    // Re-check permissions on resume (e.g. returning from system Settings)
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                permCheckTrigger++
                viewModel.refreshDiagnostics()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    // When any media permissions are confirmed, trigger the scan.
    // Also re-scan if permCheckTrigger changes (user returned from Settings).
    val hasAnyAccess = hasFullAccess || hasPartialAccess
    LaunchedEffect(hasAnyAccess, permCheckTrigger) {
        if (hasAnyAccess) {
            viewModel.scanFolders()
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Backup Folders") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(painter = painterResource(R.drawable.ic_back_arrow), contentDescription = "Back", modifier = Modifier.size(24.dp))
                    }
                },
                actions = {
                    // Rescan button — available when we have any access
                    if (hasFullAccess || hasPartialAccess) {
                        IconButton(onClick = { viewModel.resetAndRescan() }) {
                            Icon(Icons.Default.Refresh, contentDescription = "Rescan folders")
                        }
                    }
                }
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
        ) {
            when {
                hasFullAccess || hasPartialAccess -> {
                    // ── Any access — show folder list with inline warning
                    //    when results look incomplete ───────────────────
                    FolderListContent(viewModel, context)
                }
                else -> {
                    // ── No access — shouldn't reach here (PermissionGate
                    //    in MainActivity blocks), but handle gracefully ──
                    NoAccessBanner(context)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// No-access fallback (shouldn't normally appear — PermissionGate blocks first)
// ---------------------------------------------------------------------------

@Composable
private fun NoAccessBanner(context: android.content.Context) {
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        Card(
            modifier = Modifier
                .fillMaxWidth()
                .padding(24.dp),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant
            )
        ) {
            Column(
                modifier = Modifier.padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Icon(
                    painter = painterResource(R.drawable.ic_image),
                    contentDescription = null,
                    modifier = Modifier.size(48.dp),
                    tint = MaterialTheme.colorScheme.primary
                )
                Spacer(Modifier.height(16.dp))
                Text(
                    "Media access required",
                    style = MaterialTheme.typography.titleMedium,
                    textAlign = TextAlign.Center
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    "Simple Photos needs access to your photos and videos to discover folders available for backup.\n\n" +
                        "Please grant full media access in system Settings.",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(20.dp))
                Button(onClick = {
                    val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                        data = Uri.fromParts("package", context.packageName, null)
                    }
                    context.startActivity(intent)
                }) {
                    Text("Open Settings")
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Partial access UI (Android 14+ "Select photos" chosen instead of "Allow all")
// ---------------------------------------------------------------------------

@Composable
private fun PartialAccessBanner(context: android.content.Context) {
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        Card(
            modifier = Modifier
                .fillMaxWidth()
                .padding(24.dp),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.errorContainer
            )
        ) {
            Column(
                modifier = Modifier.padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Icon(
                    painter = painterResource(R.drawable.ic_image),
                    contentDescription = null,
                    modifier = Modifier.size(48.dp),
                    tint = MaterialTheme.colorScheme.error
                )
                Spacer(Modifier.height(16.dp))
                Text(
                    "Full access required",
                    style = MaterialTheme.typography.titleMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onErrorContainer
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    "You've granted partial photo access (\"Select photos\"). " +
                        "Simple Photos needs full access to discover all folders on your device.\n\n" +
                        "Open Settings \u2192 Permissions \u2192 Photos and Videos \u2192 choose \"Allow all\".",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onErrorContainer.copy(alpha = 0.85f)
                )
                Spacer(Modifier.height(20.dp))
                Button(
                    onClick = {
                        val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                            data = Uri.fromParts("package", context.packageName, null)
                        }
                        context.startActivity(intent)
                    },
                    colors = ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.error
                    )
                ) {
                    Text("Open Settings")
                }
                Spacer(Modifier.height(8.dp))
                Text(
                    "After changing the permission, return here and it will rescan automatically.",
                    style = MaterialTheme.typography.bodySmall,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onErrorContainer.copy(alpha = 0.7f)
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Folder list content (shown after permission is granted)
// ---------------------------------------------------------------------------

@Composable
private fun FolderListContent(
    viewModel: FolderSelectionViewModel,
    context: android.content.Context
) {
    // Header info
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(16.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer
        )
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                "Choose folders to back up",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onPrimaryContainer
            )
            Spacer(Modifier.height(4.dp))
            Text(
                "Photos and videos in selected folders will be automatically encrypted and backed up to your server.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.8f)
            )
        }
    }

    // ── Diagnostic panel — auto-shown when few folders found ───────────
    val diag = viewModel.diagnostics
    AnimatedVisibility(visible = viewModel.showDiagnosticPanel && diag != null) {
        if (diag != null) {
            DiagnosticPanel(diag, viewModel, context)
        }
    }

    // Toggle diagnostic panel link
    if (diag != null && !viewModel.loading) {
        TextButton(
            onClick = { viewModel.toggleDiagnosticPanel() },
            modifier = Modifier.padding(horizontal = 16.dp)
        ) {
            Icon(
                Icons.Default.Warning,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
                tint = if (viewModel.deviceFolders.size <= 1) MaterialTheme.colorScheme.error
                       else MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.width(4.dp))
            Text(
                if (viewModel.showDiagnosticPanel) "Hide diagnostics"
                else "Show diagnostics (${viewModel.deviceFolders.size} folders found)",
                style = MaterialTheme.typography.labelSmall,
                color = if (viewModel.deviceFolders.size <= 1) MaterialTheme.colorScheme.error
                        else MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }

    when {
        viewModel.loading -> {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    CircularProgressIndicator()
                    Spacer(Modifier.height(16.dp))
                    Text("Scanning device folders...")
                }
            }
        }

        viewModel.error != null -> {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        viewModel.error ?: "Unknown error",
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.padding(16.dp)
                    )
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(onClick = { viewModel.resetAndRescan() }) {
                        Text("Retry")
                    }
                }
            }
        }

        viewModel.deviceFolders.isEmpty() -> {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        "No media folders found on this device.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(12.dp))
                    OutlinedButton(onClick = { viewModel.resetAndRescan() }) {
                        Icon(Icons.Default.Refresh, contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(4.dp))
                        Text("Reset & Rescan")
                    }
                }
            }
        }

        else -> {
            // Enabled count
            val enabledCount = viewModel.enabledBucketIds.size
            Text(
                "$enabledCount folder${if (enabledCount != 1) "s" else ""} selected for backup",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)
            )

            LazyColumn(
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp)
            ) {
                items(
                    viewModel.deviceFolders,
                    key = { it.bucketId }
                ) { folder ->
                    FolderItem(
                        folder = folder,
                        isEnabled = folder.bucketId in viewModel.enabledBucketIds,
                        onToggle = { viewModel.toggleFolder(folder) }
                    )
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Diagnostic panel — shows raw permission state for debugging
// ---------------------------------------------------------------------------

@Composable
private fun DiagnosticPanel(
    diag: PermissionDiagnostics,
    viewModel: FolderSelectionViewModel,
    context: android.content.Context
) {
    val isLikelyBroken = !diag.hasFullAccess || viewModel.deviceFolders.size <= 1

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
        colors = CardDefaults.cardColors(
            containerColor = if (isLikelyBroken) MaterialTheme.colorScheme.errorContainer
                             else MaterialTheme.colorScheme.surfaceVariant
        )
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Text(
                "Permission Diagnostics",
                style = MaterialTheme.typography.titleSmall,
                color = if (isLikelyBroken) MaterialTheme.colorScheme.onErrorContainer
                        else MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.height(8.dp))

            val textColor = if (isLikelyBroken) MaterialTheme.colorScheme.onErrorContainer
                            else MaterialTheme.colorScheme.onSurfaceVariant

            Text("Android API: ${diag.apiLevel}", style = MaterialTheme.typography.bodySmall, color = textColor)
            diag.readMediaImages?.let {
                val status = if (it) "\u2705 GRANTED" else "\u274C DENIED"
                Text("READ_MEDIA_IMAGES: $status", style = MaterialTheme.typography.bodySmall, color = textColor)
            }
            diag.readMediaVideo?.let {
                val status = if (it) "\u2705 GRANTED" else "\u274C DENIED"
                Text("READ_MEDIA_VIDEO: $status", style = MaterialTheme.typography.bodySmall, color = textColor)
            }
            diag.readMediaVisualUserSelected?.let {
                val status = if (it) "\u2705 GRANTED" else "\u274C DENIED"
                Text("VISUAL_USER_SELECTED: $status", style = MaterialTheme.typography.bodySmall, color = textColor)
            }
            diag.readExternalStorage?.let {
                val status = if (it) "\u2705 GRANTED" else "\u274C DENIED"
                Text("READ_EXTERNAL_STORAGE: $status", style = MaterialTheme.typography.bodySmall, color = textColor)
            }

            Spacer(Modifier.height(4.dp))
            Text(
                "Full access: ${if (diag.hasFullAccess) "YES" else "NO"}" +
                    "  |  Partial access: ${if (diag.hasPartialAccess) "YES" else "NO"}",
                style = MaterialTheme.typography.bodySmall,
                color = textColor
            )
            Text(
                "Folders found: ${viewModel.deviceFolders.size}",
                style = MaterialTheme.typography.bodySmall,
                color = textColor
            )

            if (isLikelyBroken) {
                Spacer(Modifier.height(8.dp))
                if (diag.hasPartialAccess || diag.readMediaImages == false) {
                    Text(
                        "\u26A0 You likely have partial access (\"Select photos\"). " +
                            "Go to Settings \u2192 Permissions \u2192 Photos & Videos \u2192 \"Allow all\".",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error
                    )
                } else {
                    Text(
                        "\u26A0 Permissions look correct but few folders were found. " +
                            "Try \"Reset & Rescan\" below.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error
                    )
                }
            }

            Spacer(Modifier.height(12.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                OutlinedButton(
                    onClick = { viewModel.resetAndRescan() },
                    modifier = Modifier.weight(1f)
                ) {
                    Icon(Icons.Default.Refresh, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text("Reset & Rescan", style = MaterialTheme.typography.labelSmall)
                }
                OutlinedButton(
                    onClick = {
                        val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                            data = Uri.fromParts("package", context.packageName, null)
                        }
                        context.startActivity(intent)
                    },
                    modifier = Modifier.weight(1f)
                ) {
                    Text("Open Settings", style = MaterialTheme.typography.labelSmall)
                }
            }
        }
    }
}

@Composable
private fun FolderItem(
    folder: DeviceFolder,
    isEnabled: Boolean,
    onToggle: () -> Unit
) {
    val isCamera = folder.relativePath.contains("DCIM/Camera", ignoreCase = true) ||
        folder.bucketName.equals("Camera", ignoreCase = true)

    val iconTint = if (isEnabled) MaterialTheme.colorScheme.primary
                   else MaterialTheme.colorScheme.onSurfaceVariant

    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.medium,
        tonalElevation = if (isEnabled) 2.dp else 0.dp
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            when {
                folder.relativePath.contains("DCIM/Camera", ignoreCase = true) ||
                folder.bucketName.equals("Camera", ignoreCase = true) ->
                    Icon(Icons.Default.CameraAlt, contentDescription = null, modifier = Modifier.size(24.dp), tint = iconTint)
                folder.relativePath.contains("Screenshots", ignoreCase = true) ||
                folder.bucketName.equals("Screenshots", ignoreCase = true) ->
                    Icon(Icons.Default.Screenshot, contentDescription = null, modifier = Modifier.size(24.dp), tint = iconTint)
                folder.relativePath.contains("Video", ignoreCase = true) ||
                folder.bucketName.contains("Video", ignoreCase = true) ->
                    Icon(Icons.Default.Videocam, contentDescription = null, modifier = Modifier.size(24.dp), tint = iconTint)
                folder.relativePath.contains("Pictures", ignoreCase = true) ||
                folder.relativePath.contains("Images", ignoreCase = true) ->
                    Icon(painter = painterResource(R.drawable.ic_image), contentDescription = null, modifier = Modifier.size(24.dp), tint = iconTint)
                else ->
                    Icon(painter = painterResource(R.drawable.ic_folder), contentDescription = null, modifier = Modifier.size(24.dp), tint = iconTint)
            }
            Spacer(Modifier.width(16.dp))
            Column(modifier = Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        folder.bucketName,
                        style = MaterialTheme.typography.bodyLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    if (isCamera) {
                        Spacer(Modifier.width(8.dp))
                        Surface(
                            shape = MaterialTheme.shapes.extraSmall,
                            color = MaterialTheme.colorScheme.secondaryContainer
                        ) {
                            Text(
                                "Default",
                                style = MaterialTheme.typography.labelSmall,
                                modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                                color = MaterialTheme.colorScheme.onSecondaryContainer
                            )
                        }
                    }
                }
                Text(
                    "${folder.relativePath}  •  ${folder.mediaCount} items",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
            Spacer(Modifier.width(8.dp))
            Switch(
                checked = isEnabled,
                onCheckedChange = { onToggle() }
            )
        }
    }
}
