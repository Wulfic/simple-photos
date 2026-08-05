/**
 * Bookkeeping the local mirror needs about itself — currently just the #38
 * delta-sync cursor.
 */
package com.simplephotos.data.local.entities

import androidx.room.Entity
import androidx.room.PrimaryKey

/**
 * A single keyed bookkeeping value, stored **in the same Room database as
 * `photos`**.
 *
 * The location is the point. A cursor asserts "the mirror holds every change up
 * to `seq`", so it must not be able to outlive the mirror it describes. Room's
 * `fallbackToDestructiveMigration` drops every table together, and this row goes
 * with them — where a `SharedPreferences` cursor would survive a database wipe
 * and then claim currency over an empty gallery, which is silent, permanent, and
 * unrecoverable by re-syncing.
 *
 * Co-location is necessary but **not sufficient**: `AuthRepository.logout()`
 * clears tables one DAO at a time rather than dropping the file, so this table
 * has to be cleared there explicitly, and `SyncCursorStore.read` still refuses a
 * cursor over an empty mirror. See `web/src/gallery/hooks/syncCursor.ts` — the
 * same rule, learnt the same way.
 */
@Entity(tableName = "sync_state")
data class SyncStateEntity(
    @PrimaryKey val key: String,
    /** The change-log sequence the mirror is current as of. */
    val seq: Long,
    val updatedAt: Long = System.currentTimeMillis(),
)
