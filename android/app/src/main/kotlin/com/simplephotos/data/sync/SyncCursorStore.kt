/**
 * Reading and writing the #38 delta-sync cursor.
 *
 * Every read of a cursor is a safety decision, not a lookup, which is why this
 * is its own type rather than two calls into [SyncStateDao] scattered through
 * `PhotoRepository`. See [com.simplephotos.data.sync.SyncPlan] for why a false
 * cursor does not degrade gracefully.
 */
package com.simplephotos.data.sync

import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.SyncStateEntity

/** Key of the single bookkeeping row holding the delta cursor. */
const val SYNC_CURSOR_KEY = "photoDeltaSeq"

private const val TAG = "SyncCursorStore"

class SyncCursorStore(private val db: AppDatabase) {

    /**
     * The sequence the mirror is current as of, or null for "unknown — take the
     * full walk".
     *
     * Returns null rather than throwing on any inconsistency. Every null costs
     * one full walk; every wrongly-trusted number costs rows that never come
     * back.
     */
    suspend fun read(): Long? {
        return try {
            val row = db.syncStateDao().get(SYNC_CURSOR_KEY) ?: return null
            if (row.seq < 0) return null

            // Coherence guard. A cursor claims the mirror holds everything up to
            // `seq`; an empty mirror cannot satisfy that unless the library is
            // itself empty, in which case a full walk is free anyway.
            //
            // This catches what co-location cannot: a partial wipe that empties
            // `photos` while `sync_state` survives. `countAll` counts in SQLite
            // rather than materialising 14k rows, so it stays cheap.
            if (row.seq > 0 && db.photoDao().countAll() == 0) {
                android.util.Log.w(
                    TAG,
                    "delta cursor present (seq=${row.seq}) but the photo mirror is empty — " +
                        "discarding the cursor and falling back to a full walk",
                )
                clear()
                return null
            }
            row.seq
        } catch (e: Exception) {
            // A cursor we cannot read is a cursor we must not trust.
            android.util.Log.w(TAG, "failed to read the delta cursor; forcing a full walk", e)
            null
        }
    }

    /**
     * Advance the cursor after a pass has successfully applied everything it
     * received.
     *
     * Call this **only** once the mirror actually reflects [seq]. Writing it
     * early — before the inserts commit, say — converts a transient failure into
     * a permanent gap.
     */
    suspend fun write(seq: Long) {
        if (seq < 0) {
            android.util.Log.w(TAG, "refusing to persist a negative delta cursor: $seq")
            return
        }
        try {
            db.syncStateDao().put(SyncStateEntity(key = SYNC_CURSOR_KEY, seq = seq))
        } catch (e: Exception) {
            // Non-fatal: the mirror is correct, we just lost the shortcut. The
            // next pass reverts to a full walk, which is the pre-#38 behaviour.
            android.util.Log.w(TAG, "failed to persist the delta cursor", e)
        }
    }

    /** Drop the cursor, forcing the next pass through the self-healing walk. */
    suspend fun clear() {
        try {
            db.syncStateDao().delete(SYNC_CURSOR_KEY)
        } catch (e: Exception) {
            android.util.Log.w(TAG, "failed to clear the delta cursor", e)
        }
    }
}
