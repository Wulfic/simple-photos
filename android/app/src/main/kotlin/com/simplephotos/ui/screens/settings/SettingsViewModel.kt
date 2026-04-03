package com.simplephotos.ui.screens.settings

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_BIOMETRIC_ENABLED
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_DIAGNOSTIC_LOGGING
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_THUMBNAIL_SIZE
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

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
        calculateFreeableSpace()
        load2faStatus()
        loadAudioBackupSetting()
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

                // When enabling audio backup, trigger a server-side scan so
                // existing audio files in the storage directory get registered
                // immediately instead of waiting for the next autoscan cycle.
                if (res.audioBackupEnabled) {
                    withContext(Dispatchers.IO) { api.scanAndRegister() }
                }
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

    fun calculateFreeableSpace() {
        viewModelScope.launch {
            try {
                val synced = withContext(Dispatchers.IO) {
                    db.photoDao().getByStatus(SyncStatus.SYNCED)
                }
                val withLocal = synced.filter { it.localPath != null }
                freeableCount = withLocal.size
                // Query actual on-device file sizes via ContentResolver
                // instead of relying on sizeBytes (which may be null for
                // locally-discovered photos or stale for server-synced ones).
                freeableBytes = withContext(Dispatchers.IO) {
                    withLocal.sumOf { photo ->
                        try {
                            val uri = android.net.Uri.parse(photo.localPath)
                            appContext.contentResolver.openFileDescriptor(uri, "r")?.use { pfd ->
                                pfd.statSize
                            } ?: photo.sizeBytes ?: 0L
                        } catch (_: Exception) {
                            photo.sizeBytes ?: 0L
                        }
                    }
                }
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
