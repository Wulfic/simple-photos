/**
 * Room entities for user-created albums and the photo↔album many-to-many join table.
 */
package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

/**
 * Local representation of a user-created album.
 *
 * In encrypted mode, the album's photo list is stored as an encrypted
 * manifest blob on the server ([serverManifestBlobId]). In plain mode,
 * albums are local-only.
 */
@Entity(tableName = "albums")
data class AlbumEntity(
    @PrimaryKey val localId: String,
    val serverManifestBlobId: String? = null,
    val name: String,
    val coverPhotoLocalId: String? = null,
    val syncStatus: SyncStatus = SyncStatus.PENDING,
    val createdAt: Long = System.currentTimeMillis()
)

/** Many-to-many join table linking photos to albums. */
@Entity(tableName = "photo_album_xref", primaryKeys = ["photoLocalId", "albumLocalId"])
data class PhotoAlbumXRef(
    val photoLocalId: String,
    val albumLocalId: String
)
