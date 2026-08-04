/**
 * Secure gallery DTOs — create, unlock, list items, and manage
 * password-protected photo galleries.
 */
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
    @SerializedName("added_at") val addedAt: String,
    // Owning album — present on both the per-gallery and aggregate responses.
    // Needed to route a "remove" from a smart view to the real album; null only
    // on responses from older servers.
    @SerializedName("gallery_id") val galleryId: String? = null,
    // Only set by the aggregate /galleries/secure/items feed (smart-album header).
    @SerializedName("gallery_name") val galleryName: String? = null,
    @SerializedName("encrypted_thumb_blob_id") val encryptedThumbBlobId: String? = null,
    val width: Int? = null,
    val height: Int? = null,
    @SerializedName("media_type") val mediaType: String? = null,
    // Subtype-aware fields (mirrors the main gallery) so the secure viewer can
    // play videos, pan panoramas/360, play motion (LIVE) photos, and collapse
    // bursts. Sourced from the original photo server-side; see list_gallery_items.
    @SerializedName("photo_subtype") val photoSubtype: String? = null,
    @SerializedName("burst_id") val burstId: String? = null,
    @SerializedName("duration_secs") val durationSecs: Float? = null,
    @SerializedName("motion_video_blob_id") val motionVideoBlobId: String? = null,
    // Non-destructive crop/edit JSON stored on the secure item itself (#31),
    // same shape as a regular photo's crop_metadata. Applied at display time in
    // the tile + viewer; null = no edits.
    @SerializedName("crop_metadata") val cropMetadata: String? = null,
    // The #49 resolution ladder of the video this item hides, highest first.
    // Carried here for the same reason photoSubtype is: secured photos are
    // excluded from main-gallery sync, so the Room row the regular viewer reads
    // its ladder from never exists for them.
    //
    // Only a video secured AFTER its rungs were generated has one — generation
    // is gated on gallery eligibility, so securing first means no picker ever.
    // Null (pre-#49 server) and empty (no rungs) both collapse to "draw no
    // picker" via toDomain(), which is the same contract PhotoDto uses.
    val renditions: List<RenditionDto>? = null
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

/** Move a secure item into another of the user's secure albums (#31). */
data class SecureGalleryMoveItemRequest(
    @SerializedName("target_gallery_id") val targetGalleryId: String
)

/** Persist (or clear, with null) a secure item's crop/edit metadata (#31). */
data class SecureGallerySetCropRequest(
    @SerializedName("crop_metadata") val cropMetadata: String?
)
