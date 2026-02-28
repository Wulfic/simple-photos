package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Secure Galleries ─────────────────────────────────────────────────────────

data class SecureGallery(
    val id: String,
    val name: String,
    @SerializedName("created_at") val createdAt: String,
    @SerializedName("item_count") val itemCount: Int
)

data class SecureGalleryListResponse(
    val galleries: List<SecureGallery>
)

data class SecureGalleryCreateRequest(
    val name: String
)

data class SecureGalleryCreateResponse(
    @SerializedName("gallery_id") val galleryId: String,
    val name: String
)

data class SecureGalleryUnlockRequest(
    val password: String
)

data class SecureGalleryUnlockResponse(
    @SerializedName("gallery_token") val galleryToken: String,
    @SerializedName("expires_in") val expiresIn: Int
)

data class SecureGalleryItem(
    val id: String,
    @SerializedName("blob_id") val blobId: String,
    @SerializedName("added_at") val addedAt: String
)

data class SecureGalleryItemsResponse(
    val items: List<SecureGalleryItem>
)

data class SecureGalleryAddItemRequest(
    @SerializedName("blob_id") val blobId: String
)

data class SecureGalleryAddItemResponse(
    @SerializedName("item_id") val itemId: String
)

data class SecureBlobIdsResponse(
    @SerializedName("blob_ids") val blobIds: List<String>
)
