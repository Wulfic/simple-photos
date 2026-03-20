/**
 * Room DAO for photo CRUD operations, reactive queries, and sync-status management.
 */
package com.simplephotos.data.local.dao

import androidx.room.*
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import kotlinx.coroutines.flow.Flow

/** Room DAO for [PhotoEntity] CRUD, reactive queries, and sync-status management. */
@Dao
interface PhotoDao {
    // Secondary sort by filename ensures deterministic order when takenAt ties
    // (matches server's COALESCE(taken_at, created_at) DESC, filename ASC)
    @Query("SELECT * FROM photos ORDER BY takenAt DESC, filename ASC")
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

    /**
     * Reset photos stuck at UPLOADING (from a crash) back to PENDING so they get retried.
     * Only resets photos that haven't already been successfully uploaded
     * (no serverBlobId and no serverPhotoId) — prevents re-uploading duplicates.
     */
    @Query("UPDATE photos SET syncStatus = 'PENDING' WHERE syncStatus = 'UPLOADING' AND serverBlobId IS NULL AND serverPhotoId IS NULL")
    suspend fun resetStuckUploading()

    /** Find a photo with the given photoHash that is already SYNCED. */
    @Query("SELECT * FROM photos WHERE photoHash = :hash AND syncStatus = 'SYNCED' LIMIT 1")
    suspend fun getSyncedByHash(hash: String): PhotoEntity?

    @Query("UPDATE photos SET serverBlobId = :blobId, thumbnailBlobId = :thumbBlobId, syncStatus = 'SYNCED' WHERE localId = :localId")
    suspend fun markSynced(localId: String, blobId: String, thumbBlobId: String?)

    @Query("UPDATE photos SET serverPhotoId = :photoId, syncStatus = 'SYNCED' WHERE localId = :localId")
    suspend fun markSynced(localId: String, photoId: String)

    @Query("UPDATE photos SET thumbnailPath = :path WHERE localId = :id")
    suspend fun updateThumbnailPath(id: String, path: String)

    @Query("UPDATE photos SET cropMetadata = :metadata WHERE localId = :id")
    suspend fun updateCropMetadata(id: String, metadata: String?)

    @Query("SELECT * FROM photos WHERE serverBlobId = :blobId LIMIT 1")
    suspend fun getByServerBlobId(blobId: String): PhotoEntity?

    @Query("SELECT * FROM photos WHERE serverPhotoId = :photoId LIMIT 1")
    suspend fun getByServerPhotoId(photoId: String): PhotoEntity?

    @Query("SELECT * FROM photos WHERE localPath = :path LIMIT 1")
    suspend fun getByLocalPath(path: String): PhotoEntity?

    /** Find a SYNCED photo with the same filename (for server dedup). */
    @Query("SELECT * FROM photos WHERE filename = :filename AND syncStatus = 'SYNCED' LIMIT 1")
    suspend fun getSyncedByFilename(filename: String): PhotoEntity?

    @Delete
    suspend fun delete(photo: PhotoEntity)

    @Query("DELETE FROM photos WHERE localId = :id")
    suspend fun deleteById(id: String)

    @Query("DELETE FROM photos")
    suspend fun deleteAll()
}
