package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

@Entity(tableName = "albums")
data class AlbumEntity(
    @PrimaryKey val localId: String,
    val serverManifestBlobId: String? = null,
    val name: String,
    val coverPhotoLocalId: String? = null,
    val syncStatus: SyncStatus = SyncStatus.PENDING,
    val createdAt: Long = System.currentTimeMillis()
)

@Entity(tableName = "photo_album_xref", primaryKeys = ["photoLocalId", "albumLocalId"])
data class PhotoAlbumXRef(
    val photoLocalId: String,
    val albumLocalId: String
)
