/**
 * How much work one server→Room sync pass has to do (#38).
 *
 * This file is deliberately pure: no Room, no Retrofit, no `android.util.Log`.
 * The decisions it encodes are the entire safety argument for delta sync, and
 * on this platform the alternative to a pure function is an instrumented test
 * needing a device — which is how the equivalent logic went unverified on the
 * web side until `syncPass.ts` was extracted for exactly this reason.
 *
 * Read `web/src/gallery/hooks/syncPass.ts` alongside this. Three of its four
 * hazards are protocol-level rather than web-specific and are reproduced here:
 * cursor lifetime, the `deleted` handshake, and keeping the FIRST page's head.
 *
 * ## Why every uncertainty resolves to FULL
 *
 * A cursor is a *claim*: "the mirror already holds every change up to sequence
 * N". The delta feed only ever names what changed *after* N, so if that claim
 * is false the missing rows are never mentioned by any future response. The
 * gallery is silently and permanently short, and re-syncing does not repair it.
 *
 * The full walk has no such failure mode — it re-sends everything and the
 * client set-differences, so local damage heals on the next pass. A needless
 * full walk costs one slow pass. A wrongly-trusted cursor costs rows forever.
 * That asymmetry is why this file prefers FULL in every ambiguous case.
 */
package com.simplephotos.data.sync

import com.simplephotos.data.local.entities.PhotoEntity

/** What a pass decided to do, before it does it. */
enum class SyncMode {
    /** `head_seq` matches the cursor: nothing changed, so the pass costs one
     *  small JSON request and touches neither the network nor Room again. This
     *  is the steady state and the entire point of #38. */
    SKIPPED,

    /** Cursor known and the head has moved: fetch only what changed. */
    DELTA,

    /** No usable cursor, an incoherent one, or a server that does not speak
     *  `since`: the historical full walk, which is also the recovery path. */
    FULL,
}

/**
 * Choose the cheapest mode that is provably correct.
 *
 * @param cursor the sequence the mirror is current as of, or null for "unknown".
 * @param headSeq the server's current change-log head, or null when it could
 *   not be established (the summary call failed, or the server predates #38).
 *
 * A null [headSeq] does **not** force a full walk. Losing the summary call
 * costs us the skip shortcut, not correctness — the delta response carries its
 * own `head_seq`, and the `deleted` handshake ([isDeltaFeed]) still catches a
 * server too old to honour `since`.
 */
fun decideSyncMode(cursor: Long?, headSeq: Long?): SyncMode {
    // No cursor at all: cold start, post-logout, or one we refused to trust.
    if (cursor == null || cursor < 0) return SyncMode.FULL

    if (headSeq != null) {
        if (headSeq == cursor) return SyncMode.SKIPPED

        // The head moved BACKWARDS. The log is global and monotonic, so this
        // cannot happen on a server that has simply been running; it means the
        // client is now talking to a restored database or a different server
        // altogether. Our cursor then indexes a sequence space that no longer
        // exists, and a delta against it would return "nothing changed above N"
        // while the mirror holds rows this server has never heard of.
        //
        // Not a case web handles — noted there as a follow-up. Detecting it is
        // one comparison and the alternative is a mirror that never reconciles.
        if (headSeq < cursor) return SyncMode.FULL
    }

    return SyncMode.DELTA
}

/**
 * Whether a response is genuinely a delta feed.
 *
 * This is the protocol handshake, and it is load-bearing. A server predating
 * #38 ignores the unknown `since` query parameter and answers with a **full
 * walk**, whose `photos` are indistinguishable from a delta's. Reading that as
 * a delta means pruning nothing while believing we pruned correctly, and then
 * persisting a cursor that makes the mistake permanent.
 *
 * The server therefore sends `deleted` as **empty rather than absent** on a
 * delta. Absent means "this server does not speak `since`" — fall back to FULL.
 *
 * Gson leaves an absent field at its default, so `null` here is exactly
 * "the key was not in the JSON".
 */
fun isDeltaFeed(deleted: List<String>?): Boolean = deleted != null

/**
 * Which mirror rows a set of tombstones actually removes.
 *
 * A tombstone names a **server photo id** and means "this photo has left the
 * eligible feed" — deleted outright, or claimed by a secure gallery. The client
 * treats both identically.
 *
 * ## The guard is copied from the full walk on purpose
 *
 * `reconcileServerDeletions` only removes rows with a `serverPhotoId` and **no**
 * `localPath`, so a photo that was captured on this device, uploaded, and then
 * merged survives a server-side deletion. Whether that is the right product
 * decision is a separate argument; what matters here is that the delta path and
 * the full path must agree.
 *
 * If a delta deleted a row the full walk would have kept, the next cold start
 * could not restore it — the server no longer lists the photo, so the walk that
 * is supposed to be self-healing has nothing to heal from, and a photo still
 * physically present on the device is gone from the library for good. Divergence
 * between the two paths is precisely how a "recovery" path stops recovering.
 */
fun tombstoneVictims(rows: List<PhotoEntity>, tombstoned: Set<String>): List<PhotoEntity> {
    if (tombstoned.isEmpty()) return emptyList()
    return rows.filter { row ->
        val serverId = row.serverPhotoId
        serverId != null && serverId in tombstoned && row.localPath == null
    }
}
