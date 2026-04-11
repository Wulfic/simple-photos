/**
 * Composable screen for initial server connection setup and configuration.
 */
package com.simplephotos.ui.screens.setup

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.R
import com.simplephotos.data.remote.DiscoveredServer
import com.simplephotos.data.remote.ServerDiscovery
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_CONFIGURED
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import com.simplephotos.ui.theme.ThemeToggleButton
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import android.content.Context
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * Handles first-run server configuration: LAN auto-discovery via [ServerDiscovery]
 * and manual URL entry. Persists the chosen server URL to DataStore.
 */
@HiltViewModel
class ServerSetupViewModel @Inject constructor(
    val dataStore: DataStore<Preferences>,
    @ApplicationContext private val appContext: Context
) : ViewModel() {
    var serverUrl by mutableStateOf("")
    var loading by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var scanning by mutableStateOf(false)
    var discoveredServers by mutableStateOf<List<DiscoveredServer>>(emptyList())
    var showManualEntry by mutableStateOf(false)

    init {
        scanNetwork()
    }

    /**
     * Normalize user-entered URL: ensure it has a protocol prefix and no trailing slash.
     */
    private fun normalizeUrl(input: String): String {
        var url = input.trim().trimEnd('/')
        if (url.isNotEmpty() && !url.startsWith("http://") && !url.startsWith("https://")) {
            url = "http://$url"
        }
        return url
    }

    fun scanNetwork() {
        viewModelScope.launch {
            scanning = true
            error = null
            discoveredServers = emptyList()
            try {
                discoveredServers = ServerDiscovery.discover(appContext)
                    .filter { it.mode == "primary" }
            } catch (e: Exception) {
                error = "Scan failed: ${e.message}"
            } finally {
                scanning = false
            }
        }
    }

    fun testAndSave(onSuccess: () -> Unit) {
        viewModelScope.launch {
            loading = true
            error = null
            try {
                val url = normalizeUrl(serverUrl)
                val (statusCode, body) = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                    val conn = java.net.URL("$url/health").openConnection() as java.net.HttpURLConnection
                    conn.connectTimeout = 5000
                    conn.readTimeout = 5000
                    conn.setRequestProperty("X-Requested-With", "SimplePhotos")
                    val code = conn.responseCode
                    val responseBody = if (code == 200) {
                        conn.inputStream.bufferedReader().readText()
                    } else null
                    conn.disconnect()
                    Pair(code, responseBody)
                }
                if (statusCode != 200) {
                    error = "Server returned $statusCode"
                    return@launch
                }
                val json = org.json.JSONObject(body ?: "{}")
                if (json.optString("service") != "simple-photos") {
                    error = "Not a Simple Photos server"
                    return@launch
                }

                dataStore.edit { prefs ->
                    prefs[KEY_SERVER_URL] = url
                    prefs[KEY_SERVER_CONFIGURED] = true
                }
                onSuccess()
            } catch (e: Exception) {
                val msg = e.message ?: e.javaClass.simpleName
                android.util.Log.e("ServerSetup", "Connection failed to ${normalizeUrl(serverUrl)}", e)
                error = "Cannot connect: $msg"
            } finally {
                loading = false
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ServerSetupScreen(
    onSetupComplete: () -> Unit,
    viewModel: ServerSetupViewModel = hiltViewModel()
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = {},
                actions = {
                    ThemeToggleButton(dataStore = viewModel.dataStore)
                }
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Spacer(Modifier.height(16.dp))

            // Logo
            Image(
                painter = painterResource(id = R.drawable.logo),
                contentDescription = "Simple Photos",
                modifier = Modifier.size(72.dp)
            )
            Spacer(Modifier.height(12.dp))
            Text(
                "Simple Photos",
                style = MaterialTheme.typography.headlineMedium
            )
            Spacer(Modifier.height(4.dp))
            Text(
                "End-to-end encrypted photo library",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

            Spacer(Modifier.height(32.dp))

            // Network scan section
            Card(
                modifier = Modifier.fillMaxWidth()
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(
                                Icons.Default.Wifi,
                                contentDescription = null,
                                modifier = Modifier.size(20.dp),
                                tint = MaterialTheme.colorScheme.primary
                            )
                            Spacer(Modifier.width(8.dp))
                            Text(
                                "Servers on your network",
                                style = MaterialTheme.typography.titleSmall
                            )
                        }
                        IconButton(
                            onClick = { viewModel.scanNetwork() },
                            enabled = !viewModel.scanning,
                            modifier = Modifier.size(24.dp)
                        ) {
                            Icon(
                                painter = painterResource(R.drawable.ic_reload),
                                contentDescription = "Rescan",
                                modifier = Modifier.size(12.dp)
                            )
                        }
                    }

                    if (viewModel.scanning) {
                        Spacer(Modifier.height(16.dp))
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.Center,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(16.dp),
                                strokeWidth = 2.dp
                            )
                            Spacer(Modifier.width(12.dp))
                            Text(
                                "Scanning local network...",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    } else if (viewModel.discoveredServers.isEmpty() && !viewModel.scanning) {
                        Spacer(Modifier.height(12.dp))
                        Text(
                            "No servers found on your network.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                            modifier = Modifier.fillMaxWidth()
                        )
                    } else {
                        Spacer(Modifier.height(8.dp))
                        viewModel.discoveredServers.forEach { server ->
                            val isSelected = viewModel.serverUrl == server.url
                            Surface(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable {
                                        viewModel.serverUrl = server.url
                                    },
                                shape = MaterialTheme.shapes.medium,
                                tonalElevation = if (isSelected) 4.dp else 0.dp,
                                color = if (isSelected)
                                    MaterialTheme.colorScheme.primaryContainer
                                else
                                    MaterialTheme.colorScheme.surface
                            ) {
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .padding(12.dp),
                                    verticalAlignment = Alignment.CenterVertically
                                ) {
                                    Column(modifier = Modifier.weight(1f)) {
                                        Text(
                                            server.host,
                                            style = MaterialTheme.typography.bodyLarge
                                        )
                                        Text(
                                            "${server.url}  •  v${server.version}",
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant
                                        )
                                    }
                                    if (isSelected) {
                                        Icon(
                                            Icons.Default.Check,
                                            contentDescription = "Selected",
                                            tint = MaterialTheme.colorScheme.primary,
                                            modifier = Modifier.size(20.dp)
                                        )
                                    }
                                }
                            }
                            Spacer(Modifier.height(4.dp))
                        }
                    }
                }
            }

            Spacer(Modifier.height(16.dp))

            // Manual entry toggle
            TextButton(
                onClick = { viewModel.showManualEntry = !viewModel.showManualEntry }
            ) {
                Text(
                    if (viewModel.showManualEntry) "Hide manual entry" else "Enter server address manually"
                )
            }

            AnimatedVisibility(visible = viewModel.showManualEntry) {
                Column {
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = viewModel.serverUrl,
                        onValueChange = { viewModel.serverUrl = it },
                        label = { Text("Server URL") },
                        placeholder = { Text("http://192.168.1.100:8080") },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true
                    )
                }
            }

            viewModel.error?.let { err ->
                Spacer(Modifier.height(8.dp))
                Text(
                    err,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall
                )
            }

            Spacer(Modifier.height(16.dp))

            Button(
                onClick = { viewModel.testAndSave(onSetupComplete) },
                enabled = !viewModel.loading && viewModel.serverUrl.isNotBlank(),
                modifier = Modifier.fillMaxWidth()
            ) {
                if (viewModel.loading) {
                    CircularProgressIndicator(
                        Modifier.size(20.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary
                    )
                    Spacer(Modifier.width(8.dp))
                    Text("Connecting...")
                } else {
                    Text("Connect")
                }
            }
        }
    }
}
