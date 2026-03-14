/**
 * Room entity representing a device folder selected for automatic backup.
 */
package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

/**
 * Represents a device folder selected for automatic backup.
 * The [bucketId] is MediaStore's BUCKET_ID which uniquely identifies a folder.
 * [bucketName] is the human-readable folder name (e.g., "Camera", "Screenshots").
 * [relativePath] is the relative path on the device (e.g., "DCIM/Camera").
 */
@Entity(tableName = "backup_folders")
data class BackupFolderEntity(
    @PrimaryKey val bucketId: Long,
    val bucketName: String,
    val relativePath: String,
    val enabled: Boolean = true,
    val addedAt: Long = System.currentTimeMillis()
)
