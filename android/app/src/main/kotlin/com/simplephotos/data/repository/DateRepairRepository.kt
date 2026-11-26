package com.simplephotos.data.repository

import android.content.Context
import android.net.Uri
import android.provider.MediaStore
import android.util.Log
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.media.DateRepairPlanner
import com.simplephotos.data.media.LibraryDate
import com.simplephotos.data.media.ServerDate
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.DateRepairCounts
import com.simplephotos.data.remote.dto.DateRepairItem
import com.simplephotos.data.remote.dto.DateRepairRequest
import com.simplephotos.data.remote.dto.DateRepairUndoRequest
import com.simplephotos.data.remote.isHttpStatus
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import java.time.Instant
import javax.inject.Inject
import javax.inject.Singleton

/**
 * "Fix photo dates" (TODO §4b P1c): finds backed-up photos whose server date
 * breaks the [LibraryDate] rule and repairs them through
 * `POST /api/photos/dates/repair` — dry run first, apply on the owner's OK, undo
 * of the last apply. The decisions live in [DateRepairPlanner]; this class only
 * gathers its inputs and talks to the server.
 *
 * Every server call here passes `dry_run` explicitly. The server re-checks each
 * row inside its own write transaction (compare-and-set on `expect_taken_at`),
 * so a preview that went stale between Check and Apply is skipped, not forced.
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
        private const val SAMPLE = 20
        /** Comma-joined batch ids of the last apply — what "Undo last fix" reverses. */
        private val KEY_LAST_BATCHES = stringPreferencesKey("date_repair_last_batches")
    }

    data class Sample(val filename: String, val fromMs: Long?, val toMs: Long)

    data class Preview(
        val plan: DateRepairPlanner.Plan,
        /** Server's verdict on the candidates, summed over chunks. */
        val server: DateRepairCounts,
        /** Candidates the server says WILL change — what Apply sends. */
        val toApply: List<DateRepairPlanner.Candidate>,
        val samples: List<Sample>,
    )

    data class ApplyResult(val counts: DateRepairCounts, val batches: Int)

    data class UndoPreview(val counts: DateRepairCounts, val batches: Int)

    /** Read-only: walks the server, Room and MediaStore; asks the server to dry-run. */
    suspend fun preview(onProgress: (String) -> Unit): Preview = withContext(Dispatchers.IO) {
        onProgress("Reading the library from the server…")
        val serverRows = walkServer()
        onProgress("Checking ${serverRows.size} photos against this phone…")
        val links = db.photoDao().getServerFileLinks()
        val linkedUri = HashMap<String, String>(links.size)
        for (l in links) linkedUri.putIfAbsent(l.serverPhotoId, l.localPath)
        val files = readPhoneFiles(linkedUri.values.toSet())
        val plan = DateRepairPlanner.plan(serverRows, linkedUri, files, System.currentTimeMillis())
        Log.i(TAG, "preview: server=${serverRows.size} linked=${linkedUri.size} files=${files.size} " +
            "candidates=${plan.candidates.size} atEpoch=${plan.atEpoch} alreadyRight=${plan.alreadyRight} " +
            "notOnPhone=${plan.notOnPhone} gone=${plan.fileGone} noDate=${plan.noUsableDate} " +
            "serverUnreadable=${plan.serverDateUnreadable}")

        var counts = DateRepairCounts()
        val willChange = HashSet<String>()
        plan.candidates.chunked(CHUNK).forEachIndexed { i, chunk ->
            onProgress("Asking the server about ${plan.candidates.size} photos (${i * CHUNK + chunk.size})…")
            val resp = api.repairDates(DateRepairRequest(dryRun = true, items = chunk.map(::toItem)))
            check(resp.dryRun) { "server answered a dry run with dry_run=false" }
            counts = counts + resp.counts
            resp.items.filter { it.outcome == "change" }.mapTo(willChange) { it.photoId }
        }
        val toApply = plan.candidates.filter { it.photoId in willChange }
        Log.i(TAG, "preview: server dry run $counts")
        Preview(
            plan = plan,
            server = counts,
            toApply = toApply,
            samples = toApply.take(SAMPLE).map { Sample(it.filename, it.fromMs, it.toMs) },
        )
    }

    /** Writes. Sends exactly the candidates the preview showed as changing. */
    suspend fun apply(preview: Preview, onProgress: (String) -> Unit): ApplyResult = withContext(Dispatchers.IO) {
        require(preview.toApply.isNotEmpty()) { "nothing to apply" }
        val byId = preview.toApply.associateBy { it.photoId }
        val batches = ArrayList<String>()
        var counts = DateRepairCounts()
        Log.i(TAG, "apply: start ${preview.toApply.size} photos")
        try {
            preview.toApply.chunked(CHUNK).forEachIndexed { i, chunk ->
                onProgress("Fixing ${i * CHUNK + chunk.size} of ${preview.toApply.size}…")
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
            }
        } finally {
            // Saved even after a mid-run failure, so the chunks that DID apply
            // stay undoable. An apply with no batch keeps the previous undo.
            if (batches.isNotEmpty()) {
                dataStore.edit { it[KEY_LAST_BATCHES] = batches.joinToString(",") }
                Log.i(TAG, "apply: saved ${batches.size} batch ids for undo: $batches")
            }
        }
        Log.i(TAG, "apply: done $counts in ${batches.size} batches")
        ApplyResult(counts, batches.size)
    }

    suspend fun hasUndo(): Boolean = lastBatches().isNotEmpty()

    /** Read-only: what undoing the last apply would restore. */
    suspend fun previewUndo(): UndoPreview = withContext(Dispatchers.IO) {
        val batches = lastBatches()
        require(batches.isNotEmpty()) { "no fix to undo" }
        var counts = DateRepairCounts()
        var live = 0
        for (b in batches) {
            try {
                val resp = api.undoDateRepair(DateRepairUndoRequest(dryRun = true, batchId = b))
                counts = counts + resp.counts
                live++
            } catch (e: Exception) {
                if (!isHttpStatus(e, 404)) throw e
                Log.w(TAG, "previewUndo: batch $b already undone or unknown (404)")
            }
        }
        UndoPreview(counts, live)
    }

    suspend fun undoLast(onProgress: (String) -> Unit): DateRepairCounts = withContext(Dispatchers.IO) {
        val batches = lastBatches()
        require(batches.isNotEmpty()) { "no fix to undo" }
        var counts = DateRepairCounts()
        Log.i(TAG, "undo: start ${batches.size} batches")
        batches.forEachIndexed { i, b ->
            onProgress("Undoing part ${i + 1} of ${batches.size}…")
            try {
                val resp = api.undoDateRepair(DateRepairUndoRequest(dryRun = false, batchId = b))
                counts = counts + resp.counts
                for (r in resp.items) {
                    if (r.outcome != "change") continue
                    // A NULL original has no instant; the next sync adopts the
                    // server's created_at for it (ServerDate.ofRecord).
                    val ms = ServerDate.parseMs(r.restoreTakenAt) ?: continue
                    db.photoDao().updateTakenAtByServerId(r.photoId, ms)
                }
            } catch (e: Exception) {
                if (!isHttpStatus(e, 404)) throw e
                Log.w(TAG, "undo: batch $b already undone or unknown (404)")
            }
        }
        dataStore.edit { it.remove(KEY_LAST_BATCHES) }
        Log.i(TAG, "undo: done $counts")
        counts
    }

    // ── inputs ───────────────────────────────────────────────────────────

    private suspend fun lastBatches(): List<String> =
        dataStore.data.first()[KEY_LAST_BATCHES]
            ?.split(',')
            ?.map { it.trim() }
            ?.filter { it.isNotEmpty() }
            ?: emptyList()

    /** Full metadata walk (no `since`): the server's current date for every photo. */
    private suspend fun walkServer(): List<DateRepairPlanner.ServerRow> {
        val rows = ArrayList<DateRepairPlanner.ServerRow>()
        var after: String? = null
        repeat(10_000) {
            val page = api.encryptedSync(after = after, limit = CHUNK)
            page.photos.mapTo(rows) { DateRepairPlanner.ServerRow(it.id, it.filename, it.takenAt, it.createdAt) }
            after = page.nextCursor ?: return rows
        }
        error("encrypted-sync walk did not terminate")
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
