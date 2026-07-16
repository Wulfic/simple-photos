/**
 * Room DAO providing CRUD operations and reactive queries for albums
 * and the photo↔album cross-reference join table.
 */
package com.simplephotos.data.local.dao

import androidx.room.*
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoAlbumXRef
import kotlinx.coroutines.flow.Flow

/** Room DAO for albums and the photo↔album cross-reference table. */
@Dao
interface AlbumDao {
    @Query("SELECT * FROM albums ORDER BY name")
    fun getAllAlbums(): Flow<List<AlbumEntity>>

    @Query("SELECT * FROM albums WHERE localId = :id")
    suspend fun getById(id: String): AlbumEntity?

    @Query("SELECT * FROM albums WHERE serverManifestBlobId = :blobId LIMIT 1")
    suspend fun getByManifestBlobId(blobId: String): AlbumEntity?

    @Query("SELECT localId FROM albums")
    suspend fun getAllAlbumIds(): List<String>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(album: AlbumEntity)

    @Update
    suspend fun update(album: AlbumEntity)

    @Delete
    suspend fun delete(album: AlbumEntity)

    @Query("DELETE FROM albums WHERE localId = :id")
    suspend fun deleteById(id: String)

    @Query("DELETE FROM photo_album_xref WHERE albumLocalId = :albumId")
    suspend fun deleteAllXRefsForAlbum(albumId: String)

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertXRef(xRef: PhotoAlbumXRef)

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertXRefs(xRefs: List<PhotoAlbumXRef>)

    /**
     * Persist an album's visible member count, but only when it actually
     * changed. The `AND cachedCount != :count` is load-bearing: Room's
     * invalidation triggers fire per *changed row*, so an unconditional write of
     * an identical value would re-emit `getAllAlbums()`, re-run the count, and
     * write again — a self-sustaining refresh loop behind the album list.
     */
    @Query("UPDATE albums SET cachedCount = :count WHERE localId = :id AND cachedCount != :count")
    suspend fun updateCachedCount(id: String, count: Int)

    @Query("DELETE FROM photo_album_xref WHERE photoLocalId = :photoId AND albumLocalId = :albumId")
    suspend fun deleteXRef(photoId: String, albumId: String)

    @Query("SELECT photoLocalId FROM photo_album_xref WHERE albumLocalId = :albumId")
    suspend fun getPhotoIdsForAlbum(albumId: String): List<String>

    @Query("DELETE FROM albums")
    suspend fun deleteAll()

    @Query("DELETE FROM photo_album_xref")
    suspend fun deleteAllXRefs()
}
