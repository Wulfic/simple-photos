/**
 * Composable screen that displays and manages user-configurable application settings.
 */
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
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_THUMBNAIL_SIZE
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

/**
 * ViewModel powering the Settings screen: account management (password change,
 * 2FA status), encryption mode admin controls, storage stats, backup server
 * configuration, user management (admin), and app preferences (thumbnail size,
 * Wi-Fi-only backup, biometric lock, diagnostic logging).
 */
@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val authRepo: AuthRepository,
    private val api: ApiService,
    private val db: AppDatabase,
    val dataStore: DataStore<Preferences>,
    @ApplicationContext private val appContext: Context,
    private val keyManager: com.simplephotos.crypto.KeyManager
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

    // Thumbnail size ("normal" or "large")
    var thumbnailSize by mutableStateOf("normal")
        private set

    // 2FA status
    var totpEnabled by mutableStateOf(false)
        private set
    var totpLoading by mutableStateOf(true)
        private set

    // Encryption toggle
    var togglingEncryption by mutableStateOf(false)
        private set

    // Cleanup
    var cleanableCount by mutableStateOf(0)
    var cleanableBytes by mutableStateOf(0L)
    var cleaningUp by mutableStateOf(false)

    // Audio backup
    var audioBackupEnabled by mutableStateOf(false)
    var audioBackupLoading by mutableStateOf(true)
    var togglingAudioBackup by mutableStateOf(false)

    // Backup servers
    var backupServers by mutableStateOf<List<com.simplephotos.data.remote.dto.BackupServer>>(emptyList())
    var backupServersLoaded by mutableStateOf(false)
    var recovering by mutableStateOf(false)

    // SSL status
    var sslEnabled by mutableStateOf(false)
    var sslLoading by mutableStateOf(true)

    // Scan
    var scanning by mutableStateOf(false)
    var scanResult by mutableStateOf<String?>(null)

    // Re-convert encrypted media
    var reconverting by mutableStateOf(false)
    var reconvertResult by mutableStateOf<String?>(null)

    init {
        viewModelScope.launch {
            val prefs = dataStore.data.first()
            serverUrl = prefs[KEY_SERVER_URL] ?: ""
            username = prefs[KEY_USERNAME] ?: ""
            diagnosticLogging = prefs[KEY_DIAGNOSTIC_LOGGING] ?: false
            biometricEnabled = prefs[KEY_BIOMETRIC_ENABLED] ?: false
            thumbnailSize = prefs[KEY_THUMBNAIL_SIZE] ?: "normal"

            // Sync diagnostic logging toggle with server config (best-effort)
            syncDiagnosticsFromServer()
        }
        loadStorageStats()
        loadEncryptionSettings()
        calculateFreeableSpace()
        load2faStatus()
        loadAudioBackupSetting()
        loadCleanupStatus()
        loadBackupServers()
        loadSslStatus()
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

    fun toggleThumbnailSize() {
        val next = if (thumbnailSize == "normal") "large" else "normal"
        thumbnailSize = next
        viewModelScope.launch {
            dataStore.edit { it[KEY_THUMBNAIL_SIZE] = next }
        }
    }

    // ── 2FA Status ───────────────────────────────────────────────────────

    private fun load2faStatus() {
        viewModelScope.launch {
            totpLoading = true
            try {
                val status = withContext(Dispatchers.IO) { api.get2faStatus() }
                totpEnabled = status.totpEnabled
            } catch (_: Exception) {
                // Endpoint may not be available — leave default
            }
            totpLoading = false
        }
    }

    fun disable2fa(code: String, onSuccess: () -> Unit, onError: (String) -> Unit) {
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) {
                    api.disable2fa(com.simplephotos.data.remote.dto.TotpDisableRequest(code))
                }
                if (response.isSuccessful) {
                    totpEnabled = false
                    onSuccess()
                } else {
                    onError("Invalid code")
                }
            } catch (e: retrofit2.HttpException) {
                onError("Invalid code")
            } catch (e: Exception) {
                onError("Failed: ${e.message}")
            }
        }
    }

    // ── Encryption mode toggle ───────────────────────────────────────────

    fun toggleEncryptionMode() {
        viewModelScope.launch {
            togglingEncryption = true
            error = null
            try {
                val newMode = if (encryptionMode == "plain") "encrypted" else "plain"
                val res = withContext(Dispatchers.IO) {
                    api.setEncryptionMode(com.simplephotos.data.remote.dto.SetEncryptionModeRequest(newMode))
                }
                encryptionMode = newMode
                message = res.message
                loadEncryptionSettings()
            } catch (e: Exception) {
                error = "Failed to change encryption: ${e.message}"
            }
            togglingEncryption = false
        }
    }

    // ── Cleanup ──────────────────────────────────────────────────────────

    private fun loadCleanupStatus() {
        viewModelScope.launch {
            try {
                val res = withContext(Dispatchers.IO) { api.getCleanupStatus() }
                cleanableCount = res.cleanableCount
                cleanableBytes = res.cleanableBytes
            } catch (_: Exception) {}
        }
    }

    fun cleanupPlainFiles(onDone: () -> Unit) {
        viewModelScope.launch {
            cleaningUp = true
            try {
                val res = withContext(Dispatchers.IO) { api.cleanupPlainFiles() }
                message = res.message
                cleanableCount = 0
                cleanableBytes = 0
                loadStorageStats()
            } catch (e: Exception) {
                error = "Cleanup failed: ${e.message}"
            }
            cleaningUp = false
            onDone()
        }
    }

    // ── Backup servers ───────────────────────────────────────────────────

    private fun loadBackupServers() {
        viewModelScope.launch {
            try {
                val res = withContext(Dispatchers.IO) { api.listBackupServers() }
                backupServers = res.servers
            } catch (_: Exception) {}
            backupServersLoaded = true
        }
    }

    fun recoverFromBackup(serverId: String) {
        viewModelScope.launch {
            recovering = true
            error = null
            try {
                val res = withContext(Dispatchers.IO) { api.recoverFromBackup(serverId) }
                message = res.message
            } catch (e: Exception) {
                error = "Recovery failed: ${e.message}"
            }
            recovering = false
        }
    }

    // ── Audio backup ─────────────────────────────────────────────────────

    private fun loadAudioBackupSetting() {
        viewModelScope.launch {
            audioBackupLoading = true
            try {
                val res = withContext(Dispatchers.IO) { api.getAudioBackupSetting() }
                audioBackupEnabled = res.audioBackupEnabled
            } catch (_: Exception) {}
            audioBackupLoading = false
        }
    }

    fun toggleAudioBackup() {
        viewModelScope.launch {
            togglingAudioBackup = true
            try {
                val newVal = !audioBackupEnabled
                val res = withContext(Dispatchers.IO) {
                    api.setAudioBackupSetting(com.simplephotos.data.remote.dto.SetAudioBackupRequest(newVal))
                }
                audioBackupEnabled = res.audioBackupEnabled
                message = res.message
            } catch (e: Exception) {
                error = "Failed to update audio backup: ${e.message}"
            }
            togglingAudioBackup = false
        }
    }

    // ── SSL/TLS ──────────────────────────────────────────────────────────

    private fun loadSslStatus() {
        viewModelScope.launch {
            sslLoading = true
            try {
                val res = withContext(Dispatchers.IO) { api.getSslStatus() }
                sslEnabled = res.enabled
            } catch (_: Exception) {}
            sslLoading = false
        }
    }

    // ── Scan for new files ───────────────────────────────────────────────

    fun scanForNewFiles() {
        viewModelScope.launch {
            scanning = true
            scanResult = null
            error = null
            try {
                val res = withContext(Dispatchers.IO) { api.scanAndRegister() }
                scanResult = if (res.registered > 0)
                    "Found and registered ${res.registered} new file${if (res.registered > 1) "s" else ""}."
                else "No new files found."
            } catch (e: Exception) {
                error = "Scan failed: ${e.message}"
            }
            scanning = false
        }
    }

    // ── Re-convert encrypted media ───────────────────────────────────────

    fun triggerReconvert() {
        viewModelScope.launch {
            reconverting = true
            reconvertResult = null
            error = null
            try {
                val keyHex = withContext(Dispatchers.IO) { keyManager.getKeyHex() }
                if (keyHex == null) {
                    error = "Encryption key not available. Please log out and log back in."
                    reconverting = false
                    return@launch
                }
                val res = withContext(Dispatchers.IO) {
                    api.triggerReconvert(com.simplephotos.data.remote.dto.ReconvertRequest(keyHex))
                }
                reconvertResult = res.message
            } catch (e: Exception) {
                error = "Re-conversion failed: ${e.message}"
            }
            reconverting = false
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

                if (viewModel.totpLoading) {
                    CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                } else if (viewModel.totpEnabled) {
                    // 2FA is enabled — show status and disable option
                    var showDisableDialog by remember { mutableStateOf(false) }
                    var disableCode by remember { mutableStateOf("") }
                    var disableError by remember { mutableStateOf<String?>(null) }
                    var disabling by remember { mutableStateOf(false) }

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(Icons.Default.CheckCircle, contentDescription = null, modifier = Modifier.size(18.dp), tint = Color(0xFF22C55E))
                            Spacer(Modifier.width(6.dp))
                            Text("Enabled", color = Color(0xFF22C55E))
                        }
                        OutlinedButton(
                            onClick = { showDisableDialog = true },
                            colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error)
                        ) { Text("Disable") }
                    }

                    if (showDisableDialog) {
                        AlertDialog(
                            onDismissRequest = { if (!disabling) showDisableDialog = false },
                            title = { Text("Disable 2FA") },
                            text = {
                                Column {
                                    Text("Enter your current TOTP code to disable two-factor authentication.")
                                    Spacer(Modifier.height(12.dp))
                                    OutlinedTextField(
                                        value = disableCode,
                                        onValueChange = { disableCode = it.filter { c -> c.isDigit() }.take(6); disableError = null },
                                        label = { Text("6-digit code") },
                                        singleLine = true,
                                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                                        isError = disableError != null,
                                        supportingText = disableError?.let { e -> { Text(e) } },
                                        modifier = Modifier.fillMaxWidth(),
                                        enabled = !disabling
                                    )
                                }
                            },
                            confirmButton = {
                                Button(
                                    onClick = {
                                        disabling = true
                                        viewModel.disable2fa(disableCode,
                                            onSuccess = { disabling = false; showDisableDialog = false; disableCode = "" },
                                            onError = { disabling = false; disableError = it }
                                        )
                                    },
                                    enabled = disableCode.length == 6 && !disabling,
                                    colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error)
                                ) {
                                    if (disabling) { CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp); Spacer(Modifier.width(8.dp)) }
                                    Text("Disable 2FA")
                                }
                            },
                            dismissButton = { TextButton(onClick = { showDisableDialog = false; disableCode = "" }, enabled = !disabling) { Text("Cancel") } }
                        )
                    }
                } else {
                    // 2FA is disabled — show setup button
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text("Not enabled", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        OutlinedButton(onClick = onSetup2fa) { Text("Enable 2FA") }
                    }
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

            // ── Display ──────────────────────────────────────────────────
            SettingsCard(title = "Display", icon = Icons.Default.GridView) {
                Text(
                    "Choose how large thumbnails appear in the photo grid.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column {
                        Text("Thumbnail Size")
                        Text(
                            if (viewModel.thumbnailSize == "large") "Large — fewer, bigger thumbnails"
                            else "Normal — more thumbnails per row",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(
                            "Normal",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (viewModel.thumbnailSize == "normal") MaterialTheme.colorScheme.primary
                                    else MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Switch(
                            checked = viewModel.thumbnailSize == "large",
                            onCheckedChange = { viewModel.toggleThumbnailSize() }
                        )
                        Text(
                            "Large",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (viewModel.thumbnailSize == "large") MaterialTheme.colorScheme.primary
                                    else MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
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


            // ── Active Server ────────────────────────────────────────────
            SettingsCard(title = "Active Server", icon = Icons.Default.Dns) {
                SettingsRow("URL", viewModel.serverUrl)
                SettingsRow("Status", "Connected")
            }

            // ── Scan for New Files (admin) ───────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "Scan for New Files", icon = Icons.Default.Radar) {
                    Text(
                        "Scan the server storage directory for files that were placed there manually and register them in the database.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(
                        onClick = { viewModel.scanForNewFiles() },
                        enabled = !viewModel.scanning,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        if (viewModel.scanning) {
                            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.width(8.dp))
                        }
                        Icon(Icons.Default.Radar, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(8.dp))
                        Text("Scan Now")
                    }
                    viewModel.scanResult?.let { result ->
                        Spacer(Modifier.height(8.dp))
                        Text(result, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
                    }
                }
            }

            // ── Privacy & Encryption (admin) ────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "Privacy & Encryption", iconPainter = painterResource(R.drawable.ic_lock)) {
                    Text(
                        if (viewModel.encryptionMode == "encrypted")
                            "Photos are encrypted on the server. File contents cannot be read without your key."
                        else
                            "Photos are stored as plain files on the server. Consider enabling encryption for added privacy.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text("Encryption")
                        if (viewModel.togglingEncryption) {
                            CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                        } else {
                            Switch(
                                checked = viewModel.encryptionMode == "encrypted",
                                onCheckedChange = { viewModel.toggleEncryptionMode() }
                            )
                        }
                    }
                    Text(
                        "Mode: ${viewModel.encryptionMode.replaceFirstChar { it.uppercase() }}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            // ── Cleanup (admin, encrypted mode) ─────────────────────────
            if (viewModel.isAdmin && viewModel.encryptionMode == "encrypted") {
                SettingsCard(title = "Cleanup", icon = Icons.Default.CleaningServices) {
                    if (viewModel.cleanableCount > 0) {
                        Text(
                            "${viewModel.cleanableCount} leftover plain files (${formatBytes(viewModel.cleanableBytes)}) can be removed now that encryption is enabled.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(Modifier.height(8.dp))
                        OutlinedButton(
                            onClick = { viewModel.cleanupPlainFiles {} },
                            enabled = !viewModel.cleaningUp,
                            modifier = Modifier.fillMaxWidth(),
                            colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error)
                        ) {
                            if (viewModel.cleaningUp) {
                                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                                Spacer(Modifier.width(8.dp))
                            }
                            Text("Clean Up ${formatBytes(viewModel.cleanableBytes)}")
                        }
                    } else {
                        Text(
                            "No leftover plain files to clean up.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }

            // ── Re-convert Encrypted Media (admin, encrypted mode) ──────
            if (viewModel.isAdmin && viewModel.encryptionMode == "encrypted") {
                SettingsCard(title = "Re-convert Media", icon = Icons.Default.Sync) {
                    Text(
                        "Convert encrypted videos and images to web-compatible formats (MP4, JPEG). " +
                        "Required for video playback on mobile devices.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    if (viewModel.reconvertResult != null) {
                        Spacer(Modifier.height(8.dp))
                        Text(
                            viewModel.reconvertResult!!,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.primary
                        )
                    }
                    Spacer(Modifier.height(8.dp))
                    Button(
                        onClick = { viewModel.triggerReconvert() },
                        enabled = !viewModel.reconverting,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        if (viewModel.reconverting) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(16.dp),
                                strokeWidth = 2.dp,
                                color = MaterialTheme.colorScheme.onPrimary
                            )
                            Spacer(Modifier.width(8.dp))
                        }
                        Text(if (viewModel.reconverting) "Starting…" else "Re-convert Encrypted Media")
                    }
                }
            }

            // ── Backup Recovery (admin) ──────────────────────────────────
            if (viewModel.isAdmin && viewModel.backupServersLoaded && viewModel.backupServers.isNotEmpty()) {
                SettingsCard(title = "Backup Recovery", icon = Icons.Default.Restore) {
                    Text(
                        "Recover photos from a backup server instance.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    viewModel.backupServers.forEach { server ->
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(vertical = 4.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(server.name, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
                                Text(server.address, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                            OutlinedButton(
                                onClick = { viewModel.recoverFromBackup(server.id) },
                                enabled = !viewModel.recovering
                            ) {
                                if (viewModel.recovering) {
                                    CircularProgressIndicator(modifier = Modifier.size(14.dp), strokeWidth = 2.dp)
                                    Spacer(Modifier.width(4.dp))
                                }
                                Text("Recover")
                            }
                        }
                    }
                }
            }

            // ── Audio Backup (admin) ─────────────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "Audio Backup", icon = Icons.Default.MusicNote) {
                    Text(
                        "When enabled, audio files on the device will also be backed up to the server.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text("Audio Backup")
                        if (viewModel.audioBackupLoading || viewModel.togglingAudioBackup) {
                            CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                        } else {
                            Switch(
                                checked = viewModel.audioBackupEnabled,
                                onCheckedChange = { viewModel.toggleAudioBackup() }
                            )
                        }
                    }
                }
            }

            // ── SSL/TLS (admin) ──────────────────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "SSL / TLS", iconPainter = painterResource(R.drawable.ic_lock)) {
                    if (viewModel.sslLoading) {
                        CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                    } else {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(
                                if (viewModel.sslEnabled) Icons.Default.CheckCircle else Icons.Default.Warning,
                                contentDescription = null,
                                modifier = Modifier.size(18.dp),
                                tint = if (viewModel.sslEnabled) Color(0xFF22C55E) else Color(0xFFF59E0B)
                            )
                            Spacer(Modifier.width(8.dp))
                            Text(
                                if (viewModel.sslEnabled) "SSL/TLS is enabled"
                                else "SSL/TLS is not enabled — connections are unencrypted",
                                style = MaterialTheme.typography.bodySmall,
                                color = if (viewModel.sslEnabled) Color(0xFF22C55E) else Color(0xFFF59E0B)
                            )
                        }
                    }
                }
            }

            // ── Manage Users (admin) ─────────────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "Manage Users", icon = Icons.Default.Group) {
                    Text(
                        "User management is available in the web interface.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Open ${viewModel.serverUrl} in a browser to manage users.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.primary
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
