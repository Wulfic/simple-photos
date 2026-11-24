/**
 * Secure-gallery exclusion — shared across every "regular" surface so they all
 * hide the exact same set of photos.
 */
package com.simplephotos.data

import com.simplephotos.data.local.entities.PhotoEntity

/**
 * Remove photos that currently live in a secure gallery.
 *
 * A photo is secure-hidden when its [PhotoEntity.serverBlobId] **or** its
 * [PhotoEntity.serverPhotoId] appears in [secureBlobIds]. Rows with neither
 * (local-only, never uploaded) can't be in a secure gallery and pass through.
 *
 * ## Why the photo id too (TODO.md S1)
 *
 * The set is deliberately mixed: `/galleries/secure/blob-ids` carries each
 * secured item's `original_blob_id`, which the server stores as the owning
 * **photo id** — the very key it hides by (`photos.id NOT IN (original_blob_id)`).
 * Matching only the blob id missed every device row whose blob was its OWN
 * upload: a phone re-backing-up a photo already in a secure album gets mapped
 * onto the hidden server photo but keeps the blob it sent. 158 secured photos
 * resurfaced in the main grid that way on 2026-11-17. UUIDs from the two spaces
 * cannot collide, so the wider match cannot hide an unsecured photo.
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
    return filter { row ->
        val blob = row.serverBlobId
        val photo = row.serverPhotoId
        (blob == null || blob !in secureBlobIds) && (photo == null || photo !in secureBlobIds)
    }
}
