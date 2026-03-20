/**
 * Room entity representing a photo in the local database.
 *
 * Stores both server-side metadata (server ID, blob IDs, timestamps) and
 * device-local state (content URI, sync status, file hash). Used by
 * [com.simplephotos.data.local.dao.PhotoDao] for all photo queries.
 */
package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

enum class SyncStatus { PENDING, UPLOADING, SYNCED, FAILED }

@Entity(tableName = "photos")
data class PhotoEntity(
    @PrimaryKey val localId: String,
    /** Server photo ID */
    val serverPhotoId: String? = null,
    /** Server blob ID for encrypted photos */
    val serverBlobId: String? = null,
    val thumbnailBlobId: String? = null,
    val filename: String,
    val takenAt: Long,
    val mimeType: String,
    /** "photo" | "gif" | "video" */
    val mediaType: String = "photo",
    val width: Int,
    val height: Int,
    /** Duration in seconds for videos, null for photos/GIFs */
    val durationSecs: Float? = null,
    val latitude: Double? = null,
    val longitude: Double? = null,
    /** content:// URI for locally-captured media, null for server-only */
    val localPath: String? = null,
    /** File path to cached thumbnail JPEG in app-internal storage */
    val thumbnailPath: String? = null,
    val syncStatus: SyncStatus = SyncStatus.PENDING,
    /** File size in bytes (from server) */
    val sizeBytes: Long? = null,
    val createdAt: Long = System.currentTimeMillis(),
    /** Whether this photo is favorited */
    val isFavorite: Boolean = false,
    /** JSON crop metadata for crop/edit feature */
    val cropMetadata: String? = null,
    /** Camera model / device name (from EXIF data) */
    val cameraModel: String? = null,
    /** Short content-based hash (12 hex chars of SHA-256) for cross-platform alignment */
    val photoHash: String? = null
)
