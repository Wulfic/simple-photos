package com.simplephotos.ui.screens.trash

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.crypto.CryptoManager
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.TrashItemDto
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import javax.inject.Inject

private const val TAG = "TrashViewModel"

/** Manages the trash bin: fetching trashed items, restoring, permanent deletion, and empty-all. */
@HiltViewModel
class TrashViewModel @Inject constructor(
    private val api: ApiService,
    private val photoRepository: PhotoRepository,
    private val authRepository: AuthRepository,
    private val crypto: CryptoManager,
    @ApplicationContext private val appContext: Context,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    var items by mutableStateOf<List<TrashItemDto>>(emptyList())
        private set
    var isLoading by mutableStateOf(true)
        private set
    var error by mutableStateOf<String?>(null)
    var actionLoading by mutableStateOf<String?>(null)
        private set
    var selectedIds by mutableStateOf(emptySet<String>())
        private set
    var isSelectionMode by mutableStateOf(false)
        private set
    var serverBaseUrl by mutableStateOf("")
        private set
    var username by mutableStateOf("")
        private set
    /** Decrypted thumbnail file paths for encrypted trash items, keyed by trash item ID. */
    var decryptedThumbPaths by mutableStateOf<Map<String, String>>(emptyMap())
        private set

    private val trashThumbDir: File
        get() = File(appContext.filesDir, "trash_thumbs").also { it.mkdirs() }

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
            } catch (_: Exception) {}
            loadTrash()
        }
    }

    fun loadTrash() {
        viewModelScope.launch {
            isLoading = true
            error = null
            try {
                val resp = withContext(Dispatchers.IO) { api.listTrash(limit = 500) }
                items = resp.items
                android.util.Log.i(TAG, "loadTrash: fetched ${resp.items.size} items")

                // Decrypt thumbnails for encrypted trash items in the background
                val thumbMap = mutableMapOf<String, String>()
                for (item in resp.items) {
                    if (item.encryptedBlobId != null) {
                        // Encrypted item — need to download + decrypt thumbnail
                        try {
                            val cached = File(trashThumbDir, "${item.id}.jpg")
                            if (cached.exists()) {
                                thumbMap[item.id] = cached.absolutePath
                                continue
                            }
                            val encrypted = withContext(Dispatchers.IO) {
                                api.trashThumbnail(item.id).bytes()
                            }
                            val decrypted = crypto.decrypt(encrypted)
                            // Parse envelope to extract image bytes
                            val payload = JSONObject(String(decrypted, Charsets.UTF_8))
                            val dataB64 = payload.optString("data", "")
                            if (dataB64.isNotEmpty()) {
                                val thumbBytes = android.util.Base64.decode(dataB64, android.util.Base64.NO_WRAP)
                                cached.writeBytes(thumbBytes)
                                thumbMap[item.id] = cached.absolutePath
                                android.util.Log.d(TAG, "loadTrash: decrypted thumb for ${item.id} (${thumbBytes.size} bytes)")
                            }
                        } catch (e: Exception) {
                            android.util.Log.w(TAG, "loadTrash: failed to decrypt thumb for ${item.id}: ${e.message}")
                        }
                    }
                }
                decryptedThumbPaths = thumbMap
            } catch (e: Exception) {
                error = e.message ?: "Failed to load trash"
                android.util.Log.e(TAG, "loadTrash: failed: ${e.message}")
            } finally {
                isLoading = false
            }
        }
    }

    fun enterSelectionMode(id: String) {
        isSelectionMode = true
        selectedIds = setOf(id)
    }

    fun toggleSelect(id: String) {
        if (!isSelectionMode) return
        selectedIds = if (id in selectedIds) selectedIds - id else selectedIds + id
        if (selectedIds.isEmpty()) isSelectionMode = false
    }

    fun clearSelection() {
        selectedIds = emptySet()
        isSelectionMode = false
    }

    fun emptyTrash() {
        viewModelScope.launch {
            actionLoading = "empty"
            try {
                withContext(Dispatchers.IO) { api.emptyTrash() }
                items = emptyList()
                // Clean up all cached trash thumbnails
                withContext(Dispatchers.IO) {
                    trashThumbDir.listFiles()?.forEach { it.delete() }
                }
                decryptedThumbPaths = emptyMap()
                clearSelection()
                android.util.Log.i(TAG, "emptyTrash: completed")
            } catch (e: Exception) {
                error = e.message ?: "Failed to empty trash"
                android.util.Log.e(TAG, "emptyTrash: failed: ${e.message}")
            } finally {
                actionLoading = null
            }
        }
    }

    fun restoreSelected() {
        viewModelScope.launch {
            actionLoading = "bulk-restore"
            try {
                val restoring = selectedIds.toSet()
                withContext(Dispatchers.IO) {
                    coroutineScope {
                        restoring.map { id -> async { api.restoreFromTrash(id) } }.awaitAll()
                    }
                    // Clean up cached thumbnails for restored items
                    for (id in restoring) { File(trashThumbDir, "$id.jpg").delete() }
                }
                items = items.filter { it.id !in restoring }
                decryptedThumbPaths = decryptedThumbPaths.filterKeys { it !in restoring }
                clearSelection()
                android.util.Log.i(TAG, "restoreSelected: restored ${restoring.size} items")
            } catch (e: Exception) {
                error = e.message ?: "Failed to restore"
                android.util.Log.e(TAG, "restoreSelected: failed: ${e.message}")
                loadTrash()
            } finally {
                actionLoading = null
            }
        }
    }

    fun deleteSelected() {
        viewModelScope.launch {
            actionLoading = "bulk-delete"
            try {
                val deleting = selectedIds.toSet()
                withContext(Dispatchers.IO) {
                    coroutineScope {
                        deleting.map { id -> async { api.permanentDeleteTrash(id) } }.awaitAll()
                    }
                    // Clean up cached thumbnails for permanently deleted items
                    for (id in deleting) { File(trashThumbDir, "$id.jpg").delete() }
                }
                items = items.filter { it.id !in deleting }
                decryptedThumbPaths = decryptedThumbPaths.filterKeys { it !in deleting }
                clearSelection()
                android.util.Log.i(TAG, "deleteSelected: permanently deleted ${deleting.size} items")
            } catch (e: Exception) {
                error = e.message ?: "Failed to delete"
                android.util.Log.e(TAG, "deleteSelected: failed: ${e.message}")
                loadTrash()
            } finally {
                actionLoading = null
            }
        }
    }

    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { authRepository.logout() }
            } catch (_: Exception) {}
            onLoggedOut()
        }
    }
}
