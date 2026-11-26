package com.simplephotos.data.repository

import android.content.Context
import android.net.Uri
import android.provider.MediaStore
import android.util.Log
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.longPreferencesKey
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.media.AutoDateRepair
import com.simplephotos.data.media.DateRepairPlanner
import com.simplephotos.data.media.LibraryDate
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.DateRepairCounts
import com.simplephotos.data.remote.dto.DateRepairItem
import com.simplephotos.data.remote.dto.DateRepairRequest
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import java.time.Instant
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Automatic photo-date repair (TODO §4b P1c → §4c P2). Backed-up photos the old
 * scanner stored at 1970-01-01 are moved to the day they landed on this phone
 * ([LibraryDate]), without a button: the owner ran the manual Check/Apply once
 * (4,900 photos moved) and asked for it to be automatic from then on.
 *
 * Runs from the backup worker, at most every [AutoDateRepair.INTERVAL_MS]:
 * 1. `GET /api/photos/dates/needs-repair` — this user's client rows at 1970,
 *    **secure photos included** (the first repair walked `encrypted-sync`, which
 *    hides them, and missed 104). Usually empty: one request and done.
 * 2. Keep the rows Room links to a file on this phone; read their MediaStore dates.
 * 3. [DateRepairPlanner] decides; [AutoDateRepair.epochOnly] keeps only changes
 *    that replace a 1970 value.
 * 4. Apply in chunks of [CHUNK]. The server re-checks every row inside its write
 *    transaction (compare-and-set on `expect_taken_at`), refuses server-file rows
 *    (`protected`) and logs each batch with its old values; reverse one with
 *    `POST /api/photos/dates/repair/undo {batch_id, dry_run:false}` — the ids are
 *    in the returned [Result] (logged to client logs) and the server log.
 */
