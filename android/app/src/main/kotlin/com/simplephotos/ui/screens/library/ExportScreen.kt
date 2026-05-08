/**
 * Export screen — start a full-library export, poll progress, and list
 * previously generated archives. Mirrors the web "Export" page.
 */
package com.simplephotos.ui.screens.library

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.remote.dto.ExportFile
import com.simplephotos.data.remote.dto.ExportStatusResponse
import com.simplephotos.data.repository.ExportRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import javax.inject.Inject
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@HiltViewModel
class ExportViewModel @Inject constructor(private val repo: ExportRepository) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var status by mutableStateOf<ExportStatusResponse?>(null); private set
    var files by mutableStateOf<List<ExportFile>>(emptyList()); private set
    var startingExport by mutableStateOf(false); private set
    var includeMetadata by mutableStateOf(true)
    var decrypt by mutableStateOf(true)
    var stripGeo by mutableStateOf(false)

    init { reload() }

    fun reload() {
        viewModelScope.launch {
            loading = true; error = null
            try {
                status = repo.status()
                files = repo.listFiles()
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }

    fun startExport() {
        viewModelScope.launch {
            startingExport = true; error = null
            try {
                repo.start(
                    scope = "all",
                    includeMetadata = includeMetadata,
                    decrypt = decrypt,
                    stripGeo = stripGeo,
                )
                // Poll status briefly so the UI updates
                repeat(15) {
                    delay(2000)
                    try { status = repo.status() } catch (_: Exception) {}
                    if (status?.state == "completed" || status?.state == "failed") {
                        files = repo.listFiles()
                        return@repeat
                    }
                }
            } catch (e: Exception) { error = e.message }
            startingExport = false
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ExportScreen(onBack: () -> Unit, vm: ExportViewModel = hiltViewModel()) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Export") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier.fillMaxSize().padding(padding).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                "Export your library",
                fontWeight = FontWeight.Bold,
                fontSize = 20.sp,
            )

            // Options
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    OptionRow("Include metadata sidecars", vm.includeMetadata) {
                        vm.includeMetadata = it
                    }
                    OptionRow("Decrypt encrypted blobs", vm.decrypt) { vm.decrypt = it }
                    OptionRow("Strip geo / EXIF GPS", vm.stripGeo) { vm.stripGeo = it }
                }
            }

            // Status / start button
            val st = vm.status
            if (st != null && st.state == "running") {
                Column {
                    Text("Export running… ${st.processedCount}/${st.totalCount}")
                    LinearProgressIndicator(
                        progress = { st.progress.toFloat().coerceIn(0f, 1f) },
                        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    )
                }
            } else {
                Button(
                    onClick = { vm.startExport() },
                    enabled = !vm.startingExport,
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(if (vm.startingExport) "Starting…" else "Start full export") }
            }

            vm.error?.let { Text("Error: $it", color = MaterialTheme.colorScheme.error) }

            HorizontalDivider()

            Text("Previous archives", fontWeight = FontWeight.SemiBold, fontSize = 16.sp)

            if (vm.loading) {
                CircularProgressIndicator(modifier = Modifier.align(Alignment.CenterHorizontally))
            } else if (vm.files.isEmpty()) {
                Text(
                    "No exports yet.",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(vm.files) { f ->
                        Card(modifier = Modifier.fillMaxWidth()) {
                            Column(modifier = Modifier.padding(12.dp)) {
                                Text(f.filename, fontWeight = FontWeight.Medium)
                                Text(
                                    "${f.photoCount} photos · ${f.sizeBytes / (1024 * 1024)} MB · ${f.createdAt}",
                                    fontSize = 12.sp,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun OptionRow(label: String, value: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, modifier = Modifier.weight(1f))
        Switch(checked = value, onCheckedChange = onChange)
    }
}
