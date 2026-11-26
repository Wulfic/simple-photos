package com.simplephotos.data.media

/**
 * Rules for the automatic photo-date repair (TODO §4c P2). The owner ran the
 * manual Check/Apply once (4,900 photos moved) and asked for it to happen on its
 * own from then on; this is what keeps an unattended run safe. Pure, JVM-tested.
 */
object AutoDateRepair {

    /** At most one run per this interval, piggybacked on a backup run. */
    const val INTERVAL_MS: Long = 6L * 60 * 60 * 1000

    /** Anything inside 1970-01-01 (UTC) is the bug value. */
    private const val EPOCH_DAY_MS: Long = 24L * 60 * 60 * 1000

    /**
     * Due when it never ran, the interval has passed, or the clock went backwards
     * (a stuck future timestamp must not disable the repair for good).
     */
    fun isDue(lastRunMs: Long?, nowMs: Long): Boolean =
        lastRunMs == null || nowMs < lastRunMs || nowMs - lastRunMs >= INTERVAL_MS

    /**
     * Keep only candidates that would replace a 1970 value. The server's
     * compare-and-set already guarantees a row changes only if it still holds
     * `expectTakenAt`; this guarantees that value is the bug — so an automatic
     * run can never overwrite a date someone set by hand.
     *
     * @return (kept, dropped)
     */
    fun epochOnly(
        candidates: List<DateRepairPlanner.Candidate>,
    ): Pair<List<DateRepairPlanner.Candidate>, Int> {
        val kept = candidates.filter { c ->
            val ms = ServerDate.parseMs(c.expectTakenAt) ?: return@filter false
            ms in 0 until EPOCH_DAY_MS
        }
        return kept to (candidates.size - kept.size)
    }
}
