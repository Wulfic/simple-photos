package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

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
