package com.simplephotos.ui.screens.diagnostics

import android.content.Context
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_DIAGNOSTIC_LOGGING
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

// ── ViewModel ────────────────────────────────────────────────────────────────

@HiltViewModel
class DiagnosticsViewModel @Inject constructor(
    private val api: ApiService,
    private val db: AppDatabase,
    val dataStore: DataStore<Preferences>,
    @ApplicationContext private val appContext: Context
) : ViewModel() {

    var serverUrl by mutableStateOf("")
    var username by mutableStateOf("")
    var diagnosticLoggingEnabled by mutableStateOf(false)

    // Connection
    var serverReachable by mutableStateOf<Boolean?>(null)
    var serverLatencyMs by mutableStateOf<Long?>(null)
    var checkingConnection by mutableStateOf(false)

    // Photo counts
    var totalPhotos by mutableStateOf(0)
    var syncedCount by mutableStateOf(0)
    var pendingCount by mutableStateOf(0)
    var failedCount by mutableStateOf(0)
    var uploadingCount by mutableStateOf(0)

    // Device info
    val deviceModel = "${Build.MANUFACTURER} ${Build.MODEL}"
    val androidVersion = "Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})"

    var appVersion by mutableStateOf("")

    var loading by mutableStateOf(true)

    init {
        viewModelScope.launch {
            try {
                val prefs = dataStore.data.first()
                serverUrl = prefs[KEY_SERVER_URL] ?: ""
                username = prefs[KEY_USERNAME] ?: ""
                diagnosticLoggingEnabled = prefs[KEY_DIAGNOSTIC_LOGGING] ?: false

                // App version
                try {
                    val pInfo = appContext.packageManager.getPackageInfo(appContext.packageName, 0)
                    appVersion = pInfo.versionName ?: "unknown"
                } catch (_: Exception) {
                    appVersion = "unknown"
                }

                // Photo counts
                loadPhotoCounts()

                // Server check
                checkConnection()
            } finally {
                loading = false
            }
        }
    }

    private suspend fun loadPhotoCounts() = withContext(Dispatchers.IO) {
        val dao = db.photoDao()
        val all = dao.getByStatus(SyncStatus.SYNCED)
        syncedCount = all.size
        val pending = dao.getByStatus(SyncStatus.PENDING)
        pendingCount = pending.size
        val failed = dao.getByStatus(SyncStatus.FAILED)
        failedCount = failed.size
        val uploading = dao.getByStatus(SyncStatus.UPLOADING)
        uploadingCount = uploading.size
        totalPhotos = syncedCount + pendingCount + failedCount + uploadingCount
    }

    fun checkConnection() {
        viewModelScope.launch {
            checkingConnection = true
            try {
                val start = System.currentTimeMillis()
                withContext(Dispatchers.IO) { api.health() }
                serverLatencyMs = System.currentTimeMillis() - start
                serverReachable = true
            } catch (_: Exception) {
                serverReachable = false
                serverLatencyMs = null
            } finally {
                checkingConnection = false
            }
        }
    }
}

// ── Screen ───────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DiagnosticsScreen(
    onBack: () -> Unit,
    viewModel: DiagnosticsViewModel = hiltViewModel()
) {
    val scrollState = rememberScrollState()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Diagnostics", fontSize = 18.sp) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back")
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = Color(0xFF111111),
                    titleContentColor = Color.White,
                    navigationIconContentColor = Color.White
                )
            )
        },
        containerColor = Color(0xFF111111)
    ) { padding ->
        if (viewModel.loading) {
            Box(
                modifier = Modifier.fillMaxSize().padding(padding),
                contentAlignment = Alignment.Center
            ) {
                CircularProgressIndicator(color = Color.White)
            }
        } else {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .verticalScroll(scrollState)
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                // ── Server Connection ────────────────────────────────────
                DiagnosticCard(title = "Server Connection") {
                    DiagRow("URL", viewModel.serverUrl.ifEmpty { "Not configured" })
                    DiagRow("Status", buildString {
                        when (viewModel.serverReachable) {
                            true -> append("Connected")
                            false -> append("Unreachable")
                            null -> append("Checking…")
                        }
                        viewModel.serverLatencyMs?.let { append("  (${it}ms)") }
                    })
                    StatusDot(
                        color = when (viewModel.serverReachable) {
                            true -> Color(0xFF22C55E)
                            false -> Color(0xFFEF4444)
                            null -> Color.Gray
                        },
                        label = when (viewModel.serverReachable) {
                            true -> "Healthy"
                            false -> "Error"
                            null -> "Unknown"
                        }
                    )
                    Spacer(Modifier.height(4.dp))
                    TextButton(
                        onClick = { viewModel.checkConnection() },
                        enabled = !viewModel.checkingConnection
                    ) {
                        Text(
                            if (viewModel.checkingConnection) "Checking…" else "Re-check connection",
                            color = Color(0xFF3B82F6)
                        )
                    }
                }

                // ── Account ──────────────────────────────────────────────
                DiagnosticCard(title = "Account") {
                    DiagRow("Username", viewModel.username.ifEmpty { "—" })
                    DiagRow("Diagnostic logging",
                        if (viewModel.diagnosticLoggingEnabled) "Enabled" else "Disabled"
                    )
                }

                // ── Library Stats ────────────────────────────────────────
                DiagnosticCard(title = "Library") {
                    DiagRow("Total photos", viewModel.totalPhotos.toString())
                    DiagRow("Synced", viewModel.syncedCount.toString())
                    DiagRow("Pending upload", viewModel.pendingCount.toString())
                    if (viewModel.uploadingCount > 0) {
                        DiagRow("Currently uploading", viewModel.uploadingCount.toString())
                    }
                    if (viewModel.failedCount > 0) {
                        DiagRow("Failed", viewModel.failedCount.toString())
                    }
                }

                // ── Device Info ──────────────────────────────────────────
                DiagnosticCard(title = "Device") {
                    DiagRow("Model", viewModel.deviceModel)
                    DiagRow("Android", viewModel.androidVersion)
                    DiagRow("App version", viewModel.appVersion)
                }

                Spacer(Modifier.height(24.dp))
            }
        }
    }
}

// ── Sub-components ───────────────────────────────────────────────────────────

@Composable
private fun DiagnosticCard(title: String, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(Color(0xFF1A1A1A))
            .padding(16.dp)
    ) {
        Text(
            title,
            color = Color.White,
            fontSize = 15.sp,
            fontWeight = FontWeight.SemiBold
        )
        Spacer(Modifier.height(10.dp))
        content()
    }
}

@Composable
private fun DiagRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 3.dp),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(label, color = Color.Gray, fontSize = 13.sp)
        Text(
            value,
            color = Color.White,
            fontSize = 13.sp,
            fontFamily = FontFamily.Monospace
        )
    }
}

@Composable
private fun StatusDot(color: Color, label: String) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.padding(top = 6.dp)
    ) {
        Box(
            modifier = Modifier
                .size(10.dp)
                .clip(CircleShape)
                .background(color)
        )
        Spacer(Modifier.width(8.dp))
        Text(label, color = color, fontSize = 13.sp, fontWeight = FontWeight.Medium)
    }
}
