package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Shared Albums ────────────────────────────────────────────────────────────

/** Response item from `GET /api/sharing/albums`. */
data class SharedAlbumInfo(
    val id: String,
    val name: String,
    @SerializedName("owner_username") val ownerUsername: String,
    @SerializedName("is_owner") val isOwner: Boolean,
    @SerializedName("photo_count") val photoCount: Long,
    @SerializedName("member_count") val memberCount: Long,
    @SerializedName("created_at") val createdAt: String,
)

/** Response from `POST /api/sharing/albums`. */
data class CreateSharedAlbumResponse(
    val id: String,
    val name: String,
    @SerializedName("created_at") val createdAt: String,
)

/** Request body for `POST /api/sharing/albums`. */
data class CreateSharedAlbumRequest(
    val name: String,
)

// ── Members ──────────────────────────────────────────────────────────────────

/** Response item from `GET /api/sharing/albums/{id}/members`. */
data class SharedAlbumMember(
    val id: String,
    @SerializedName("user_id") val userId: String,
    val username: String,
    @SerializedName("added_at") val addedAt: String,
)

/** Request body for `POST /api/sharing/albums/{id}/members`. */
data class AddMemberRequest(
    @SerializedName("user_id") val userId: String,
)

/** Response from `POST /api/sharing/albums/{id}/members`. */
data class AddMemberResponse(
    @SerializedName("member_id") val memberId: String,
    @SerializedName("user_id") val userId: String,
)

// ── Photos ───────────────────────────────────────────────────────────────────

/** Response item from `GET /api/sharing/albums/{id}/photos`. */
data class SharedAlbumPhoto(
    val id: String,
    @SerializedName("photo_ref") val photoRef: String,
    @SerializedName("ref_type") val refType: String,
    @SerializedName("added_at") val addedAt: String,
)

/** Request body for `POST /api/sharing/albums/{id}/photos`. */
data class AddSharedPhotoRequest(
    @SerializedName("photo_ref") val photoRef: String,
    @SerializedName("ref_type") val refType: String = "plain",
)

/** Response from `POST /api/sharing/albums/{id}/photos`. */
data class AddSharedPhotoResponse(
    @SerializedName("photo_id") val photoId: String,
)

// ── User picker ──────────────────────────────────────────────────────────────

/** Response item from `GET /api/sharing/users`. */
data class ShareableUser(
    val id: String,
    val username: String,
)
