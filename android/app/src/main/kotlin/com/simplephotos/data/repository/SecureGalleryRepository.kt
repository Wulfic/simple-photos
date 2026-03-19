/**
 * Repository for PIN-protected secure gallery operations including
 * creation, unlock, and item management.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.SecureGalleryAddItemRequest
import com.simplephotos.data.remote.dto.SecureGalleryAddItemResponse
import com.simplephotos.data.remote.dto.SecureGalleryCreateRequest
import com.simplephotos.data.remote.dto.SecureGalleryCreateResponse
import com.simplephotos.data.remote.dto.SecureGalleryItemsResponse
import com.simplephotos.data.remote.dto.SecureGalleryListResponse
import com.simplephotos.data.remote.dto.SecureGalleryUnlockRequest
import com.simplephotos.data.remote.dto.SecureGalleryUnlockResponse
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Repository for password-protected secure galleries.
 *
 * Secure galleries require a separate unlock step (with the user's account
 * password) before browsing is allowed.  All operations are server-side
 * only — no local caching.
 */
@Singleton
class SecureGalleryRepository @Inject constructor(
    private val api: ApiService
) {
    /** Verify password and obtain a short-lived gallery access token. */
    suspend fun unlock(password: String): SecureGalleryUnlockResponse =
        api.unlockSecureGalleries(SecureGalleryUnlockRequest(password))

    /** List all secure galleries for the current user. */
    suspend fun listGalleries(): SecureGalleryListResponse =
        api.listSecureGalleries()

    /** Create a new secure gallery with the given name. */
    suspend fun createGallery(name: String): SecureGalleryCreateResponse =
        api.createSecureGallery(SecureGalleryCreateRequest(name))

    /** Permanently delete a secure gallery and its item associations. */
    suspend fun deleteGallery(galleryId: String) {
        api.deleteSecureGallery(galleryId)
    }

    /** List items inside a secure gallery (requires gallery token from [unlock]). */
    suspend fun listItems(galleryId: String, galleryToken: String): SecureGalleryItemsResponse =
        api.listSecureGalleryItems(galleryId, galleryToken)

    /** Add a blob to a secure gallery. */
    suspend fun addItem(galleryId: String, blobId: String): SecureGalleryAddItemResponse =
        api.addSecureGalleryItem(galleryId, SecureGalleryAddItemRequest(blobId))

    /** Return all blob IDs that belong to any secure gallery for the current user. */
    suspend fun getSecureBlobIds(): Set<String> =
        try { api.getSecureBlobIds().blobIds.toSet() } catch (_: Exception) { emptySet() }
}
