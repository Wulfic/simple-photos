package com.simplephotos.data.media

/**
 * The date a backed-up item sorts by in the library (P1, owner rule 2026-11-26):
 * **camera photos keep their capture date; everything else uses the date it landed
 * on the phone.**
 *
 * Why this exists: `MediaStore.DATE_TAKEN` is NULL for a downloaded file, and the
 * scanner used to store `Cursor.getLong()` of it — 0 — so 5,003 of 5,146 Android
 * uploads sat at 1970-01-01 at the bottom of the grid. The phone encrypts before
 * upload, so the server never sees EXIF: the value computed here is final.
 *
 * This is the ONE place the rule lives. The backup scanner and the P1c date repair
 * must both call it — `PhotoDao.getLocalByFilenameAndDate` merges server rows with
 * local rows on `filename + takenAt`, so two diverging rules would stop the merge
 * from matching and duplicate the tile.
 */
object LibraryDate {

    /**
     * Anything before Android shipped cannot be the day a file landed on an Android
     * phone. Guards against file mtimes carried over from archives (FAT zips stamp
     * 1980-01-01) that would otherwise win the "earlier of" comparison.
     */
    private const val ANDROID_ERA_START_MS = 1_199_145_600_000L // 2008-01-01T00:00:00Z

    /**
     * @param relativePath MediaStore `RELATIVE_PATH` (API 29+), or
     *   [relativeFromAbsolute] of `DATA` below that.
     * @param dateTakenMs `DATE_TAKEN` in ms, null when the column is NULL.
     * @param dateAddedSecs `DATE_ADDED` in seconds (MediaStore unit).
     * @param dateModifiedSecs `DATE_MODIFIED` in seconds (file mtime).
     * @return epoch ms; never zero or negative.
     */
    fun forScan(
        relativePath: String?,
        dateTakenMs: Long?,
        dateAddedSecs: Long?,
        dateModifiedSecs: Long?,
        nowMs: Long,
    ): Long {
        val taken = dateTakenMs?.takeIf { it > 0 }
        if (taken != null && isCameraTree(relativePath)) return taken

        // A fresh download has added ≈ modified ≈ the download. A phone migration
        // resets DATE_ADDED but keeps the mtime; an edit bumps the mtime but not
        // DATE_ADDED. The earlier of the two is the landed date in all three.
        val landed = listOfNotNull(dateAddedSecs, dateModifiedSecs)
            .map { it * 1000 }
            .filter { it >= ANDROID_ERA_START_MS }
            .minOrNull()
        return landed ?: taken ?: nowMs
    }

    /** True when the first path segment is `DCIM` (case-insensitive). */
    fun isCameraTree(relativePath: String?): Boolean {
        val first = relativePath
            ?.trimStart('/')
            ?.substringBefore('/')
            ?: return false
        return first.equals("DCIM", ignoreCase = true)
    }

    /**
     * Below API 29 there is no `RELATIVE_PATH`, only the absolute `DATA` path whose
     * volume prefix varies (`/storage/emulated/0/`, `/storage/1234-ABCD/`). Returns
     * the path from the `DCIM` segment on when there is one, else the input as-is
     * (which [isCameraTree] then rejects).
     */
    fun relativeFromAbsolute(absolutePath: String?): String? {
        if (absolutePath == null) return null
        val idx = absolutePath.indexOf("/DCIM/", ignoreCase = true)
        return if (idx >= 0) absolutePath.substring(idx + 1) else absolutePath
    }
}
