/**
 * Secure-gallery exclusion — shared across every "regular" surface so they all
 * hide the exact same set of photos.
 */
package com.simplephotos.data

import com.simplephotos.data.local.entities.PhotoEntity

/**
 * Remove photos that currently live in a secure gallery.
 *
 * A photo is secure-hidden when its [PhotoEntity.serverBlobId] appears in
 * [secureBlobIds]. Photos with no server blob id (local-only, never uploaded)
 * can't be in a secure gallery, so they always pass through.
 *
 * This is the single source of truth for "what does the regular gallery hide",
 * used by the main grid, the album-detail grids, and the album/smart counts.
 * Before it, the main gallery filtered inline while album detail did not — so
 * securing a photo removed it from the gallery but left it visible (and counted)
 * inside its albums (#16: "secure albums don't fully remove media from the
 * regular gallery"). Returns the receiver unchanged when the set is empty.
 */
fun List<PhotoEntity>.excludeSecure(secureBlobIds: Set<String>): List<PhotoEntity> {
    if (secureBlobIds.isEmpty()) return this
    return filter { it.serverBlobId == null || it.serverBlobId !in secureBlobIds }
}