@Singleton
class DateRepairRepository @Inject constructor(
    @ApplicationContext private val context: Context,
    private val api: ApiService,
    private val db: AppDatabase,
    private val dataStore: DataStore<Preferences>,
) {
    companion object {
        private const val TAG = "DateRepair"
        /** The server's per-call limit. */
        const val CHUNK = 500
        private val KEY_LAST_AUTO_RUN = longPreferencesKey("date_repair_last_auto_run")
    }

    data class Result(
        val listed: Int,
        val plan: DateRepairPlanner.Plan,
        val droppedNotEpoch: Int,
        val counts: DateRepairCounts,
        val batchIds: List<String>,
        val truncated: Boolean,
    ) {
        fun toLogMap(): Map<String, String> = mapOf(
            "listed" to listed.toString(),
            "sent" to (plan.candidates.size - droppedNotEpoch).toString(),
            "changed" to counts.change.toString(),
            "unchanged" to counts.unchanged.toString(),
            "stale" to counts.stale.toString(),
            "protected" to counts.protected.toString(),
            "notFound" to counts.notFound.toString(),
            "notOnPhone" to plan.notOnPhone.toString(),
            "fileGone" to plan.fileGone.toString(),
            "noUsableDate" to plan.noUsableDate.toString(),
            "droppedNotEpoch" to droppedNotEpoch.toString(),
            "truncated" to truncated.toString(),
            "batchIds" to batchIds.joinToString(","),
        )
    }

    /** Null when not due. Throws on a server/network failure (the caller logs it). */
    suspend fun autoRepairIfDue(): Result? = withContext(Dispatchers.IO) {
        val now = System.currentTimeMillis()
        val last = dataStore.data.first()[KEY_LAST_AUTO_RUN]
        if (!AutoDateRepair.isDue(last, now)) return@withContext null

        val listed = api.datesNeedingRepair()
        val rows = listed.photos.map { DateRepairPlanner.ServerRow(it.id, it.filename, it.takenAt, it.createdAt) }

        val wanted = rows.mapTo(HashSet()) { it.id }
        val linkedUri = HashMap<String, String>()
        if (wanted.isNotEmpty()) {
            for (l in db.photoDao().getServerFileLinks()) {
                if (l.serverPhotoId in wanted) linkedUri.putIfAbsent(l.serverPhotoId, l.localPath)
            }
        }
        val files = readPhoneFiles(linkedUri.values.toSet())
        val plan = DateRepairPlanner.plan(rows, linkedUri, files, now)
        val (toSend, dropped) = AutoDateRepair.epochOnly(plan.candidates)
        if (dropped > 0) Log.w(TAG, "auto: dropped $dropped candidates whose server date is not 1970")
        val byId = toSend.associateBy { it.photoId }

        var counts = DateRepairCounts()
        val batches = ArrayList<String>()
        for (chunk in toSend.chunked(CHUNK)) {
            val resp = api.repairDates(DateRepairRequest(dryRun = false, items = chunk.map(::toItem)))
            check(!resp.dryRun) { "server answered an apply as a dry run" }
            resp.batchId?.let { batches += it }
            counts = counts + resp.counts
            // Room follows only what the server confirmed it changed.
            for (r in resp.items) {
                if (r.outcome != "change") continue
                val ms = byId[r.photoId]?.toMs ?: continue
                db.photoDao().updateTakenAtByServerId(r.photoId, ms)
            }
            Log.i(TAG, "auto: applied chunk of ${chunk.size}: ${resp.counts} batch=${resp.batchId}")
        }

        // A truncated listing means more rows are waiting: leave the stamp so
        // the next backup run continues instead of waiting the full interval.
        if (!listed.truncated) dataStore.edit { it[KEY_LAST_AUTO_RUN] = now }

        Result(rows.size, plan, dropped, counts, batches, listed.truncated).also {
            Log.i(TAG, "auto: done ${it.toLogMap()}")
        }
    }

    /** MediaStore columns for each linked URI. A URI that resolves to nothing is absent. */
    private fun readPhoneFiles(uris: Set<String>): Map<String, DateRepairPlanner.PhoneFile> {
        val pathCol = SyncRepository.PATH_COLUMN
        // "datetaken" is the same column name in Images and Video; the per-type
        // constant is used because the MediaColumns one is API 29.
        val projection = arrayOf(
            MediaStore.Images.Media.DATE_TAKEN,
            MediaStore.MediaColumns.DATE_ADDED,
            MediaStore.MediaColumns.DATE_MODIFIED,
            pathCol,
        )
        val out = HashMap<String, DateRepairPlanner.PhoneFile>(uris.size)
        var failed = 0
        for (uri in uris) {
            try {
                context.contentResolver.query(Uri.parse(uri), projection, null, null, null)?.use { c ->
                    if (!c.moveToFirst()) return@use
                    fun long(i: Int): Long? = if (c.isNull(i)) null else c.getLong(i)
                    val raw = if (c.isNull(3)) null else c.getString(3)
                    val rel = if (pathCol == MediaStore.MediaColumns.DATA) LibraryDate.relativeFromAbsolute(raw) else raw
                    out[uri] = DateRepairPlanner.PhoneFile(rel, long(0), long(1), long(2))
                }
            } catch (e: Exception) {
                // A revoked or deleted item throws on some OEMs instead of
                // returning an empty cursor — either way it is "gone".
                failed++
                if (failed <= 5) Log.w(TAG, "readPhoneFiles: query failed for $uri", e)
            }
        }
        if (failed > 0) Log.w(TAG, "readPhoneFiles: $failed of ${uris.size} URIs could not be read (counted as gone)")
        return out
    }

    private fun toItem(c: DateRepairPlanner.Candidate) = DateRepairItem(
        photoId = c.photoId,
        takenAt = Instant.ofEpochMilli(c.toMs).toString(),
        expectTakenAt = c.expectTakenAt,
    )

    private operator fun DateRepairCounts.plus(o: DateRepairCounts) = DateRepairCounts(
        change = change + o.change,
        unchanged = unchanged + o.unchanged,
        stale = stale + o.stale,
        notFound = notFound + o.notFound,
        protected = protected + o.protected,
    )
}
