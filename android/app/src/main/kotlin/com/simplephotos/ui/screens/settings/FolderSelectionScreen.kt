package com.simplephotos.ui.screens.settings

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.CameraAlt
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Screenshot
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.BackupFolderEntity
import com.simplephotos.data.repository.BackupFolderRepository
import com.simplephotos.data.repository.DeviceFolder
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * ViewModel for the folder selection screen.
 * Scans the device for folders containing media and shows which ones
 * are enabled for automatic backup.
 */
@HiltViewModel
class FolderSelectionViewModel @Inject constructor(
    private val backupFolderRepository: BackupFolderRepository
) : ViewModel() {

    var deviceFolders by mutableStateOf<List<DeviceFolder>>(emptyList())
        private set
    var enabledBucketIds by mutableStateOf<Set<Long>>(emptySet())
        private set
    var loading by mutableStateOf(true)
        private set
    var error by mutableStateOf<String?>(null)
        private set

    // Observe saved folders from the database
    val savedFolders = backupFolderRepository.getSelectedFolders()

    init {
        loadFolders()
    }

    private fun loadFolders() {
        viewModelScope.launch {
            loading = true
            error = null
            try {
                // Initialize defaults if first launch
                backupFolderRepository.initializeDefaultsIfNeeded()

                // Scan device for available folders
                deviceFolders = backupFolderRepository.scanDeviceFolders()

                // Get currently enabled bucket IDs
                enabledBucketIds = backupFolderRepository.getEnabledBucketIds().toSet()
            } catch (e: Exception) {
                error = "Failed to scan folders: ${e.message}"
            } finally {
                loading = false
            }
        }
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
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Backup Folders") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.Default.ArrowBack, contentDescription = "Back")
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
                        Text(
                            viewModel.error ?: "Unknown error",
                            color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.padding(16.dp)
                        )
                    }
                }

                viewModel.deviceFolders.isEmpty() -> {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            "No media folders found on this device.",
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
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
    }
}

@Composable
private fun FolderItem(
    folder: DeviceFolder,
    isEnabled: Boolean,
    onToggle: () -> Unit
) {
    val icon = when {
        folder.relativePath.contains("DCIM/Camera", ignoreCase = true) ||
        folder.bucketName.equals("Camera", ignoreCase = true) -> Icons.Default.CameraAlt
        folder.relativePath.contains("Screenshots", ignoreCase = true) ||
        folder.bucketName.equals("Screenshots", ignoreCase = true) -> Icons.Default.Screenshot
        folder.relativePath.contains("Video", ignoreCase = true) ||
        folder.bucketName.contains("Video", ignoreCase = true) -> Icons.Default.Videocam
        folder.relativePath.contains("Pictures", ignoreCase = true) ||
        folder.relativePath.contains("Images", ignoreCase = true) -> Icons.Default.Image
        else -> Icons.Default.Folder
    }

    val isCamera = folder.relativePath.contains("DCIM/Camera", ignoreCase = true) ||
        folder.bucketName.equals("Camera", ignoreCase = true)

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
            Icon(
                icon,
                contentDescription = null,
                modifier = Modifier.size(24.dp),
                tint = if (isEnabled) MaterialTheme.colorScheme.primary
                       else MaterialTheme.colorScheme.onSurfaceVariant
            )
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
