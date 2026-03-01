package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Plain-mode photo models ──────────────────────────────────────────────────

data class PlainPhotoRecord(
    val id: String,
    val filename: String,
    @SerializedName("file_path") val filePath: String,
    @SerializedName("mime_type") val mimeType: String,
    @SerializedName("media_type") val mediaType: String,
    @SerializedName("size_bytes") val sizeBytes: Long,
    val width: Long,
    val height: Long,
    @SerializedName("duration_secs") val durationSecs: Double? = null,
    @SerializedName("taken_at") val takenAt: String? = null,
    val latitude: Double? = null,
    val longitude: Double? = null,
    @SerializedName("thumb_path") val thumbPath: String? = null,
    @SerializedName("created_at") val createdAt: String,
    @SerializedName("is_favorite") val isFavorite: Boolean = false,
    @SerializedName("crop_metadata") val cropMetadata: String? = null,
    @SerializedName("camera_model") val cameraModel: String? = null,
    @SerializedName("photo_hash") val photoHash: String? = null
)

data class PlainPhotoListResponse(
    val photos: List<PlainPhotoRecord>,
    @SerializedName("next_cursor") val nextCursor: String?
)

data class PhotoUploadResponse(
    @SerializedName("photo_id") val photoId: String,
    val filename: String,
    @SerializedName("size_bytes") val sizeBytes: Long
)

// ── Encryption settings ──────────────────────────────────────────────────────

data class EncryptionSettingsResponse(
    @SerializedName("encryption_mode") val encryptionMode: String,
    @SerializedName("migration_status") val migrationStatus: String,
    @SerializedName("migration_total") val migrationTotal: Long,
    @SerializedName("migration_completed") val migrationCompleted: Long,
    @SerializedName("migration_error") val migrationError: String?
)

// ── Encrypted-mode sync (lightweight manifest from photos table) ─────────────

data class EncryptedSyncRecord(
    val id: String,
    val filename: String,
    @SerializedName("mime_type") val mimeType: String,
    @SerializedName("media_type") val mediaType: String,
    @SerializedName("size_bytes") val sizeBytes: Long,
    val width: Long,
    val height: Long,
    @SerializedName("duration_secs") val durationSecs: Double? = null,
    @SerializedName("taken_at") val takenAt: String? = null,
    @SerializedName("created_at") val createdAt: String,
    @SerializedName("encrypted_blob_id") val encryptedBlobId: String? = null,
    @SerializedName("encrypted_thumb_blob_id") val encryptedThumbBlobId: String? = null,
    @SerializedName("is_favorite") val isFavorite: Boolean = false,
    @SerializedName("photo_hash") val photoHash: String? = null
)

data class EncryptedSyncResponse(
    val photos: List<EncryptedSyncRecord>,
    @SerializedName("next_cursor") val nextCursor: String?
)

// ── Storage stats ────────────────────────────────────────────────────────────

data class StorageStatsResponse(
    @SerializedName("photo_bytes") val photoBytes: Long,
    @SerializedName("photo_count") val photoCount: Long,
    @SerializedName("video_bytes") val videoBytes: Long,
    @SerializedName("video_count") val videoCount: Long,
    @SerializedName("other_blob_bytes") val otherBlobBytes: Long,
    @SerializedName("other_blob_count") val otherBlobCount: Long,
    @SerializedName("plain_bytes") val plainBytes: Long,
    @SerializedName("plain_count") val plainCount: Long,
    @SerializedName("user_total_bytes") val userTotalBytes: Long,
    @SerializedName("fs_total_bytes") val fsTotalBytes: Long,
    @SerializedName("fs_free_bytes") val fsFreeBytes: Long
)

// ── Change password ──────────────────────────────────────────────────────────

data class ChangePasswordRequest(
    @SerializedName("current_password") val currentPassword: String,
    @SerializedName("new_password") val newPassword: String
)

// ── Verify password ──────────────────────────────────────────────────────────

data class VerifyPasswordRequest(
    val password: String
)

// ── Admin user management ────────────────────────────────────────────────────

data class AdminUser(
    val id: String,
    val username: String,
    val role: String,
    @SerializedName("totp_enabled") val totpEnabled: Boolean,
    @SerializedName("created_at") val createdAt: String
)

data class CreateUserRequest(
    val username: String,
    val password: String,
    val role: String? = null
)

data class CreateUserResponse(
    @SerializedName("user_id") val userId: String,
    val username: String,
    val role: String
)

data class UpdateRoleRequest(val role: String)
data class UpdateRoleResponse(
    val message: String,
    @SerializedName("user_id") val userId: String,
    val role: String
)

data class ResetPasswordRequest(
    @SerializedName("new_password") val newPassword: String
)

data class MessageResponse(val message: String)

// ── Scan ─────────────────────────────────────────────────────────────────────

data class ScanResponse(
    val registered: Int,
    val message: String
)

// ── Favorites ────────────────────────────────────────────────────────────────

data class FavoriteToggleResponse(
    val id: String,
    @SerializedName("is_favorite") val isFavorite: Boolean
)

// ── Crop Metadata ────────────────────────────────────────────────────────────

data class SetCropRequest(
    @SerializedName("crop_metadata") val cropMetadata: String?
)

data class CropResponse(
    val id: String,
    @SerializedName("crop_metadata") val cropMetadata: String?
)
