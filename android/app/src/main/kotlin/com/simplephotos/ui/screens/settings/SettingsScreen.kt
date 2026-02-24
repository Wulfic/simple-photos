package com.simplephotos.ui.screens.settings

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.sync.SyncScheduler
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val authRepo: AuthRepository,
    private val dataStore: DataStore<Preferences>
) : ViewModel() {
    var serverUrl by mutableStateOf("")
    var username by mutableStateOf("")
    var wifiOnly by mutableStateOf(true)
    var loading by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var message by mutableStateOf<String?>(null)

    companion object {
        val KEY_WIFI_ONLY = booleanPreferencesKey("wifi_only")
    }

    init {
        viewModelScope.launch {
            val prefs = dataStore.data.first()
            serverUrl = prefs[KEY_SERVER_URL] ?: ""
            username = prefs[KEY_USERNAME] ?: ""
            wifiOnly = prefs[KEY_WIFI_ONLY] ?: true
        }
    }

    fun setWifiOnlyPref(value: Boolean, context: android.content.Context) {
        wifiOnly = value
        viewModelScope.launch {
            dataStore.edit { it[KEY_WIFI_ONLY] = value }
            SyncScheduler.schedule(context, wifiOnly = value)
        }
    }

    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            loading = true
            try {
                authRepo.logout()
                onLoggedOut()
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }

    fun triggerBackup(context: android.content.Context) {
        SyncScheduler.triggerNow(context)
        message = "Backup triggered"
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    onLogout: () -> Unit,
    onSetup2fa: () -> Unit,
    onBackupFolders: () -> Unit,
    viewModel: SettingsViewModel = hiltViewModel()
) {
    val context = LocalContext.current

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
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
                .padding(16.dp)
                .fillMaxSize()
        ) {
            // Server info
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text("Account", style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(8.dp))
                    Text("Server: ${viewModel.serverUrl}", style = MaterialTheme.typography.bodyMedium)
                    Text("Username: ${viewModel.username}", style = MaterialTheme.typography.bodyMedium)
                }
            }

            Spacer(Modifier.height(16.dp))

            // Backup settings
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text("Backup", style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(8.dp))

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text("WiFi only")
                        Switch(
                            checked = viewModel.wifiOnly,
                            onCheckedChange = { viewModel.setWifiOnlyPref(it, context) }
                        )
                    }

                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(
                        onClick = onBackupFolders,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text("Backup Folders")
                    }

                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(
                        onClick = { viewModel.triggerBackup(context) },
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text("Backup Now")
                    }
                }
            }

            Spacer(Modifier.height(16.dp))

            // Security
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text("Security", style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(8.dp))

                    OutlinedButton(
                        onClick = onSetup2fa,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text("Two-Factor Authentication")
                    }
                }
            }

            Spacer(Modifier.weight(1f))

            // Messages
            viewModel.error?.let { err ->
                Text(err, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                Spacer(Modifier.height(8.dp))
            }
            viewModel.message?.let { msg ->
                Text(msg, color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.bodySmall)
                Spacer(Modifier.height(8.dp))
            }

            // Logout
            Button(
                onClick = { viewModel.logout(onLogout) },
                enabled = !viewModel.loading,
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.error
                )
            ) {
                if (viewModel.loading) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                } else {
                    Text("Log Out")
                }
            }
        }
    }
}
