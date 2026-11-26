package com.simplephotos.data.media

/**
 * Decides which backed-up photos need a new date (TODO §4b P1c).
 *
 * 5,003 of 5,146 Android uploads were stored at 1970-01-01 because the scanner read
 * only `DATE_TAKEN`, NULL for a download. [LibraryDate] fixed new backups; this
 * finds the existing ones. Pure — the caller supplies the server's rows, Room's
 * links and the MediaStore columns — so every rule below is JVM-tested.
 *
 * Rules, in order, per server row:
 * 1. No Room row links it to a phone file ⇒ **not on this phone**. No guessing by
 *    name or size: a wrong match would put a wrong date on a right photo.
 * 2. Linked, but MediaStore no longer has the file ⇒ **file gone**.
 * 3. [LibraryDate.forScanOrNull] gives nothing, or a date past now + 1 day (the
 *    server refuses a future date and would reject the whole chunk) ⇒ **no usable date**.
 * 4. The server's `taken_at` is present but unparseable ⇒ **server date unreadable**
 *    (it cannot be sent as `expect_taken_at`; the server would 400 the chunk).
 * 5. The server already sorts it at that instant ⇒ **already right**.
 * 6. Otherwise a [Candidate], whose `expectTakenAt` is the server's EXACT string —
 *    the compare-and-set token. Comparing against Room instead would skip exactly
 *    the broken rows: a hash-merged local row keeps its scan date while the server
 *    still says 1970.
 */
object DateRepairPlanner {

    private const val FUTURE_SLACK_MS = 24L * 60 * 60 * 1000

    data class ServerRow(
        val id: String,
        val filename: String,
        val takenAt: String?,
        val createdAt: String,
    )

    /** MediaStore's view of one linked file. */
    data class PhoneFile(
        val relativePath: String?,
        val dateTakenMs: Long?,
        val dateAddedSecs: Long?,
        val dateModifiedSecs: Long?,
    )

    data class Candidate(
        val photoId: String,
        val filename: String,
        /** Server's current `taken_at`, verbatim; null when the column is NULL. */
        val expectTakenAt: String?,
        /** What the server sorts by today (null if unparseable). */
        val fromMs: Long?,
        val toMs: Long,
    )

    data class Plan(
        val candidates: List<Candidate>,
        val alreadyRight: Int,
        val notOnPhone: Int,
        val fileGone: Int,
        val noUsableDate: Int,
        val serverDateUnreadable: Int,
    ) {
        /** Candidates the server currently sorts at the epoch — the reported bug. */
        val atEpoch: Int get() = candidates.count { it.fromMs != null && it.fromMs <= 0L }
    }

    /**
     * @param linkedUri server photo id → the `localPath` (content URI) of the Room
     *   row linked to it.
     * @param files content URI → MediaStore columns; a URI absent here is gone.
     */
    fun plan(
        serverRows: List<ServerRow>,
        linkedUri: Map<String, String>,
        files: Map<String, PhoneFile>,
        nowMs: Long,
    ): Plan {
        val candidates = ArrayList<Candidate>()
        var alreadyRight = 0
        var notOnPhone = 0
        var fileGone = 0
        var noUsableDate = 0
        var serverDateUnreadable = 0
        val seen = HashSet<String>()

        for (row in serverRows) {
            // A full walk can repeat a row across a page boundary; the server
            // refuses a duplicate id, which would sink a whole chunk.
            if (!seen.add(row.id)) continue
            val uri = linkedUri[row.id]
            if (uri == null) { notOnPhone++; continue }
            val file = files[uri]
            if (file == null) { fileGone++; continue }

            val to = LibraryDate.forScanOrNull(
                file.relativePath, file.dateTakenMs, file.dateAddedSecs, file.dateModifiedSecs,
            )
            if (to == null || to <= 0L || to > nowMs + FUTURE_SLACK_MS) { noUsableDate++; continue }

            val expect = row.takenAt?.takeIf { it.isNotBlank() }
            if (expect != null && ServerDate.parseMs(expect) == null) { serverDateUnreadable++; continue }
            val from = ServerDate.ofRecord(row.takenAt, row.createdAt)
            if (from == to) { alreadyRight++; continue }

            candidates += Candidate(
                photoId = row.id,
                filename = row.filename,
                expectTakenAt = expect,
                fromMs = from,
                toMs = to,
            )
        }
        return Plan(candidates, alreadyRight, notOnPhone, fileGone, noUsableDate, serverDateUnreadable)
    }
}
