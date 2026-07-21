/**
 * DAO for the mirror's own bookkeeping (the #38 delta-sync cursor).
 */
package com.simplephotos.data.local.dao

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import com.simplephotos.data.local.entities.SyncStateEntity

@Dao
interface SyncStateDao {
    @Query("SELECT * FROM sync_state WHERE `key` = :key")
    suspend fun get(key: String): SyncStateEntity?

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun put(row: SyncStateEntity)

    @Query("DELETE FROM sync_state WHERE `key` = :key")
    suspend fun delete(key: String)

    /** Wiped alongside `photos` on logout — see [SyncStateEntity]. */
    @Query("DELETE FROM sync_state")
    suspend fun deleteAll()
}
