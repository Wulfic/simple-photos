/**
 * DTOs for the batch photo-date repair (TODO §4b P1b/P1c; server `photos/date_repair.rs`):
 *   POST /api/photos/dates/repair       → [DateRepairResponse]
 *   POST /api/photos/dates/repair/undo  → [DateRepairUndoResponse]
 *
 * Both default to a DRY RUN on the server; only `dry_run = false` writes.
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

data class DateRepairUndoRequest(
    @SerializedName("dry_run") val dryRun: Boolean,
    @SerializedName("batch_id") val batchId: String,
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

data class DateRepairUndoItemResult(
    @SerializedName("photo_id") val photoId: String,
    val outcome: String,
    @SerializedName("current_taken_at") val currentTakenAt: String? = null,
    @SerializedName("restore_taken_at") val restoreTakenAt: String? = null,
)

data class DateRepairUndoResponse(
    @SerializedName("dry_run") val dryRun: Boolean,
    @SerializedName("batch_id") val batchId: String,
    val counts: DateRepairCounts,
    val items: List<DateRepairUndoItemResult>,
)
