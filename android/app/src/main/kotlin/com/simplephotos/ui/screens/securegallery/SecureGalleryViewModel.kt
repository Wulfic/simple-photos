/**
 * ViewModel for the secure gallery screen, managing encrypted photo loading,
 * decryption, and secure album state.
 */
package com.simplephotos.ui.screens.securegallery

import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SecureGalleryRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import javax.inject.Inject

private const val TAG = "SecureGalleryVM"

// ─────────────────────────────────────────────────────────────────────────────
// ViewModel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Manages password-protected secure galleries: unlock with account password,
 * browse galleries, view encrypted items, and add photos.
 */
@HiltViewModel
class SecureGalleryViewModel @Inject constructor(
    private val secureGalleryRepository: SecureGalleryRepository,
    private val photoRepository: PhotoRepository
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
                Log.d(TAG, "Unlocking secure galleries…")
                val res = withContext(Dispatchers.IO) {
                    secureGalleryRepository.unlock(password)
                }
                galleryToken = res.galleryToken
                isAuthenticated = true
                Log.i(TAG, "Unlock successful, token obtained")
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Unlock failed", e)
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
                val res = withContext(Dispatchers.IO) { secureGalleryRepository.listGalleries() }
                galleries = res.galleries
                Log.d(TAG, "Loaded ${galleries.size} galleries")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to load galleries", e)
                error = "Failed to load galleries: ${e.message}"
            } finally {
                galleriesLoading = false
            }
        }
    }

    fun selectGallery(gallery: SecureGallery) {
        Log.d(TAG, "selectGallery: id=${gallery.id} name=${gallery.name} items=${gallery.itemCount}")
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
                    secureGalleryRepository.listItems(galleryId, galleryToken)
                }
                items = res.items
                Log.d(TAG, "Loaded ${items.size} items for gallery $galleryId")
                for (item in items) {
                    Log.d(TAG, "  item id=${item.id} blobId=${item.blobId} " +
                        "thumbBlobId=${item.encryptedThumbBlobId} " +
                        "w=${item.width} h=${item.height} type=${item.mediaType}")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to load items for gallery $galleryId", e)
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
                Log.d(TAG, "Loaded ${allPhotos.size} photos for picker")
            } catch (e: Exception) {
                Log.w(TAG, "Failed to load photos for picker", e)
            }
        }
    }

    fun createGallery(name: String) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    secureGalleryRepository.createGallery(name)
                }
                Log.i(TAG, "Created gallery: $name")
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Create gallery failed: $name", e)
                error = "Create failed: ${e.message}"
            }
        }
    }

    fun deleteGallery(gallery: SecureGallery) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { secureGalleryRepository.deleteGallery(gallery.id) }
                Log.i(TAG, "Deleted gallery: ${gallery.id}")
                if (selectedGallery?.id == gallery.id) {
                    selectedGallery = null
                    items = emptyList()
                }
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Delete gallery failed: ${gallery.id}", e)
                error = "Delete failed: ${e.message}"
            }
        }
    }

    fun addPhotosToGallery(blobIds: List<String>) {
        val gallery = selectedGallery ?: return
        Log.d(TAG, "addPhotosToGallery: ${blobIds.size} blobs → gallery ${gallery.id}")
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    coroutineScope {
                        blobIds.map { blobId ->
                            async {
                                Log.d(TAG, "  adding blobId=$blobId")
                                secureGalleryRepository.addItem(gallery.id, blobId)
                            }
                        }.awaitAll()
                    }
                }
                Log.i(TAG, "Added ${blobIds.size} photos to gallery ${gallery.id}")
                loadItems(gallery.id)
                loadGalleries()
            } catch (e: Exception) {
                Log.e(TAG, "Add photos failed", e)
                error = "Add photos failed: ${e.message}"
            }
        }
    }

    /**
     * Download and decrypt an encrypted blob, returning the raw image bytes.
     *
     * Uses file-based streaming to avoid OOM on large blobs:
     * encrypted blob → temp file → decrypt → stream-extract base64 → output file → read.
     */
    suspend fun downloadAndDecrypt(blobId: String): ByteArray = withContext(Dispatchers.IO) {
        Log.d(TAG, "downloadAndDecrypt: blobId=$blobId")
        val outputFile = File.createTempFile("secure_dec_", ".tmp", photoRepository.getCacheDir())
        try {
            photoRepository.downloadAndDecryptBlobToFile(blobId, outputFile)
            val bytes = outputFile.readBytes()
            Log.d(TAG, "downloadAndDecrypt: blobId=$blobId → ${bytes.size} bytes decoded")
            bytes
        } catch (e: Exception) {
            Log.e(TAG, "downloadAndDecrypt failed: blobId=$blobId", e)
            throw e
        } finally {
            outputFile.delete()
        }
    }

    /**
     * Download a small encrypted thumbnail for a secure gallery item.
     *
     * Resolution order:
     * 1. If [encryptedThumbBlobId] is provided (from server item metadata),
     *    download that blob directly — it's a small (~30 KB) thumbnail.
     * 2. Otherwise fall back to `GET /api/blobs/{blobId}/thumb` which asks
     *    the server to resolve the thumbnail from the photos table.
     * 3. Last resort: download the full blob (may be large).
     */
    suspend fun downloadThumb(blobId: String, encryptedThumbBlobId: String? = null): ByteArray = withContext(Dispatchers.IO) {
        // 1. Direct thumbnail blob download (most reliable for cloned/Android items)
        if (encryptedThumbBlobId != null) {
            Log.d(TAG, "downloadThumb: using encryptedThumbBlobId=$encryptedThumbBlobId for blobId=$blobId")
            try {
                val thumbBytes = photoRepository.downloadAndDecryptBlob(encryptedThumbBlobId)
                val payload = org.json.JSONObject(String(thumbBytes, Charsets.UTF_8))
                val dataBase64 = payload.getString("data")
                val decoded = android.util.Base64.decode(dataBase64, android.util.Base64.NO_WRAP)
                Log.d(TAG, "downloadThumb: direct thumb success, ${decoded.size} bytes")
                return@withContext decoded
            } catch (e: Exception) {
                Log.w(TAG, "downloadThumb: direct thumb download failed for $encryptedThumbBlobId, trying /thumb endpoint", e)
            }
        }

        // 2. Server-resolved thumbnail endpoint
        Log.d(TAG, "downloadThumb: trying /blobs/$blobId/thumb endpoint")
        try {
            val thumbBytes = photoRepository.downloadAndDecryptThumbBlob(blobId)
            if (thumbBytes != null) {
                val payload = org.json.JSONObject(String(thumbBytes, Charsets.UTF_8))
                val dataBase64 = payload.getString("data")
                val decoded = android.util.Base64.decode(dataBase64, android.util.Base64.NO_WRAP)
                Log.d(TAG, "downloadThumb: /thumb endpoint success, ${decoded.size} bytes")
                return@withContext decoded
            }
        } catch (e: Exception) {
            Log.w(TAG, "downloadThumb: /thumb endpoint failed for $blobId", e)
        }

        // 3. Fallback: download full blob via streaming (avoids OOM)
        Log.w(TAG, "downloadThumb: falling back to full blob download for $blobId")
        downloadAndDecrypt(blobId)
    }
}
