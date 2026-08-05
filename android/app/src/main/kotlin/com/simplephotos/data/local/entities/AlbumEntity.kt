/**
 * Room entities for user-created albums and the photo↔album many-to-many join table.
 */
package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

/**
 * Local representation of a user-created album.
 *
 * The album's photo list is stored as an encrypted manifest blob on the
 * server ([serverManifestBlobId]).
 */
@Entity(tableName = "albums")
data class AlbumEntity(
    @PrimaryKey val localId: String,
    val serverManifestBlobId: String? = null,
    val name: String,
    val coverPhotoLocalId: String? = null,
    val syncStatus: SyncStatus = SyncStatus.PENDING,
    val createdAt: Long = System.currentTimeMillis(),
    /**
     * The manifest's membership **verbatim**, exactly as the server manifest
     * carried it — server blob ids, including members this device has not synced
     * into its photo mirror yet.
     *
     * This is the album's membership of record and the only thing an upload is
     * ever built from. The xref table is a *derived* view of it (the subset that
     * resolves to a local photo), for the detail grid's joins.
     *
     * Before this column existed, xrefs were the only membership store, so a
     * partially-synced device forgot every member it couldn't resolve locally —
     * and then uploaded that truncated list back as the album's new manifest.
     * Two devices with different partial mirrors would each shrink the album to
     * their own subset, forever, and the count followed whichever synced last.
     */
    val photoBlobIds: List<String> = emptyList(),
    /**
     * Last computed visible member count (`photoBlobIds ∩ mirror − secure`).
     *
     * Persisted purely so a cold start can render the previous, stable number
     * immediately instead of counting up from 0 while the mirror loads. Always
     * reconciled in the background; never authoritative.
     */
    val cachedCount: Int = 0,
)

/** Many-to-many join table linking photos to albums. */
@Entity(tableName = "photo_album_xref", primaryKeys = ["photoLocalId", "albumLocalId"])
data class PhotoAlbumXRef(
    val photoLocalId: String,
    val albumLocalId: String
)
