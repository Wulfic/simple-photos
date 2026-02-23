package com.simplephotos.data.local.dao

import androidx.room.*
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import kotlinx.coroutines.flow.Flow

@Dao
interface PhotoDao {
    @Query("SELECT * FROM photos ORDER BY takenAt DESC")
    fun getAllPhotos(): Flow<List<PhotoEntity>>

    @Query("SELECT * FROM photos WHERE localId = :id")
    suspend fun getById(id: String): PhotoEntity?

    @Query("SELECT * FROM photos WHERE syncStatus = :status")
    suspend fun getByStatus(status: SyncStatus): List<PhotoEntity>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(photo: PhotoEntity)

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertAll(photos: List<PhotoEntity>)

    @Update
    suspend fun update(photo: PhotoEntity)

    @Query("UPDATE photos SET syncStatus = :status WHERE localId = :id")
    suspend fun updateSyncStatus(id: String, status: SyncStatus)

    @Query("UPDATE photos SET serverBlobId = :blobId, thumbnailBlobId = :thumbBlobId, syncStatus = 'SYNCED' WHERE localId = :localId")
    suspend fun markSynced(localId: String, blobId: String, thumbBlobId: String?)

    @Query("UPDATE photos SET thumbnailPath = :path WHERE localId = :id")
    suspend fun updateThumbnailPath(id: String, path: String)

    @Query("SELECT * FROM photos WHERE serverBlobId = :blobId LIMIT 1")
    suspend fun getByServerBlobId(blobId: String): PhotoEntity?

    @Delete
    suspend fun delete(photo: PhotoEntity)

    @Query("DELETE FROM photos WHERE localId = :id")
    suspend fun deleteById(id: String)
}
