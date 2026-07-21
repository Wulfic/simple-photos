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

    /** Non-reactive snapshot of all photos (for dedup and batch operations). */
    @Query("SELECT * FROM photos")
    suspend fun getAllPhotosSnapshot(): List<PhotoEntity>

    /** How many photos the local mirror holds. Counts in SQLite rather than
     *  materialising every row, so it stays cheap on large libraries. */
    @Query("SELECT COUNT(*) FROM photos")
    suspend fun countAll(): Int

    @Query("SELECT * FROM photos WHERE localId = :id")
    suspend fun getById(id: String): PhotoEntity?

    @Query("SELECT * FROM photos WHERE syncStatus = :status")
    suspend fun getByStatus(status: SyncStatus): List<PhotoEntity>

    /**
     * Count items still queued for upload/encryption on this device — PENDING or
     * FAILED rows that were never successfully uploaded (no serverBlobId). Fed to
     * the server's `/status/encryption/contribute` so the unified banner total
     * includes local backup work the server can't see yet (TODO #2).
     */
    @Query("SELECT COUNT(*) FROM photos WHERE (syncStatus = 'PENDING' OR syncStatus = 'FAILED') AND serverBlobId IS NULL")
    suspend fun countPendingUploads(): Int

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

    /** Find a local entity (has localPath, no serverPhotoId) matching by photoHash — for merge during sync. */
    @Query("SELECT * FROM photos WHERE photoHash = :hash AND serverPhotoId IS NULL AND localPath IS NOT NULL LIMIT 1")
    suspend fun getLocalByHash(hash: String): PhotoEntity?

    /** Find a local entity (has localPath, no serverPhotoId) matching by filename + takenAt — fallback merge. */
    @Query("SELECT * FROM photos WHERE filename = :filename AND takenAt = :takenAt AND serverPhotoId IS NULL AND localPath IS NOT NULL LIMIT 1")
    suspend fun getLocalByFilenameAndDate(filename: String, takenAt: Long): PhotoEntity?

    @Query("UPDATE photos SET serverPhotoId = :serverPhotoId, serverBlobId = :blobId, thumbnailBlobId = :thumbBlobId, photoHash = :photoHash, syncStatus = 'SYNCED' WHERE localId = :localId")
    suspend fun markSynced(localId: String, serverPhotoId: String, blobId: String, thumbBlobId: String?, photoHash: String?)

    @Query("UPDATE photos SET serverPhotoId = :photoId, syncStatus = 'SYNCED' WHERE localId = :localId")
    suspend fun markSynced(localId: String, photoId: String)

    /** Merge a server photo into an existing local entity — sets serverPhotoId, blobId, thumbBlobId, cropMetadata, photoHash, isFavorite. */
    @Query("UPDATE photos SET serverPhotoId = :serverPhotoId, serverBlobId = :blobId, thumbnailBlobId = :thumbBlobId, cropMetadata = :cropMetadata, photoHash = :photoHash, isFavorite = :isFavorite, syncStatus = 'SYNCED' WHERE localId = :localId")
    suspend fun mergeServerPhoto(localId: String, serverPhotoId: String, blobId: String, thumbBlobId: String?, cropMetadata: String?, photoHash: String?, isFavorite: Boolean)

    /**
     * Back-fill the burst / motion / photo-subtype fields on an already-synced
     * row. The encrypted-sync pull skips photos that already exist locally, so
     * subtype data added on the server (or computed after first sync) would
     * otherwise never reach the device.
     */
    @Query("UPDATE photos SET photoSubtype = :subtype, burstId = :burstId, motionVideoBlobId = :motionBlobId WHERE serverPhotoId = :serverPhotoId")
    suspend fun backfillSubtypeFields(
        serverPhotoId: String,
        subtype: String?,
        burstId: String?,
        motionBlobId: String?,
    )

    /**
     * Land the #49 resolution ladder on an already-synced photo.
     *
     * Separate from [backfillSubtypeFields] because it fires on a different
     * schedule: a rung is produced by a background sweep *long after* the photo
     * synced, so the ladder arrives on a row that has been local for weeks. The
     * caller must guard with `renditionsEqual` — the ladder is unchanged for
     * every photo on every pass, and writing it unconditionally is the
     * O(library) write amplification #38 spent a workstream removing.
     */
    @Query("UPDATE photos SET renditions = :renditions WHERE serverPhotoId = :serverPhotoId")
    suspend fun updateRenditions(serverPhotoId: String, renditions: List<com.simplephotos.data.media.Rendition>)

    @Query("UPDATE photos SET thumbnailPath = :path WHERE localId = :id")
    suspend fun updateThumbnailPath(id: String, path: String)

    @Query("UPDATE photos SET isFavorite = :isFavorite WHERE localId = :id")
    suspend fun updateFavorite(id: String, isFavorite: Boolean)

    @Query("UPDATE photos SET cropMetadata = :metadata WHERE localId = :id")
    suspend fun updateCropMetadata(id: String, metadata: String?)

    /** Persist a manual photo-subtype correction (Info-panel edit) WITHOUT
     *  clobbering the burst/motion fields that [backfillSubtypeFields] also
     *  writes. Matched by serverPhotoId so it lines up with the edit API. */
    @Query("UPDATE photos SET photoSubtype = :subtype WHERE serverPhotoId = :serverPhotoId")
    suspend fun updatePhotoSubtype(serverPhotoId: String, subtype: String?)

    @Query("SELECT * FROM photos WHERE serverBlobId = :blobId LIMIT 1")
    suspend fun getByServerBlobId(blobId: String): PhotoEntity?

    @Query("SELECT * FROM photos WHERE serverPhotoId = :photoId LIMIT 1")
    suspend fun getByServerPhotoId(photoId: String): PhotoEntity?

    /** Batch lookup: get all photos whose serverPhotoId is in the given list.
     *  Callers must chunk — see [com.simplephotos.data.repository.SQLITE_VARIABLE_CHUNK]. */
    @Query("SELECT * FROM photos WHERE serverPhotoId IN (:photoIds)")
    suspend fun getByServerPhotoIds(photoIds: List<String>): List<PhotoEntity>

    /** Batch lookup: get all photos whose serverBlobId is in the given list.
     *  Callers must chunk — see [com.simplephotos.data.repository.SQLITE_VARIABLE_CHUNK]. */
    @Query("SELECT * FROM photos WHERE serverBlobId IN (:blobIds)")
    suspend fun getByServerBlobIds(blobIds: List<String>): List<PhotoEntity>

    /** Batch lookup: get all photos whose localId is in the given list. */
    @Query("SELECT * FROM photos WHERE localId IN (:ids)")
    suspend fun getByIds(ids: List<String>): List<PhotoEntity>

    /**
     * Get every frame belonging to one of the given burst groups. Used to
     * expand a collapsed burst representative back to its full stack when
     * adding to an album / secure album.
     */
    @Query("SELECT * FROM photos WHERE burstId IN (:burstIds)")
    suspend fun getByBurstIds(burstIds: List<String>): List<PhotoEntity>

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
