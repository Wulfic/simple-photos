/**
 * Room entity representing a pending blob upload in the backup queue.
 */
package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

/**
 * Upload queue entry for a pending blob (photo, thumbnail, video, or album manifest).
 *
 * [com.simplephotos.sync.BackupWorker] processes entries ordered by [priority] ASC
 * (thumbnails first, then media). Entries are retried up to 5 times before being marked "failed".
 */
@Entity(tableName = "blob_queue")
data class BlobQueueEntity(
    @PrimaryKey val id: String,
    val photoLocalId: String? = null,
    val albumLocalId: String? = null,
    val blobType: String,
    val priority: Int,
    val attempts: Int = 0,
    val lastAttemptAt: Long? = null,
    val status: String = "queued"
)
