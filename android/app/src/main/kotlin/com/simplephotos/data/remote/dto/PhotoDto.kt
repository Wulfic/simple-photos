/**
 * Photo DTOs — photo records, upload/register payloads, encrypted sync
 * responses, favorites, crop metadata, and edit copies.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Photo models ─────────────────────────────────────────────────────────────

data class PhotoRecord(
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

data class PhotoListResponse(
    val photos: List<PhotoRecord>,
    @SerializedName("next_cursor") val nextCursor: String?
)

data class PhotoUploadResponse(
    @SerializedName("photo_id") val photoId: String,
    val filename: String,
    @SerializedName("size_bytes") val sizeBytes: Long
)

// ── Photo register (non-encrypted) ────────────────────────────────────────────

data class RegisterPhotoRequest(
    val filename: String,
    @SerializedName("file_path") val filePath: String,
    @SerializedName("mime_type") val mimeType: String,
    @SerializedName("media_type") val mediaType: String? = null,
    @SerializedName("size_bytes") val sizeBytes: Long,
    val width: Int? = null,
    val height: Int? = null,
    @SerializedName("duration_secs") val durationSecs: Double? = null,
    @SerializedName("taken_at") val takenAt: String? = null,
    val latitude: Double? = null,
    val longitude: Double? = null,
)

data class RegisterPhotoResponse(
    @SerializedName("photo_id") val photoId: String,
    @SerializedName("thumb_path") val thumbPath: String? = null,
    @SerializedName("photo_hash") val photoHash: String? = null,
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
    @SerializedName("crop_metadata") val cropMetadata: String? = null,
    @SerializedName("photo_hash") val photoHash: String? = null,
    @SerializedName("source_path") val sourcePath: String? = null,
    @SerializedName("photo_subtype") val photoSubtype: String? = null,
    @SerializedName("burst_id") val burstId: String? = null,
    @SerializedName("motion_video_blob_id") val motionVideoBlobId: String? = null,
    /**
     * The video resolution ladder (#49), highest quality first, `is_source`
     * marking the untouched original.
     *
     * Null means the server predates #49; an **empty list is the normal case**
     * and means "one quality, draw no picker". Those two states are deliberately
     * collapsed by `renditionsEqual` rather than distinguished — see
     * [com.simplephotos.data.media.renditionsEqual].
     *
     * Gson has silently ignored this field since the server started sending it,
     * which is why `8564636` broke neither client.
     */
    val renditions: List<RenditionDto>? = null,
)

/**
 * One playable quality of a video (#49).
 *
 * `file_path` is deliberately absent from the wire shape — it is a server-side
 * storage path no client can fetch — so [shortEdge] doubles as the selector for
 * the `?rendition=` form used on unencrypted installs.
 */
data class RenditionDto(
    @SerializedName("short_edge") val shortEdge: Int,
    val width: Int,
    val height: Int,
    @SerializedName("is_source") val isSource: Boolean = false,
    /**
     * Encrypted mode: stream as `spblob://<blob_id>`.
     *
     * Null on an unencrypted install, where the bytes live behind
     * `GET /api/photos/{id}/file?rendition={short_edge}`. This client has no
     * plaintext playback branch, so such rungs are filtered out of the picker.
     */
    @SerializedName("blob_id") val blobId: String? = null,
    val codec: String? = null,
    @SerializedName("size_bytes") val sizeBytes: Long = 0,
)

/**
 * Wire shape → the domain model the picker and Room both use.
 *
 * Null collapses to empty deliberately: a pre-#49 server sends no field at all
 * and a #49 server sends `[]` for the ~600 videos needing no rung, and nothing
 * downstream can act on the difference between them.
 */
fun List<RenditionDto>?.toDomain(): List<com.simplephotos.data.media.Rendition> =
    this.orEmpty().map {
        com.simplephotos.data.media.Rendition(
            shortEdge = it.shortEdge,
            width = it.width,
            height = it.height,
            isSource = it.isSource,
            blobId = it.blobId,
            codec = it.codec,
            sizeBytes = it.sizeBytes,
        )
    }

data class EncryptedSyncResponse(
    val photos: List<EncryptedSyncRecord>,
    @SerializedName("next_cursor") val nextCursor: String?,
    /**
     * Photo ids that have LEFT the eligible feed — deleted outright, or claimed
     * by a secure gallery. The client treats both identically (#38).
     *
     * The nullability is the protocol handshake, not defensiveness. On a delta
     * the server sends this **empty rather than absent**; a server predating #38
     * ignores the unknown `since` parameter and replies with a full walk, whose
     * `photos` are indistinguishable from a delta's. Null therefore means "this
     * server does not speak `since`" and forces the full path — see
     * [com.simplephotos.data.sync.isDeltaFeed]. Gson leaves an absent field at
     * its default, so null here really is "the key was not in the JSON".
     */
    val deleted: List<String>? = null,
    /**
     * The change log's head at the moment this page was computed.
     *
     * Persist the **first** page's value, never the last: a change committed
     * while a multi-page walk is in flight lands above the first page's head, so
     * keeping the first re-delivers it next pass while keeping the last steps
     * over it and loses it permanently.
     *
     * Null on a pre-#38 server, which keeps this client on full walks — correct,
     * just not fast.
     */
    @SerializedName("head_seq") val headSeq: Long? = null,
)

