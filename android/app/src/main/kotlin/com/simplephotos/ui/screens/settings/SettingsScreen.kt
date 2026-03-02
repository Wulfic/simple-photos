package com.simplephotos.ui.screens.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_BIOMETRIC_ENABLED
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_DIAGNOSTIC_LOGGING
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject
import com.simplephotos.R
import androidx.compose.ui.res.painterResource
import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import dagger.hilt.android.qualifiers.ApplicationContext

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val authRepo: AuthRepository,
    private val api: ApiService,
    private val db: AppDatabase,
    val dataStore: DataStore<Preferences>,
    @ApplicationContext private val appContext: Context
) : ViewModel() {
    var serverUrl by mutableStateOf("")
    var username by mutableStateOf("")
    var loading by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var message by mutableStateOf<String?>(null)

    // Storage stats
    var storageStats by mutableStateOf<StorageStatsResponse?>(null)
    var storageLoading by mutableStateOf(false)

    // Encryption mode
    var encryptionMode by mutableStateOf("plain")

    // Admin status
    var isAdmin by mutableStateOf(false)

    // Diagnostic logging toggle
    var diagnosticLogging by mutableStateOf(false)
        private set

    // Biometric lock
    var biometricEnabled by mutableStateOf(false)
        private set

    // Free up space
    var freeableBytes by mutableStateOf(0L)
    var freeableCount by mutableStateOf(0)
    var freeUpLoading by mutableStateOf(false)



    init {
        viewModelScope.launch {
            val prefs = dataStore.data.first()
            serverUrl = prefs[KEY_SERVER_URL] ?: ""
            username = prefs[KEY_USERNAME] ?: ""
            diagnosticLogging = prefs[KEY_DIAGNOSTIC_LOGGING] ?: false
            biometricEnabled = prefs[KEY_BIOMETRIC_ENABLED] ?: false

            // Sync diagnostic logging toggle with server config (best-effort)
            syncDiagnosticsFromServer()
        }
        loadStorageStats()
        loadEncryptionSettings()
        calculateFreeableSpace()
    }

    fun loadStorageStats() {
        viewModelScope.launch {
            storageLoading = true
            try {
                storageStats = withContext(Dispatchers.IO) { api.getStorageStats() }
            } catch (_: Exception) {}
            storageLoading = false
        }
    }

    private fun loadEncryptionSettings() {
        viewModelScope.launch {
            try {
                val settings = withContext(Dispatchers.IO) { api.getEncryptionSettings() }
                encryptionMode = settings.encryptionMode
            } catch (_: Exception) {}
        }
    }

    fun checkAdmin() {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { api.listUsers() }
                isAdmin = true
            } catch (_: Exception) {
                isAdmin = false
            }
        }
    }

    fun changePassword(currentPassword: String, newPassword: String, onSuccess: () -> Unit) {
        viewModelScope.launch {
            loading = true
            error = null
            try {
                val response = withContext(Dispatchers.IO) {
                    api.changePassword(ChangePasswordRequest(currentPassword, newPassword))
                }
                if (response.isSuccessful) {
                    message = "Password changed successfully"
                    onSuccess()
                } else {
                    error = "Failed to change password (${response.code()})"
                }
            } catch (e: Exception) {
                error = "Failed to change password: ${e.message}"
            }
            loading = false
        }
    }



    fun toggleDiagnosticLogging() {
        viewModelScope.launch {
            diagnosticLogging = !diagnosticLogging
            dataStore.edit { it[KEY_DIAGNOSTIC_LOGGING] = diagnosticLogging }

            // Best-effort: notify server of the client diagnostics state change
            try {
                withContext(Dispatchers.IO) {
                    api.updateDiagnosticsConfig(
                        UpdateDiagnosticsConfigRequest(
                            clientDiagnosticsEnabled = diagnosticLogging
                        )
                    )
                }
            } catch (_: Exception) {
                // Non-critical — local toggle still works independently
            }
        }
    }

    /**
     * Sync local diagnostic logging toggle with the server's
     * `client_diagnostics_enabled` setting. Best-effort — if the server
     * is unreachable or the user isn't admin, the local toggle is unaffected.
     */
    private fun syncDiagnosticsFromServer() {
        viewModelScope.launch {
            try {
                val config = withContext(Dispatchers.IO) { api.getDiagnosticsConfig() }
                diagnosticLogging = config.clientDiagnosticsEnabled
                dataStore.edit { it[KEY_DIAGNOSTIC_LOGGING] = diagnosticLogging }
            } catch (_: Exception) {
                // Best-effort — keep local value if server call fails
            }
        }
    }

    fun toggleBiometric() {
        viewModelScope.launch {
            biometricEnabled = !biometricEnabled
            dataStore.edit { it[KEY_BIOMETRIC_ENABLED] = biometricEnabled }
        }
    }

    /**
     * Verify password with the server, then enable biometric lock.
     * Also stores the password in EncryptedSharedPreferences so that
     * Secure Albums can auto-unlock via biometric without re-entering it.
     */
    fun enableBiometricWithPassword(password: String, onSuccess: () -> Unit, onError: (String) -> Unit) {
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) {
                    api.verifyPassword(VerifyPasswordRequest(password))
                }
                if (response.isSuccessful) {
                    biometricEnabled = true
                    dataStore.edit { it[KEY_BIOMETRIC_ENABLED] = true }
                    // Persist password for Secure Gallery biometric auto-unlock
                    try {
                        val masterKey = MasterKey.Builder(appContext)
                            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                            .build()
                        val encPrefs = EncryptedSharedPreferences.create(
                            appContext,
                            "secure_gallery_prefs",
                            masterKey,
                            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
                        )
                        encPrefs.edit().putString("gallery_password", password).apply()
                    } catch (_: Exception) {
                        // Non-fatal: biometric is enabled but gallery auto-unlock may not work
                    }
                    onSuccess()
                } else {
                    onError("Incorrect password")
                }
            } catch (e: retrofit2.HttpException) {
                onError("Incorrect password")
            } catch (e: Exception) {
                onError("Verification failed: ${e.message}")
            }
        }
    }

    fun disableBiometric() {
        viewModelScope.launch {
            biometricEnabled = false
            dataStore.edit { it[KEY_BIOMETRIC_ENABLED] = false }
            // Clear stored Secure Gallery password when biometric is disabled
            try {
                val masterKey = MasterKey.Builder(appContext)
                    .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                    .build()
                val encPrefs = EncryptedSharedPreferences.create(
                    appContext,
                    "secure_gallery_prefs",
                    masterKey,
                    EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                    EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
                )
                encPrefs.edit().remove("gallery_password").apply()
            } catch (_: Exception) {
                // Non-fatal
            }
        }
    }

    fun calculateFreeableSpace() {
        viewModelScope.launch {
            try {
                val synced = withContext(Dispatchers.IO) {
                    db.photoDao().getByStatus(SyncStatus.SYNCED)
                }
                val withLocal = synced.filter { it.localPath != null }
                freeableCount = withLocal.size
                freeableBytes = withLocal.sumOf { it.sizeBytes ?: 0L }
            } catch (_: Exception) {
                freeableCount = 0
                freeableBytes = 0L
            }
        }
    }

    fun freeUpSpace(context: android.content.Context, onComplete: (Int) -> Unit) {
        viewModelScope.launch {
            freeUpLoading = true
            var deleted = 0
            try {
                val synced = withContext(Dispatchers.IO) {
                    db.photoDao().getByStatus(SyncStatus.SYNCED)
                }
                val withLocal = synced.filter { it.localPath != null }

                for (photo in withLocal) {
                    try {
                        val uri = android.net.Uri.parse(photo.localPath)
                        val rows = withContext(Dispatchers.IO) {
                            context.contentResolver.delete(uri, null, null)
                        }
                        if (rows > 0) {
                            // Clear localPath so we don't try again
                            withContext(Dispatchers.IO) {
                                db.photoDao().update(photo.copy(localPath = null))
                            }
                            deleted++
                        }
                    } catch (_: Exception) {
                        // Some URIs may not be deletable (permission denied)
                    }
                }
                message = "Freed up space from $deleted photos"
                calculateFreeableSpace()
            } catch (e: Exception) {
                error = "Failed to free up space: ${e.message}"
            }
            freeUpLoading = false
            onComplete(deleted)
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
    val scrollState = rememberScrollState()

    // Check admin status on first compose
    LaunchedEffect(Unit) {
        viewModel.checkAdmin()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(painter = painterResource(R.drawable.ic_back_arrow), contentDescription = "Back", modifier = Modifier.size(24.dp))
                    }
                }
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
                .verticalScroll(scrollState)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            // ── Account ──────────────────────────────────────────────────
            SettingsCard(title = "Account", icon = Icons.Default.Person) {
                SettingsRow("Server", viewModel.serverUrl)
                SettingsRow("Username", viewModel.username)
                SettingsRow("Mode", viewModel.encryptionMode.replaceFirstChar { it.uppercase() })

                // ── Change Password (inline) ─────────────────────────────
                HorizontalDivider(modifier = Modifier.padding(vertical = 12.dp))
                AccountChangePasswordSection(viewModel)

                // ── Two-Factor Authentication (inline) ───────────────────
                HorizontalDivider(modifier = Modifier.padding(vertical = 12.dp))
                Text("Two-Factor Authentication", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
                Spacer(Modifier.height(8.dp))
                OutlinedButton(onClick = onSetup2fa, modifier = Modifier.fillMaxWidth()) {
                    Text("Two-Factor Authentication")
                }
            }

            // ── Biometric Lock ───────────────────────────────────────────
            SettingsCard(title = "Security", iconPainter = painterResource(R.drawable.ic_lock)) {
                Text(
                    "Require fingerprint or face unlock to open the app.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(8.dp))

                // Password verification dialog state
                var showBiometricPasswordDialog by remember { mutableStateOf(false) }
                var biometricPassword by remember { mutableStateOf("") }
                var biometricPasswordError by remember { mutableStateOf<String?>(null) }
                var biometricPasswordVisible by remember { mutableStateOf(false) }
                var biometricVerifying by remember { mutableStateOf(false) }

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Biometric Lock")
                    Switch(
                        checked = viewModel.biometricEnabled,
                        onCheckedChange = { enabling ->
                            if (enabling) {
                                // Prompt for password before enabling
                                biometricPassword = ""
                                biometricPasswordError = null
                                showBiometricPasswordDialog = true
                            } else {
                                // Disabling doesn't need verification
                                viewModel.disableBiometric()
                            }
                        }
                    )
                }

                if (showBiometricPasswordDialog) {
                    AlertDialog(
                        onDismissRequest = {
                            if (!biometricVerifying) {
                                showBiometricPasswordDialog = false
                            }
                        },
                        icon = { Icon(painter = painterResource(R.drawable.ic_lock), contentDescription = null, modifier = Modifier.size(24.dp), tint = MaterialTheme.colorScheme.primary) },
                        title = { Text("Enable Biometric Lock") },
                        text = {
                            Column {
                                Text(
                                    "Enter your password to enable biometric lock.",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                                Spacer(Modifier.height(12.dp))
                                OutlinedTextField(
                                    value = biometricPassword,
                                    onValueChange = {
                                        biometricPassword = it
                                        biometricPasswordError = null
                                    },
                                    label = { Text("Password") },
                                    singleLine = true,
                                    visualTransformation = if (biometricPasswordVisible) VisualTransformation.None else PasswordVisualTransformation(),
                                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                                    trailingIcon = {
                                        IconButton(onClick = { biometricPasswordVisible = !biometricPasswordVisible }) {
                                            Icon(
                                                imageVector = if (biometricPasswordVisible) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                                                contentDescription = if (biometricPasswordVisible) "Hide password" else "Show password"
                                            )
                                        }
                                    },
                                    isError = biometricPasswordError != null,
                                    supportingText = biometricPasswordError?.let { err -> { Text(err) } },
                                    modifier = Modifier.fillMaxWidth(),
                                    enabled = !biometricVerifying
                                )
                            }
                        },
                        confirmButton = {
                            Button(
                                onClick = {
                                    biometricVerifying = true
                                    viewModel.enableBiometricWithPassword(
                                        password = biometricPassword,
                                        onSuccess = {
                                            biometricVerifying = false
                                            showBiometricPasswordDialog = false
                                            biometricPassword = ""
                                        },
                                        onError = { errorMsg ->
                                            biometricVerifying = false
                                            biometricPasswordError = errorMsg
                                        }
                                    )
                                },
                                enabled = biometricPassword.isNotEmpty() && !biometricVerifying
                            ) {
                                if (biometricVerifying) {
                                    CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                                    Spacer(Modifier.width(8.dp))
                                }
                                Text("Enable")
                            }
                        },
                        dismissButton = {
                            TextButton(
                                onClick = {
                                    showBiometricPasswordDialog = false
                                    biometricPassword = ""
                                },
                                enabled = !biometricVerifying
                            ) {
                                Text("Cancel")
                            }
                        }
                    )
                }
            }

            // ── Storage Stats ────────────────────────────────────────────
            SettingsCard(title = "Storage", iconPainter = painterResource(R.drawable.ic_floppy_disk)) {
                val stats = viewModel.storageStats
                if (viewModel.storageLoading) {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
                } else if (stats != null) {
                    StorageBar(stats)
                    Spacer(Modifier.height(8.dp))
                    SettingsRow("Photos", "${stats.photoCount + stats.plainCount} files (${formatBytes(stats.photoBytes + stats.plainBytes)})")
                    SettingsRow("Videos", "${stats.videoCount} files (${formatBytes(stats.videoBytes)})")
                    SettingsRow("Total Used", formatBytes(stats.userTotalBytes))
                    SettingsRow("Disk Free", formatBytes(stats.fsFreeBytes))
                    SettingsRow("Disk Total", formatBytes(stats.fsTotalBytes))
                } else {
                    Text("Unable to load stats", style = MaterialTheme.typography.bodySmall)
                }
                Spacer(Modifier.height(4.dp))
                TextButton(onClick = { viewModel.loadStorageStats() }) {
                    Text("Refresh")
                }
            }

            // ── Backup Folders ───────────────────────────────────────────
            SettingsCard(title = "Backup Folders", iconPainter = painterResource(R.drawable.ic_image)) {
                Text(
                    "Choose which folders on your device to back up. Camera is backed up by default.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(8.dp))
                OutlinedButton(
                    onClick = onBackupFolders,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Icon(painter = painterResource(R.drawable.ic_folder), contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text("Select Backup Folders")
                }
            }

            // ── Free Up Space ────────────────────────────────────────────
            val context = LocalContext.current
            var showFreeUpDialog by remember { mutableStateOf(false) }

            SettingsCard(title = "Free Up Space", icon = Icons.Default.CleaningServices) {
                if (viewModel.freeableCount > 0) {
                    Text(
                        "${viewModel.freeableCount} photos already backed up to the server can be removed from this device, freeing up ${formatBytes(viewModel.freeableBytes)}.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(
                        onClick = { showFreeUpDialog = true },
                        enabled = !viewModel.freeUpLoading,
                        modifier = Modifier.fillMaxWidth(),
                        colors = ButtonDefaults.outlinedButtonColors(
                            contentColor = MaterialTheme.colorScheme.error
                        )
                    ) {
                        if (viewModel.freeUpLoading) {
                            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.width(8.dp))
                        }
                        Icon(Icons.Default.DeleteForever, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(8.dp))
                        Text("Free Up ${formatBytes(viewModel.freeableBytes)}")
                    }
                } else {
                    Text(
                        "No backed-up photos to remove from this device.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            if (showFreeUpDialog) {
                AlertDialog(
                    onDismissRequest = { showFreeUpDialog = false },
                    icon = { Icon(Icons.Default.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
                    title = { Text("Free Up Space?") },
                    text = {
                        Text(
                            "This will permanently delete ${viewModel.freeableCount} photos and videos from your device that have already been backed up to the server.\n\nYou'll free up approximately ${formatBytes(viewModel.freeableBytes)}.\n\nThis cannot be undone. The files will still be available on your server."
                        )
                    },
                    confirmButton = {
                        Button(
                            onClick = {
                                showFreeUpDialog = false
                                viewModel.freeUpSpace(context) { _ -> }
                            },
                            colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error)
                        ) {
                            Text("Delete from Device")
                        }
                    },
                    dismissButton = {
                        TextButton(onClick = { showFreeUpDialog = false }) {
                            Text("Cancel")
                        }
                    }
                )
            }


            // ── Diagnostic Logging ───────────────────────────────────────
            SettingsCard(title = "Troubleshooting", icon = Icons.Default.BugReport) {
                Text(
                    "When enabled, the app sends detailed backup logs to the server to help diagnose upload issues.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Diagnostic Logging")
                    Switch(
                        checked = viewModel.diagnosticLogging,
                        onCheckedChange = { viewModel.toggleDiagnosticLogging() }
                    )
                }
            }




            // ── About ────────────────────────────────────────────────────
            SettingsCard(title = "About", icon = Icons.Default.Info) {
                Text("Simple Photos", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                Text("Version 0.6.9", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(4.dp))
                Text(
                    "A self-hosted photo storage solution with optional end-to-end encryption.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            // ── Credits & Links ──────────────────────────────────────────
            SettingsCard(title = "Credits & Links", iconPainter = painterResource(R.drawable.ic_star)) {
                val uriHandler = androidx.compose.ui.platform.LocalUriHandler.current
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(painter = painterResource(R.drawable.ic_star), contentDescription = null, modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(8.dp))
                    Column {
                        Text("Icons", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
                        Text(
                            "Custom icons by Angus_87 on Flaticon",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.clickable { uriHandler.openUri("https://www.flaticon.com/authors/angus-87") }
                        )
                    }
                }
                Spacer(Modifier.height(12.dp))
                HorizontalDivider()
                Spacer(Modifier.height(12.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(painter = painterResource(R.drawable.ic_shared), contentDescription = null, modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(8.dp))
                    Column {
                        Text("Source Code", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
                        Text(
                            "github.com/wulfic/simple-photos",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.clickable { uriHandler.openUri("https://github.com/wulfic/simple-photos") }
                        )
                    }
                }
            }

            // ── Messages ─────────────────────────────────────────────────
            viewModel.error?.let { err ->
                Text(err, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
            viewModel.message?.let { msg ->
                Text(msg, color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.bodySmall)
            }

            // ── Logout ───────────────────────────────────────────────────
            Button(
                onClick = { viewModel.logout(onLogout) },
                enabled = !viewModel.loading,
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error)
            ) {
                if (viewModel.loading) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onError)
                } else {
                    Text("Log Out")
                }
            }

            Spacer(Modifier.height(32.dp))
        }
    }
}

// ── Reusable settings card ──────────────────────────────────────────────────

@Composable
private fun SettingsCard(
    title: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    content: @Composable ColumnScope.() -> Unit
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(icon, contentDescription = null, modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(8.dp))
                Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            }
            Spacer(Modifier.height(12.dp))
            content()
        }
    }
}

@Composable
private fun SettingsCard(
    title: String,
    iconPainter: androidx.compose.ui.graphics.painter.Painter,
    content: @Composable ColumnScope.() -> Unit
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(painter = iconPainter, contentDescription = null, modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(8.dp))
                Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            }
            Spacer(Modifier.height(12.dp))
            content()
        }
    }
}

@Composable
private fun SettingsRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodyMedium)
    }
}

