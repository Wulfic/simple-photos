package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

enum class SyncStatus { PENDING, UPLOADING, SYNCED, FAILED }

@Entity(tableName = "photos")
data class PhotoEntity(
    @PrimaryKey val localId: String,
    /** Server photo ID for plain-mode photos */
    val serverPhotoId: String? = null,
    /** Server blob ID for encrypted-mode photos */
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
    val encryptedBlobSize: Long? = null,
    /** File size in bytes (from server, for plain-mode photos) */
    val sizeBytes: Long? = null,
    val createdAt: Long = System.currentTimeMillis(),
    /** Whether this photo is favorited */
    val isFavorite: Boolean = false,
    /** JSON crop metadata for crop/edit feature */
    val cropMetadata: String? = null,
    /** Camera model / device name (from EXIF data) */
    val cameraModel: String? = null
)
