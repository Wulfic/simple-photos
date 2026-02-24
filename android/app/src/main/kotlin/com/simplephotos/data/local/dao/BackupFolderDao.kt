package com.simplephotos.data.local.dao

import androidx.room.*
import com.simplephotos.data.local.entities.BackupFolderEntity
import kotlinx.coroutines.flow.Flow

@Dao
interface BackupFolderDao {
    @Query("SELECT * FROM backup_folders ORDER BY bucketName")
    fun getAllFolders(): Flow<List<BackupFolderEntity>>

    @Query("SELECT * FROM backup_folders WHERE enabled = 1")
    suspend fun getEnabledFolders(): List<BackupFolderEntity>

    @Query("SELECT COUNT(*) FROM backup_folders")
    suspend fun count(): Int

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(folder: BackupFolderEntity)

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertAll(folders: List<BackupFolderEntity>)

    @Update
    suspend fun update(folder: BackupFolderEntity)

    @Query("UPDATE backup_folders SET enabled = :enabled WHERE bucketId = :bucketId")
    suspend fun setEnabled(bucketId: Long, enabled: Boolean)

    @Delete
    suspend fun delete(folder: BackupFolderEntity)

    @Query("DELETE FROM backup_folders WHERE bucketId = :bucketId")
    suspend fun deleteByBucketId(bucketId: Long)

    @Query("SELECT bucketId FROM backup_folders WHERE enabled = 1")
    suspend fun getEnabledBucketIds(): List<Long>
}
