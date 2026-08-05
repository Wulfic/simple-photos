/**
 * Pure planning for the cross-secure-album flows: the push direction (#43,
 * select items in the OPEN album and file them elsewhere) and the removal that
 * shares its burst rules.
 *
 * **The one-secure-album rule is gone (Z1).** A photo may live in several secure
 * albums at once, sharing a single encrypted clone; what survives is only "at
 * most once per *album*". That splits what used to be one operation into two,
 * and they are not interchangeable:
 *
 *  - **MOVE** (`moveItem`) — reassigns the membership row, so the photo leaves
 *    the source album. Still what the #31 *pull* picker wants: "bring these here".
 *  - **ADD** (`addItem`) — an additional membership row against the same clone,
 *    so the photo is in both albums. This is what a "+"-shaped affordance means
 *    everywhere else in the app, and it is what the #43 push flow was silently
 *    getting wrong: it offered a "+" and then moved, so filing a photo into a
 *    second secure album quietly emptied it out of the first. That is the
 *    originally reported Z1 bug, and on Android it outlived the web fix.
 *
 * Kept free of Android / ViewModel dependencies so the part that can be wrong is
 * JVM-unit-testable, the same move `RenditionChoice.kt` and `ClusterRename.kt`
 * made. Mirrors `web/src/gallery/secureMovePicker.ts` so both clients agree on
 * what goes where.
 */
package com.simplephotos.ui.screens.securegallery

import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem

object SecureMovePlan {
    /** A single add: give [blobId] an additional membership in the target album. */
    data class Add(val itemId: String, val blobId: String)

    /**
     * Expand a selection of representative tile ids to every underlying item id,
     * pulling in all frames of any selected burst. The grid collapses a burst to
     * one tile, but the transfer must carry every frame (mirrors secure-add,
     * which adds all frames) — otherwise a burst is split across two albums.
     */
    fun expandBurstSelection(
        items: List<SecureGalleryItem>,
        selectedIds: Set<String>,
    ): Set<String> {
        val framesByBurst = HashMap<String, MutableList<String>>()
        for (it in items) {
            val bid = it.burstId
            if (!bid.isNullOrEmpty()) framesByBurst.getOrPut(bid) { mutableListOf() }.add(it.id)
        }
        val byId = items.associateBy { it.id }
        val out = LinkedHashSet<String>()
        for (id in selectedIds) {
            out.add(id)
            val bid = byId[id]?.burstId
            if (!bid.isNullOrEmpty()) framesByBurst[bid]?.let { out.addAll(it) }
        }
        return out
    }

    /**
     * Resolve a selection to concrete ADDs into the target album.
     *
     * Two deliberate omissions, both copied from web's `planSecureAddsToTarget`
     * rather than re-reasoned:
     *
     * 1. **No "is it already in the target" filter**, and no target parameter to
     *    build one from. The old move planner had one, because a move's source is
     *    knowable from the item's own `galleryId`. An add's answer lives in a
     *    *different* album's membership rows, which the per-album feed this runs
     *    against cannot see. The server answers it authoritatively with a 409, so
     *    the caller treats that as "already there" rather than as a failure.
     *    Deriving it here would be a second derivation of membership — the exact
     *    drift this repo has now recorded ten times — and it would be the *wrong*
     *    one, since it would be a guess from a feed that cannot see the target.
     * 2. **`blobId`, not `id`.** An add is keyed on the clone blob (the server
     *    matches it to find the donor membership and adopt it); the item id is
     *    carried only so a caller can report per-item outcomes.
     */
    fun planAddsToTarget(
        items: List<SecureGalleryItem>,
        selectedIds: Set<String>,
    ): List<Add> {
        val byId = items.associateBy { it.id }
        val seenBlobs = HashSet<String>()
        return selectedIds.mapNotNull { id ->
            val item = byId[id] ?: return@mapNotNull null
            // One add per clone: two selected burst frames sharing a clone would
            // otherwise issue two requests, the second guaranteed to 409.
            if (!seenBlobs.add(item.blobId)) null else Add(item.id, item.blobId)
        }
    }

    /**
     * Every item a removal of [targets] must actually delete: the targets plus
     * all sibling frames of any burst among them.
     *
     * Burst-aware because the grid and viewer collapse a burst to one tile/page,
     * so a naive single-item delete strands the other frames in the album while
     * only the cover returns to the gallery. Gallery-SCOPED — siblings must share
     * BOTH the `burstId` and the owning album — because frames of one burst live
     * in one album, so this only guards a hypothetical cross-album `burstId`
     * collision.
     *
     * Extracted from the view model so the confirmation dialog can describe the
     * *same* set the removal will act on. Asking one derivation what will be
     * removed and another what to say about it is how a prompt ends up accurate
     * about a batch that isn't the one being removed.
     */
    fun expandForRemoval(
        items: List<SecureGalleryItem>,
        targets: List<SecureGalleryItem>,
    ): List<SecureGalleryItem> {
        val burstKeys = targets
            .filter { !it.burstId.isNullOrEmpty() }
            .map { it.galleryId to it.burstId }
            .toSet()
        val siblings = items.filter {
            !it.burstId.isNullOrEmpty() && (it.galleryId to it.burstId) in burstKeys
        }
        return (targets + siblings).distinctBy { it.id }
    }

    /**
     * Real albums a selection can be filed INTO, excluding the album currently
     * open. For a synthetic smart view [currentGalleryId] is null, so every album
     * is offered.
     *
     * Since Z1 an item already in the offered album is no longer excluded here,
     * and cannot be: the add is keyed on the clone, and only the server knows
     * which albums hold it. Adding a photo to an album it is already in is a 409
     * the caller reports as a no-op — see [planAddsToTarget].
     */
    fun addTargets(
        galleries: List<SecureGallery>,
        currentGalleryId: String?,
    ): List<SecureGallery> = galleries.filter { it.id != currentGalleryId }
}
