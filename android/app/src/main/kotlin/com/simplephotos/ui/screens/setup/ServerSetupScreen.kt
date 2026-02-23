package com.simplephotos.ui.screens.setup

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.remote.ApiService
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_CONFIGURED
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class ServerSetupViewModel @Inject constructor(
    private val dataStore: DataStore<Preferences>
) : ViewModel() {
    var serverUrl by mutableStateOf("http://")
    var loading by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)

    fun testAndSave(onSuccess: () -> Unit) {
        viewModelScope.launch {
            loading = true
            error = null
            try {
                val url = serverUrl.trimEnd('/')
                // Simple connectivity test
                val response = java.net.URL("$url/health").openConnection() as java.net.HttpURLConnection
                response.connectTimeout = 5000
                response.readTimeout = 5000
                if (response.responseCode != 200) {
                    error = "Server returned ${response.responseCode}"
                    return@launch
                }

                dataStore.edit { prefs ->
                    prefs[KEY_SERVER_URL] = url
                    prefs[KEY_SERVER_CONFIGURED] = true
                }
                onSuccess()
            } catch (e: Exception) {
                error = "Cannot connect: ${e.message}"
            } finally {
                loading = false
            }
        }
    }
}

@Composable
fun ServerSetupScreen(
    onSetupComplete: () -> Unit,
    viewModel: ServerSetupViewModel = hiltViewModel()
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text("Server Setup", style = MaterialTheme.typography.headlineMedium)
        Spacer(Modifier.height(8.dp))
        Text(
            "Enter your Simple Photos server URL",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Spacer(Modifier.height(24.dp))

        OutlinedTextField(
            value = viewModel.serverUrl,
            onValueChange = { viewModel.serverUrl = it },
            label = { Text("Server URL") },
            placeholder = { Text("http://192.168.1.100:3000") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )

        viewModel.error?.let { err ->
            Spacer(Modifier.height(8.dp))
            Text(err, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }

        Spacer(Modifier.height(16.dp))
        Button(
            onClick = { viewModel.testAndSave(onSetupComplete) },
            enabled = !viewModel.loading && viewModel.serverUrl.isNotBlank(),
            modifier = Modifier.fillMaxWidth()
        ) {
            if (viewModel.loading) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
            } else {
                Text("Connect")
            }
        }
    }
}
