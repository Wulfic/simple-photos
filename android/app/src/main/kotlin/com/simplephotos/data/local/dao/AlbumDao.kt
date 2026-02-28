package com.simplephotos.data.local.dao

import androidx.room.*
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoAlbumXRef
import kotlinx.coroutines.flow.Flow

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

    @Query("DELETE FROM photo_album_xref WHERE photoLocalId = :photoId AND albumLocalId = :albumId")
    suspend fun deleteXRef(photoId: String, albumId: String)

    @Query("SELECT photoLocalId FROM photo_album_xref WHERE albumLocalId = :albumId")
    suspend fun getPhotoIdsForAlbum(albumId: String): List<String>
}