// ── Storage Bar ─────────────────────────────────────────────────────────────

@Composable
private fun StorageBar(stats: StorageStatsResponse) {
    val total = stats.fsTotalBytes.toFloat().coerceAtLeast(1f)
    val photoFraction = (stats.photoBytes + stats.plainBytes) / total
    val videoFraction = stats.videoBytes / total
    val otherFraction = stats.otherBlobBytes / total
    val freeFraction = stats.fsFreeBytes / total

    Column {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(20.dp)
                .clip(RoundedCornerShape(4.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant)
        ) {
            Row(modifier = Modifier.fillMaxSize()) {
                if (photoFraction > 0.001f) {
                    Box(modifier = Modifier.fillMaxHeight().weight(photoFraction).background(Color(0xFF3B82F6)))
                }
                if (videoFraction > 0.001f) {
                    Box(modifier = Modifier.fillMaxHeight().weight(videoFraction).background(Color(0xFF8B5CF6)))
                }
                if (otherFraction > 0.001f) {
                    Box(modifier = Modifier.fillMaxHeight().weight(otherFraction).background(Color(0xFFF59E0B)))
                }
                if (freeFraction > 0.001f) {
                    Box(modifier = Modifier.fillMaxHeight().weight(freeFraction))
                }
            }
        }
        Spacer(Modifier.height(4.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            LegendDot(Color(0xFF3B82F6), "Photos")
            LegendDot(Color(0xFF8B5CF6), "Videos")
            LegendDot(Color(0xFFF59E0B), "Other")
        }
    }
}

@Composable
private fun LegendDot(color: Color, label: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(modifier = Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(color))
        Spacer(Modifier.width(4.dp))
        Text(label, style = MaterialTheme.typography.labelSmall)
    }
}

