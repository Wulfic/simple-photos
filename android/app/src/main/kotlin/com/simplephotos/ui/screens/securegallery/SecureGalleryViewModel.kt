package com.simplephotos.ui.screens.securegallery

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryAddItemRequest
import com.simplephotos.data.remote.dto.SecureGalleryCreateRequest
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.data.remote.dto.SecureGalleryUnlockRequest
import com.simplephotos.data.repository.PhotoRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

// ─────────────────────────────────────────────────────────────────────────────
// ViewModel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Manages password-protected secure galleries: unlock with account password,
 * browse galleries, view encrypted items, and add photos.
 */
@HiltViewModel
class SecureGalleryViewModel @Inject constructor(
    private val api: ApiService,
    private val photoRepository: PhotoRepository,
    private val db: AppDatabase
) : ViewModel() {

    // Auth gate
    var isAuthenticated by mutableStateOf(false)
        private set
    var galleryToken by mutableStateOf("")
        private set
    var authError by mutableStateOf<String?>(null)
        private set
    var authLoading by mutableStateOf(false)
        private set

    // Gallery list
    var galleries by mutableStateOf<List<SecureGallery>>(emptyList())
        private set
    var galleriesLoading by mutableStateOf(false)
        private set

    // Selected gallery detail
    var selectedGallery by mutableStateOf<SecureGallery?>(null)
        private set
    var items by mutableStateOf<List<SecureGalleryItem>>(emptyList())
        private set
    var itemsLoading by mutableStateOf(false)
        private set

    // Photos for picker
    var allPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
        private set

    var error by mutableStateOf<String?>(null)
        private set

    fun unlock(password: String) {
        viewModelScope.launch {
            authLoading = true
            authError = null
            try {
                val res = withContext(Dispatchers.IO) {
                    api.unlockSecureGalleries(SecureGalleryUnlockRequest(password))
                }
                galleryToken = res.galleryToken
                isAuthenticated = true
                loadGalleries()
            } catch (e: Exception) {
                authError = e.message ?: "Invalid password"
            } finally {
                authLoading = false
            }
        }
    }

    fun loadGalleries() {
        viewModelScope.launch {
            galleriesLoading = true
            try {
                val res = withContext(Dispatchers.IO) { api.listSecureGalleries() }
                galleries = res.galleries
            } catch (e: Exception) {
                error = "Failed to load galleries: ${e.message}"
            } finally {
                galleriesLoading = false
            }
        }
    }

    fun selectGallery(gallery: SecureGallery) {
        selectedGallery = gallery
        loadItems(gallery.id)
        loadPhotos()
    }

    fun deselectGallery() {
        selectedGallery = null
        items = emptyList()
    }

    private fun loadItems(galleryId: String) {
        viewModelScope.launch {
            itemsLoading = true
            try {
                val res = withContext(Dispatchers.IO) {
                    api.listSecureGalleryItems(galleryId, galleryToken)
                }
                items = res.items
            } catch (e: Exception) {
                error = "Failed to load items: ${e.message}"
            } finally {
                itemsLoading = false
            }
        }
    }

    private fun loadPhotos() {
        viewModelScope.launch {
            try {
                allPhotos = withContext(Dispatchers.IO) {
                    photoRepository.getAllPhotos().first()
                }
            } catch (_: Exception) {}
        }
    }

    fun createGallery(name: String) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    api.createSecureGallery(SecureGalleryCreateRequest(name))
                }
                loadGalleries()
            } catch (e: Exception) {
                error = "Create failed: ${e.message}"
            }
        }
    }

    fun deleteGallery(gallery: SecureGallery) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { api.deleteSecureGallery(gallery.id) }
                if (selectedGallery?.id == gallery.id) {
                    selectedGallery = null
                    items = emptyList()
                }
                loadGalleries()
            } catch (e: Exception) {
                error = "Delete failed: ${e.message}"
            }
        }
    }

    fun addPhotosToGallery(blobIds: List<String>) {
        val gallery = selectedGallery ?: return
        viewModelScope.launch {
            try {
                for (blobId in blobIds) {
                    withContext(Dispatchers.IO) {
                        api.addSecureGalleryItem(gallery.id, SecureGalleryAddItemRequest(blobId))
                    }
                }
                loadItems(gallery.id)
                loadGalleries()
            } catch (e: Exception) {
                error = "Add photos failed: ${e.message}"
            }
        }
    }

    /**
     * Download and decrypt an encrypted blob, returning the raw image bytes.
     */
    suspend fun downloadAndDecrypt(blobId: String): ByteArray = withContext(Dispatchers.IO) {
        val decrypted = photoRepository.downloadAndDecryptBlob(blobId)
        val payload = org.json.JSONObject(String(decrypted, Charsets.UTF_8))
        val dataBase64 = payload.getString("data")
        android.util.Base64.decode(dataBase64, android.util.Base64.NO_WRAP)
    }
}
