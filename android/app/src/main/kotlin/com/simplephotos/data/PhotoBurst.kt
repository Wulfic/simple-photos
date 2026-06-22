/**
 * Burst-stack helpers shared across the gallery, viewer, and album screens.
 */
package com.simplephotos.data

import com.simplephotos.data.local.entities.PhotoEntity

/**
 * Collapse burst stacks: keep only the FIRST frame encountered for each
 * burstId, preserving list order. A burst (which may hold dozens of frames)
 * therefore counts and renders as a single item — matching the gallery grid
 * and the web. Non-burst photos (null/empty burstId) pass through untouched.
 */
fun List<PhotoEntity>.collapseBursts(): List<PhotoEntity> {
    val seenBursts = HashSet<String>()
    return filter { p ->
        val bid = p.burstId
        if (bid.isNullOrEmpty()) true else seenBursts.add(bid)
    }
}
