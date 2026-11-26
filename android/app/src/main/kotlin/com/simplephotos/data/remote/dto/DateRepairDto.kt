/**
 * DTOs for the batch photo-date repair (TODO §4b P1b/P1c; server `photos/date_repair.rs`):
 *   POST /api/photos/dates/repair       → [DateRepairResponse]
 *   GET  /api/photos/dates/needs-repair → [NeedsRepairResponse]
 *
 * The repair defaults to a DRY RUN on the server; only `dry_run = false` writes.
 * The phone only ever applies (automatic, TODO §4c); undo is server-side
 * (`POST /api/photos/dates/repair/undo`) and has no client.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.TypeAdapter
import com.google.gson.annotations.JsonAdapter
import com.google.gson.annotations.SerializedName
import com.google.gson.stream.JsonReader
import com.google.gson.stream.JsonWriter

/**
 * One correction. [expectTakenAt] is the compare-and-set token: the server's
 * current `taken_at`, verbatim, or null when that column is NULL.
 *
 * The server treats an ABSENT `expect_taken_at` as an error (no claim made) and
 * an explicit `null` as "the column is NULL". The app's Gson omits null fields,
 * so a plain data class would turn every NULL claim into a 400 for its whole
 * chunk — hence [Adapter], which always writes the field.
 */
@JsonAdapter(DateRepairItem.Adapter::class)
data class DateRepairItem(
    val photoId: String,
    val takenAt: String,
    val expectTakenAt: String?,
) {
    class Adapter : TypeAdapter<DateRepairItem>() {
        override fun write(out: JsonWriter, value: DateRepairItem?) {
            if (value == null) { out.nullValue(); return }
            out.beginObject()
            out.name("photo_id").value(value.photoId)
            out.name("taken_at").value(value.takenAt)
            val previous = out.serializeNulls
            out.serializeNulls = true
            try {
                out.name("expect_taken_at")
                if (value.expectTakenAt == null) out.nullValue() else out.value(value.expectTakenAt)
            } finally {
                out.serializeNulls = previous
            }
            out.endObject()
        }

        override fun read(reader: JsonReader): DateRepairItem =
            throw UnsupportedOperationException("DateRepairItem is request-only")
    }
}

data class DateRepairRequest(
    @SerializedName("dry_run") val dryRun: Boolean,
    val items: List<DateRepairItem>,
)

data class DateRepairCounts(
    val change: Int = 0,
    val unchanged: Int = 0,
    val stale: Int = 0,
    @SerializedName("not_found") val notFound: Int = 0,
    val protected: Int = 0,
)

data class DateRepairItemResult(
    @SerializedName("photo_id") val photoId: String,
    /** change | unchanged | stale | not_found | protected */
    val outcome: String,
    @SerializedName("current_taken_at") val currentTakenAt: String? = null,
    @SerializedName("new_taken_at") val newTakenAt: String,
)

data class DateRepairResponse(
    @SerializedName("dry_run") val dryRun: Boolean,
    @SerializedName("batch_id") val batchId: String? = null,
    val counts: DateRepairCounts,
    val items: List<DateRepairItemResult>,
)


/** `GET /api/photos/dates/needs-repair` — the caller's client rows at 1970, secure included. */
data class NeedsRepairRow(
    val id: String,
    val filename: String,
    /** Verbatim; sent back as `expect_taken_at`. */
    @SerializedName("taken_at") val takenAt: String,
    @SerializedName("created_at") val createdAt: String,
)

data class NeedsRepairResponse(
    val photos: List<NeedsRepairRow>,
    val truncated: Boolean = false,
)
