/**
 * Which quality of a video to play, and what to call it (#49).
 *
 * Pure by design — no Android imports, so it runs in `app/src/test` as a plain
 * JVM test rather than needing Robolectric or a device. That is deliberate: the
 * picker's *behaviour* (what it offers, what it defaults to on a metered link)
 * is exactly the part that cannot be verified by opening a video and looking at
 * it. The port of `web/src/gallery/renditionChoice.ts`; see
 * `RenditionChoiceTest` for the shared cases.
 *
 * ## The wire contract, restated so it is not re-derived at each call site
 *
 * - Renditions arrive **highest first**, `isSource` marking the untouched
 *   original. This module re-sorts anyway; see [offerableRenditions].
 * - `shortEdge` is both the rung's identity and the `?rendition=` selector, and
 *   because the ladder keys on the *short* edge it is also literally the "p"
 *   number a user expects — a portrait `1080x1920` is `1080`, not `1920`.
 * - **An empty list is the normal case.** It means one quality exists, so no
 *   picker should be drawn at all.
 *
 * ## Where Android deliberately diverges from web
 *
 * Web decides "is this connection expensive?" from the Network Information API,
 * which exposes three unreliable, partially-implemented signals that have to be
 * ORed together and still return nothing at all on Safari and Firefox. Android
 * has a definitive answer — `NET_CAPABILITY_NOT_METERED` — so the guesswork has
 * no analogue here and is not ported. What reaches this module is the single
 * resolved boolean from [isConstrained]; reading the radio is the caller's job.
 */
package com.simplephotos.data.media

/** One playable quality, as it arrives on a sync record and is mirrored in Room. */
data class Rendition(
    /** Rung identity, the `?rendition=` selector, and the "p" number. */
    val shortEdge: Int,
    val width: Int,
    val height: Int,
    /** The untouched original. Never assume this is also the highest — sort. */
    val isSource: Boolean,
    /**
     * Encrypted installs: the blob to stream as `spblob://<blobId>`.
     *
     * For the **source** rung this is the photo's own `encrypted_blob_id` — a
     * second reference to bytes the photo already owns, not a copy (which is why
     * migration `037` had to stop the orphan trigger queueing it). Selecting
     * "Original" therefore resolves to the URI the player already has loaded.
     */
    val blobId: String?,
    val codec: String?,
    val sizeBytes: Long,
)

/**
 * Ceiling applied when the connection is expensive.
 *
 * 1080 rather than "one rung down" because the issue asks for a *quality* cap,
 * not a relative step: on an 8K source one rung down is 4K, which is not what
 * "lower on cellular" means to anybody.
 */
const val CONSTRAINED_MAX_SHORT_EDGE = 1080

/**
 * Whether to default to a reduced quality.
 *
 * Both inputs must be true. The issue is explicit that the data saver is the
 * user's switch: **when it is off, always serve highest regardless of network**,
 * so a metered link alone must never downgrade anybody who did not ask for it.
 */
fun isConstrained(dataSaverEnabled: Boolean, metered: Boolean): Boolean =
    dataSaverEnabled && metered

/**
 * Normalise the server's list into what a picker may actually show.
 *
 * Sorts highest-first rather than trusting the server's `ORDER BY`, and drops
 * duplicate rungs keeping the first seen. Neither should ever happen — the
 * table's primary key is `(photo_id, short_edge)` and the query orders
 * descending — but "should never happen" is how a picker ends up offering the
 * same quality twice, and the cost of being sure is one sort of a 2-3 element
 * list.
 *
 * Rungs with a null [Rendition.blobId] are dropped. That is an unencrypted
 * install, where the bytes live behind `/photos/:id/file?rendition=` and this
 * player has no plaintext branch — it streams everything as `spblob://`. Keeping
 * such a rung would put a menu entry on screen that silently does nothing when
 * tapped; dropping it makes the picker genuinely absent there, which is honest.
 * Web filters these for the same reason.
 */
fun offerableRenditions(list: List<Rendition>?): List<Rendition> {
    if (list.isNullOrEmpty()) return emptyList()
    val seen = HashSet<Int>()
    return list
        .filter { it.blobId != null }
        .sortedByDescending { it.shortEdge }
        .filter { seen.add(it.shortEdge) }
}

/**
 * Whether to draw the gear icon at all.
 *
 * A one-entry picker is worse than no picker: it implies a choice the user does
 * not have, and it is the *normal* state for the overwhelming majority of the
 * library — only videos above the 1080p tier ever get a second rung (measured
 * live: 136 of 742).
 */
fun shouldOfferPicker(list: List<Rendition>?): Boolean =
    offerableRenditions(list).size >= 2

/**
 * The rung to start playback on.
 *
 * Returns null for an empty list, which means "play the photo's own blob exactly
 * as the viewer did before #49" — not an error.
 */
fun chooseDefaultRendition(list: List<Rendition>?, constrained: Boolean): Rendition? {
    val offerable = offerableRenditions(list)
    if (offerable.isEmpty()) return null
    if (!constrained) return offerable.first()

    // Highest rung within the cap. `first` over a descending list is that rung.
    // Nothing at or below the cap means every rung is huge (a 4K source whose
    // 1080 rung has not been produced yet). Take the smallest rather than
    // refusing to play — the alternative is a metered client fetching the 4K.
    return offerable.firstOrNull { it.shortEdge <= CONSTRAINED_MAX_SHORT_EDGE }
        ?: offerable.last()
}

/**
 * Whether the mirror's ladder differs from the one the server just sent.
 *
 * **null and empty are the same state** — "this video has one quality" — and
 * they arrive from different places: a pre-#49 server (and every row cached
 * before this field existed) yields null, while a #49 server sends an empty list
 * for the ~600 videos that need no rung. Treating those as different makes the
 * first sync pass against an upgraded server rewrite the entire library, which
 * is the exact O(library) write amplification #38 spent a workstream removing.
 */
fun renditionsEqual(a: List<Rendition>?, b: List<Rendition>?): Boolean =
    (a ?: emptyList<Rendition>()) == (b ?: emptyList<Rendition>())

/**
 * Menu label for a rung.
 *
 * The resolution is shown even for the original: "Original" alone forces the
 * user to guess whether it is bigger than the 1080p entry below it.
 */
fun formatRenditionLabel(r: Rendition): String =
    if (r.isSource) "Original (${r.shortEdge}p)" else "${r.shortEdge}p"
