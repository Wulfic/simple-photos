/**
 * Album sort (#52) — the pure Date/Name ordering logic behind the album-detail
 * sort control, ported from web's `gallery/albumSort.ts`.
 *
 * Kept free of Android and Room types (it sorts via key selectors, not
 * `PhotoEntity`) so the comparator, the toggle logic and the persistence
 * serialization are all JVM-unit-testable — the same split `RenditionChoice.kt`
 * and `ClusterRename.kt` use. The Android glue (reading DataStore, driving the
 * grid) lives in `AlbumDetailViewModel`; the direction the sort is *applied* in
 * matches web exactly.
 */
package com.simplephotos.data.album

enum class AlbumSortField { DATE, NAME }
enum class AlbumSortDir { ASC, DESC }

data class AlbumSort(val field: AlbumSortField, val dir: AlbumSortDir)

/** The historical order before #52: capture date, newest first. Used as the
 *  control's visual default when the user has not chosen a sort. */
val DEFAULT_ALBUM_SORT = AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC)

/** The direction a field starts in when first switched to: dates newest-first,
 *  names A→Z — what a first tap on each is expected to do. */
fun defaultDirFor(field: AlbumSortField): AlbumSortDir =
    if (field == AlbumSortField.NAME) AlbumSortDir.ASC else AlbumSortDir.DESC

/**
 * Result of tapping a field button: toggle its direction if it is already the
 * active field, otherwise switch to it in its natural starting direction.
 * Mirrors web's `useAlbumSort.selectField`.
 */
fun nextSort(current: AlbumSort, field: AlbumSortField): AlbumSort =
    if (current.field == field) {
        current.copy(
            dir = if (current.dir == AlbumSortDir.ASC) AlbumSortDir.DESC else AlbumSortDir.ASC
        )
    } else {
        AlbumSort(field, defaultDirFor(field))
    }

/**
 * Total order over album items, tie-broken on a stable id so equal keys never
 * reorder between renders. `takenAt`/`name`/`id` are selectors so this needs no
 * Android type and can be unit-tested with plain data.
 *
 * A missing capture time (0) sorts as the oldest — last under desc, first under
 * asc — rather than landing somewhere arbitrary.
 */
fun <T> sortAlbumItems(
    items: List<T>,
    sort: AlbumSort,
    takenAt: (T) -> Long,
    name: (T) -> String,
    id: (T) -> String,
): List<T> {
    val comparator = Comparator<T> { a, b ->
        var c =
            if (sort.field == AlbumSortField.NAME) naturalCompare(name(a), name(b))
            else takenAt(a).compareTo(takenAt(b))
        if (c == 0) c = id(a).compareTo(id(b))
        if (sort.dir == AlbumSortDir.ASC) c else -c
    }
    return items.sortedWith(comparator)
}

/**
 * Natural, case-insensitive, numeric-aware string comparison so `IMG_2` precedes
 * `IMG_10` instead of sorting lexically. Mirrors web's
 * `Intl.Collator(numeric: true, sensitivity: "base")`: digit runs compare as
 * numbers (leading zeros ignored), everything else compares case-folded.
 */
internal fun naturalCompare(a: String, b: String): Int {
    val x = a.lowercase()
    val y = b.lowercase()
    var i = 0
    var j = 0
    while (i < x.length && j < y.length) {
        val cx = x[i]
        val cy = y[j]
        if (cx.isDigit() && cy.isDigit()) {
            var si = i
            while (si < x.length && x[si].isDigit()) si++
            var sj = j
            while (sj < y.length && y[sj].isDigit()) sj++
            // Compare numerically by stripping leading zeros, then by length,
            // then lexically — avoids overflow on very long digit runs.
            val numX = x.substring(i, si).trimStart('0')
            val numY = y.substring(j, sj).trimStart('0')
            if (numX.length != numY.length) return numX.length - numY.length
            val c = numX.compareTo(numY)
            if (c != 0) return c
            i = si
            j = sj
        } else {
            if (cx != cy) return cx.compareTo(cy)
            i++
            j++
        }
    }
    return (x.length - i) - (y.length - j)
}

// ── Persistence serialization (DataStore stores the string) ─────────────────

/** `"date:desc"` / `"name:asc"`. */
fun AlbumSort.serialize(): String =
    "${field.name.lowercase()}:${dir.name.lowercase()}"

/** Parse a stored value; a missing or malformed one reads as "no choice"
 *  (null), which keeps the album in its intrinsic order. */
fun parseAlbumSort(raw: String?): AlbumSort? {
    if (raw.isNullOrBlank()) return null
    val parts = raw.split(":")
    if (parts.size != 2) return null
    val field = when (parts[0]) {
        "date" -> AlbumSortField.DATE
        "name" -> AlbumSortField.NAME
        else -> return null
    }
    val dir = when (parts[1]) {
        "asc" -> AlbumSortDir.ASC
        "desc" -> AlbumSortDir.DESC
        else -> return null
    }
    return AlbumSort(field, dir)
}