// ── Precomputed gallery count summary (Issue 3, revised by #42) ──────────────
// GET /api/photos/summary — cheap server-side aggregate, and the AUTHORITATIVE
// source for smart-album badges. The Room mirror only holds rows that carry an
// encrypted blob, so counting it under-reports the library by the whole
// pending-encryption backlog (measured live: 2,494 of 14,874 rows).
//
// Two families of number, NOT interchangeable — mirrors `PhotoSummary` in
// server/src/gallery/summary.rs:
//   - total/photos/gifs/videos/audio/favorites are raw media-type ROW counts.
//   - smart* are TILE counts: the smart-album filter applied first, burst
//     frames collapsed second. Badges must use these.
// `smartPhotos` counts photos AND gifs, because the "Photos" smart album is
// defined that way on both clients.
//
// Defaults of -1 on the smart* fields are load-bearing: Gson leaves absent
// fields at their default, so a server on a pre-#42 binary is detectable via
// `hasTileCounts` rather than silently reporting zero photos.

data class PhotoSummaryDto(
    val total: Long = 0,
    @SerializedName("collapsed_total") val collapsedTotal: Long = 0,
    val photos: Long = 0,
    val gifs: Long = 0,
    val videos: Long = 0,
    val audio: Long = 0,
    val favorites: Long = 0,
    @SerializedName("smart_photos") val smartPhotos: Long = -1,
    @SerializedName("smart_gifs") val smartGifs: Long = -1,
    @SerializedName("smart_videos") val smartVideos: Long = -1,
    @SerializedName("smart_audio") val smartAudio: Long = -1,
    @SerializedName("smart_favorites") val smartFavorites: Long = -1,
    @SerializedName("smart_recent") val smartRecent: Long = -1,
    /**
     * The photo change log's current head (#38).
     *
     * Deliberately NOT served from the summary's TTL cache on the server — a
     * stale head here would make a client skip real changes, which is exactly
     * the busywork the delta protocol removes. Null on a pre-#38 server, which
     * costs the skip shortcut and nothing else.
     */
    @SerializedName("head_seq") val headSeq: Long? = null,
) {
    /** False when the server predates #42 and sent no tile counts, in which
     *  case the caller must fall back to local mirror counts rather than
     *  rendering -1 (or 0) into every badge. */
    val hasTileCounts: Boolean get() = smartPhotos >= 0
}

// ── Authoritative Takeout source albums (Issue 2) ────────────────────────────
// GET /api/photos/source-albums — album membership captured at import time,
// keyed by photo id (survives filename collisions and `-edited` dedup). Used to
// rebuild album manifests deterministically and cross-platform.

data class SourceAlbumDto(
    /**
     * The Takeout folder name — the album's identity (the deterministic album id
     * derives from it), so it stays stable even though Google mangles it on
     * export ("Mum & Dad's 40th" ships as "Mum _ Dad_s 40th").
     */
    val name: String,
    /**
     * The album's real Google Photos title, read server-side from the album
     * folder's `metadata.json`. Display this in preference to [name]; null on
     * older exports that carry no album metadata — then fall back to [name].
     */
    val title: String? = null,
    val source: String,
    @SerializedName("photo_ids") val photoIds: List<String>,
)

data class SourceAlbumsResponse(
    val albums: List<SourceAlbumDto>
)

// POST /api/photos/source-albums/dismiss — tombstone a reconstructed album the
// user deleted, so reconstruction stops recreating it on every device. Keyed by
// the local album id; the server resolves it back to the album identity.

data class DismissSourceAlbumRequest(
    @SerializedName("album_id") val albumId: String,
)

data class DismissSourceAlbumResponse(
    /** False when the id wasn't a source album (an ordinary user album). */
    val dismissed: Boolean,
    val name: String? = null,
)

// ── Crop-metadata sync (lightweight delta for non-destructive edits) ─────────

data class CropSyncRecord(
    val id: String,
    @SerializedName("crop_metadata") val cropMetadata: String? = null
)

// ── Favorite sync (lightweight delta for cross-device favorites) ─────────────

data class FavSyncRecord(
    val id: String,
    @SerializedName("is_favorite") val isFavorite: Boolean = false
)

// ── Storage stats ────────────────────────────────────────────────────────────

