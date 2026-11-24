/**
 * Single-flight rules for the backup pipeline (TODO.md D1).
 *
 * Kept free of WorkManager / Room / `android.util.Log` so the part that can be
 * wrong is JVM-unit-testable, the same move `SyncPlan.kt` made.
 */
package com.simplephotos.sync

import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Lets at most one backup pass run in this process at a time.
 *
 * ## Why WorkManager's unique work is not enough
 *
 * The periodic, reactive (content-URI) and "back up now" requests are enqueued
 * under three **different** unique names, so WorkManager happily runs them side
 * by side. On 2026-11-17 the phone's own logs show three passes overlapping for
 * an hour on the same pending set: each took its own snapshot of PENDING rows, so
 * a row one pass had just uploaded was still PENDING in another's snapshot and was
 * re-registered (5,327 such registrations), and a pass starting mid-run reset the
 * other's UPLOADING rows back to PENDING. Merging the three names would change
 * their scheduling semantics (KEEP vs REPLACE, expedited vs content trigger); a
 * process-wide gate fixes the overlap without touching any of that.
 *
 * A refused caller must not wait: a waiting worker still burns its execution
 * window, and the pass that holds the gate already scans for everything pending.
 */
class SingleFlight {
    private val running = AtomicBoolean(false)

    /**
     * Run [block] unless another caller is inside. Returns null when refused, so
     * the caller can do its "skipped" bookkeeping. The gate is released however
     * [block] ends — normally, by exception, or by coroutine cancellation.
     */
    suspend fun <T> runExclusive(block: suspend () -> T): T? {
        if (!running.compareAndSet(false, true)) return null
        try {
            return block()
        } finally {
            running.set(false)
        }
    }

    /** True while a caller is inside [runExclusive]. For logging only. */
    val isRunning: Boolean get() = running.get()
}

/**
 * Whether [current] — the row as it is in Room **now**, re-read just before its
 * upload — still needs uploading.
 *
 * The pass works from a snapshot taken at its start, and a long batch (an hour on
 * a reinstall) outlives that snapshot: the gallery's manual import uploads its own
 * rows directly, and a hash-dedup earlier in the batch may already have settled a
 * row. Uploading from the stale copy is how a row gets registered twice. Null
 * means the row is gone (purged or collapsed into a twin) — nothing to upload.
 */
fun stillNeedsUpload(current: PhotoEntity?): Boolean {
    if (current == null) return false
    if (current.serverPhotoId != null || current.serverBlobId != null) return false
    return current.syncStatus == SyncStatus.PENDING || current.syncStatus == SyncStatus.FAILED
}