// ── Change Password (embedded in Account card) ─────────────────────────────

@Composable
private fun AccountChangePasswordSection(viewModel: SettingsViewModel) {
    var expanded by remember { mutableStateOf(false) }
    var currentPassword by remember { mutableStateOf("") }
    var newPassword by remember { mutableStateOf("") }
    var confirmPassword by remember { mutableStateOf("") }
    var showPasswords by remember { mutableStateOf(false) }

    Text("Password", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
    Spacer(Modifier.height(8.dp))

    if (!expanded) {
        OutlinedButton(onClick = { expanded = true }, modifier = Modifier.fillMaxWidth()) {
            Text("Change Password")
        }
    } else {
        OutlinedTextField(
            value = currentPassword,
            onValueChange = { currentPassword = it },
            label = { Text("Current Password") },
            visualTransformation = if (showPasswords) VisualTransformation.None else PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(
            value = newPassword,
            onValueChange = { newPassword = it },
            label = { Text("New Password") },
            visualTransformation = if (showPasswords) VisualTransformation.None else PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(
            value = confirmPassword,
            onValueChange = { confirmPassword = it },
            label = { Text("Confirm New Password") },
            visualTransformation = if (showPasswords) VisualTransformation.None else PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )

        // Password strength indicator
        if (newPassword.isNotEmpty()) {
            Spacer(Modifier.height(4.dp))
            val strength = passwordStrength(newPassword)
            LinearProgressIndicator(
                progress = { strength.first },
                modifier = Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)),
                color = strength.second
            )
            Text(strength.third, style = MaterialTheme.typography.labelSmall, color = strength.second)
        }

        Spacer(Modifier.height(8.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Checkbox(checked = showPasswords, onCheckedChange = { showPasswords = it })
            Text("Show passwords", style = MaterialTheme.typography.bodySmall)
        }

        Spacer(Modifier.height(8.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = {
                expanded = false
                currentPassword = ""
                newPassword = ""
                confirmPassword = ""
            }) { Text("Cancel") }
            Button(
                onClick = {
                    if (newPassword != confirmPassword) {
                        viewModel.error = "Passwords do not match"
                    } else {
                        viewModel.changePassword(currentPassword, newPassword) {
                            expanded = false
                            currentPassword = ""
                            newPassword = ""
                            confirmPassword = ""
                        }
                    }
                },
                enabled = currentPassword.isNotEmpty() && newPassword.isNotEmpty() && confirmPassword.isNotEmpty() && !viewModel.loading
            ) { Text("Update") }
        }
    }
}

private fun passwordStrength(password: String): Triple<Float, Color, String> {
    var score = 0
    if (password.length >= 8) score++
    if (password.length >= 12) score++
    if (password.any { it.isUpperCase() }) score++
    if (password.any { it.isLowerCase() }) score++
    if (password.any { it.isDigit() }) score++
    if (password.any { !it.isLetterOrDigit() }) score++
    return when {
        score <= 2 -> Triple(0.2f, Color(0xFFEF4444), "Weak")
        score <= 3 -> Triple(0.4f, Color(0xFFF59E0B), "Fair")
        score <= 4 -> Triple(0.7f, Color(0xFF3B82F6), "Good")
        else -> Triple(1f, Color(0xFF22C55E), "Strong")
    }
}

// ── Utilities ────────────────────────────────────────────────────────────────

private fun formatBytes(bytes: Long): String {
    if (bytes <= 0) return "0 B"
    val units = arrayOf("B", "KB", "MB", "GB", "TB")
    val digitGroups = (Math.log10(bytes.toDouble()) / Math.log10(1024.0)).toInt().coerceAtMost(units.lastIndex)
    return "%.1f %s".format(bytes / Math.pow(1024.0, digitGroups.toDouble()), units[digitGroups])
}