data class StorageStatsResponse(
    @SerializedName("photo_bytes") val photoBytes: Long,
    @SerializedName("photo_count") val photoCount: Long,
    @SerializedName("video_bytes") val videoBytes: Long,
    @SerializedName("video_count") val videoCount: Long,
    @SerializedName("other_blob_bytes") val otherBlobBytes: Long,
    @SerializedName("other_blob_count") val otherBlobCount: Long,
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

// ── Burst detection ────────────────────────────────────────────────────────────

data class DetectBurstsResponse(
    @SerializedName("burst_groups_created") val burstGroupsCreated: Int = 0
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

// ── Duplicate (Save Copy) ────────────────────────────────────────────────────

data class DuplicatePhotoRequest(
    @SerializedName("crop_metadata") val cropMetadata: String?
)

data class DuplicatePhotoResponse(
    val id: String,
    @SerializedName("source_photo_id") val sourcePhotoId: String,
    val filename: String,
    @SerializedName("crop_metadata") val cropMetadata: String?,
    val width: Int = 0,
    val height: Int = 0,
    @SerializedName("duration_secs") val durationSecs: Float? = null,
    @SerializedName("mime_type") val mimeType: String? = null,
    @SerializedName("media_type") val mediaType: String? = null,
    @SerializedName("size_bytes") val sizeBytes: Long? = null,
    @SerializedName("encrypted_blob_id") val encryptedBlobId: String? = null,
    @SerializedName("encrypted_thumb_blob_id") val encryptedThumbBlobId: String? = null,
)

// ── 2FA Status ───────────────────────────────────────────────────────────────

data class TwoFactorStatusResponse(
    @SerializedName("totp_enabled") val totpEnabled: Boolean
)

// ── Encryption key ───────────────────────────────────────────────────────────

data class StoreEncryptionKeyRequest(val key: String)
data class StoreEncryptionKeyResponse(val message: String)

// ── Backup servers ───────────────────────────────────────────────────────────

data class BackupServer(
    val id: String,
    val name: String,
    val address: String,
    @SerializedName("api_key") val apiKey: String? = null,
    val enabled: Boolean,
    @SerializedName("sync_frequency_hours") val syncFrequencyHours: Int,
    @SerializedName("last_sync_at") val lastSyncAt: String? = null,
    @SerializedName("last_sync_status") val lastSyncStatus: String? = null,
    @SerializedName("last_sync_error") val lastSyncError: String? = null,
    @SerializedName("created_at") val createdAt: String? = null,
)

data class UpdateBackupServerRequest(
    val name: String? = null,
    val address: String? = null,
    @SerializedName("api_key") val apiKey: String? = null,
    @SerializedName("sync_frequency_hours") val syncFrequencyHours: Int? = null,
    val enabled: Boolean? = null,
)

data class BackupServerStatusResponse(
    val reachable: Boolean,
    val version: String? = null,
    val error: String? = null,
)

data class BackupSyncLog(
    val id: String,
    @SerializedName("server_id") val serverId: String,
    @SerializedName("started_at") val startedAt: String,
    @SerializedName("completed_at") val completedAt: String? = null,
    val status: String,
    @SerializedName("photos_synced") val photosSynced: Int = 0,
    @SerializedName("bytes_synced") val bytesSynced: Long = 0,
    val error: String? = null,
)

data class BackupSyncStartedResponse(
    val message: String,
    @SerializedName("sync_id") val syncId: String? = null,
)

data class BackupDiscoverServer(
    val address: String,
    val name: String,
    val version: String,
)

data class BackupDiscoverResponse(
    val servers: List<BackupDiscoverServer>,
)

data class BackupServerListResponse(
    val servers: List<BackupServer>
)

data class AddBackupServerRequest(
    val name: String,
    val address: String,
    @SerializedName("api_key") val apiKey: String? = null,
    @SerializedName("sync_frequency_hours") val syncFrequencyHours: Int? = null,
)

data class RecoverResponse(val message: String)

// ── Audio backup setting ─────────────────────────────────────────────────────

data class AudioBackupResponse(
    @SerializedName("audio_backup_enabled") val audioBackupEnabled: Boolean,
    val message: String? = null
)

data class SetAudioBackupRequest(
    @SerializedName("audio_backup_enabled") val audioBackupEnabled: Boolean
)

// ── SSL/TLS settings ─────────────────────────────────────────────────────────

data class SslStatusResponse(
    val enabled: Boolean,
    @SerializedName("cert_path") val certPath: String? = null,
    @SerializedName("key_path") val keyPath: String? = null,
    val message: String? = null
)

// ── Conversion status ────────────────────────────────────────────────────────

data class ConversionStatusResponse(
    val active: Boolean,
    val total: Int,
    val done: Int,
    /** Server-authoritative seconds remaining, null until throughput is known (TODO #4/#5). */
    @SerializedName("eta_seconds") val etaSeconds: Double? = null,
)

// ── Batch dimension update ───────────────────────────────────────────────────

data class DimensionUpdateItem(
    @SerializedName("photo_id") val photoId: String? = null,
    @SerializedName("blob_id") val blobId: String? = null,
    val width: Int,
    val height: Int
)

data class BatchDimensionUpdateRequest(
    val updates: List<DimensionUpdateItem>
)

data class BatchDimensionUpdateResponse(
    val updated: Int
)
