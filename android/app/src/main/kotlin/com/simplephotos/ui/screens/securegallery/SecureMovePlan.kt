/**
 * Pure planning for the push direction of the cross-secure-album move (#43):
 * select items in the OPEN album and send them to another secure album.
 *
 * Kept free of Android / ViewModel dependencies so the part that can be wrong is
 * JVM-unit-testable, the same move `RenditionChoice.kt` and `ClusterRename.kt`
 * made. Mirrors `web/src/gallery/secureMovePicker.ts` so both clients agree on
 * what moves and where.
 */
package com.simplephotos.ui.screens.securegallery

import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem

object SecureMovePlan {
    /** A single move: reassign [itemId] from [sourceGalleryId] to the target. */
    data class Move(val sourceGalleryId: String, val itemId: String)

    /**
     * Expand a selection of representative tile ids to every underlying item id,
     * pulling in all frames of any selected burst. The grid collapses a burst to
     * one tile, but a MOVE must carry every frame (mirrors secure-add, which adds
     * all frames) — otherwise a burst is split across two albums.
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
     * Resolve a selection to concrete moves INTO [targetGalleryId], dropping any
     * item already in the target (a no-op move) and any with no source gallery.
     * Source comes from each item's own `galleryId`, which is right both for a
     * real album (every item shares it) and a smart view (items span albums).
     */
    fun planMovesToTarget(
        items: List<SecureGalleryItem>,
        selectedIds: Set<String>,
        targetGalleryId: String,
    ): List<Move> {
        val byId = items.associateBy { it.id }
        return selectedIds.mapNotNull { id ->
            val src = byId[id]?.galleryId
            if (src == null || src == targetGalleryId) null else Move(src, id)
        }
    }

    /**
     * Real albums a selection can be pushed INTO, excluding the album currently
     * open. For a synthetic smart view [currentGalleryId] is null, so every album
     * is offered — each item still routes from its own source gallery and
     * same-source moves are dropped by [planMovesToTarget].
     */
    fun moveTargets(
        galleries: List<SecureGallery>,
        currentGalleryId: String?,
    ): List<SecureGallery> = galleries.filter { it.id != currentGalleryId }
}
