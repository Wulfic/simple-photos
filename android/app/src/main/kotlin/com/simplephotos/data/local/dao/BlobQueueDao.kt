package com.simplephotos.data.local.dao

import androidx.room.*
import com.simplephotos.data.local.entities.BlobQueueEntity

@Dao
interface BlobQueueDao {
    @Query("SELECT * FROM blob_queue WHERE status = 'queued' ORDER BY priority ASC, id ASC")
    suspend fun getQueued(): List<BlobQueueEntity>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(entry: BlobQueueEntity)

    @Update
    suspend fun update(entry: BlobQueueEntity)

    @Query("UPDATE blob_queue SET status = :status WHERE id = :id")
    suspend fun updateStatus(id: String, status: String)

    @Query("UPDATE blob_queue SET attempts = attempts + 1, lastAttemptAt = :time WHERE id = :id")
    suspend fun incrementAttempts(id: String, time: Long)

    @Query("DELETE FROM blob_queue WHERE id = :id")
    suspend fun deleteById(id: String)

    @Query("UPDATE blob_queue SET status = 'failed' WHERE attempts >= 5 AND status != 'done'")
    suspend fun markExhaustedAsFailed()

    @Query("DELETE FROM blob_queue")
    suspend fun deleteAll()
}
