/**
 * ViewerHandoff — the list a non-album grid hands to the full-screen pager, and
 * the id space it is expressed in (#52 follow-up, E3a).
 *
 * ## Two defects, one cause
 *
 * E3 gave the album grid and the pager one resolver, so an album pages exactly
 * what its grid drew. The five grids that are *not* albums — Search, People,
 * Pets, Memories, Trips — navigate with `Screen.PhotoViewer.createRoute(photoId)`
 * and no list, so the viewer fell through to the resolver's gallery branch. Two
 * things were wrong with that, and only the second was recorded:
 *
 *  1. **The id space did not match.** Those grids list *server* photo ids —
 *     `listFaceClusterPhotos(...).map { it.photoId }` and friends — while the
 *     viewer locates the tapped photo by [PhotoEntity.localId], which
 *     `PhotoRepository.buildSyncedEntity` assigns as a fresh random UUID. A
 *     server id can therefore never equal a local id. Tapping a face, pet,
 *     memory or trip photo resolved to no page **every single time**: before E3
 *     that was silently coerced to page 0 (an unrelated photo — and a leak in a
 *     different hat if page 0 happened to be sensitive), and after E3 it renders
 *     "Photo not found". Search was the only one of the five that mapped
 *     (`localPhoto?.localId ?: result.id`), which is why only Search ever
 *     appeared to work.
 *  2. **The order did not match.** Even with the ids fixed, the gallery branch
 *     pages `takenAt DESC` — not the order a face cluster, a trip or a relevance
 *     -ranked search actually displayed.
 *
 * Both are the same root cause: the grid resolved a list and then threw it away.
 * The fix is the shape this repo keeps arriving at — hand the resolved list over
 * instead of re-deriving it — which is what web has always done via
 * `location.state` (`web/src/pages/Viewer.tsx`, `navigateToPhoto`).
 *
 * ## Why the projection is not "just a map lookup"
 *
 * The grid draws thumbnails straight from the server (`/api/photos/{id}/thumb`),
 * so it can render a photo this device has never mirrored. The pager cannot: it
 * renders a [PhotoEntity]. The two lists are therefore legitimately different
 * lengths, and [GridPhotoIds] keeps both rather than pretending otherwise —
 * [GridPhotoIds.serverIds] for the tiles, [GridPhotoIds.viewerIds] for the pager.
 *
 * A tap on an unmirrored tile falls back to the raw server id, which is in no
 * one's local mirror, so [AlbumPhotoResolver] answers `pageIndexOf == -1` and the
 * viewer says "Photo not found". That is the same bounded, logged failure E3
 * chose deliberately. **Do not "fix" it by coercing to page 0** — that is defect
 * 1 above, restored.
 */
package com.simplephotos.data.album

import com.simplephotos.data.local.entities.PhotoEntity

/**
 * The key the launching grid writes its [GridPhotoIds.viewerIds] under, read back
 * by `PhotoViewerViewModel` from its own `SavedStateHandle`.
 *
 * Defined once, for the same reason [albumSortPrefKey] is: the writer and the
 * reader are in different files, and a second spelling would silently drop the
 * handoff and fall back to the gallery order — i.e. it would look exactly like
 * the bug it fixes, with nothing failing.
 */
const val VIEWER_PHOTO_IDS_KEY = "viewerPhotoIds"

/**
 * One non-album grid's photo ids, in the order it renders them, in both id spaces.
 *
 * @property serverIds the grid's own order, as it received it from the server
 *   endpoint. What the tiles fetch thumbnails by; never handed to the viewer.
 * @property viewerIds the same list projected onto the local mirror and reduced
 *   to what the pager can actually render, order preserved. This is the handoff.
 * @property serverToLocal the projection itself, kept so a tap can be resolved
 *   without a second lookup — see [viewerIdFor].
 */
data class GridPhotoIds(
    val serverIds: List<String> = emptyList(),
    val viewerIds: List<String> = emptyList(),
    private val serverToLocal: Map<String, String> = emptyMap(),
) {
    /**
     * The id to navigate with when [serverId]'s tile is tapped.
     *
     * Falls back to [serverId] itself when the photo is not mirrored locally,
     * matching what `SearchScreen` already did. That id resolves to no page, so
     * the viewer reports "Photo not found" rather than opening a stranger's
     * photo — see this file's header.
     */
    fun viewerIdFor(serverId: String): String = serverToLocal[serverId] ?: serverId

    companion object {
        val EMPTY = GridPhotoIds()
    }
}

/**
 * Project a grid's server photo ids onto the local mirror, preserving grid order.
 *
 * Pure so the id-space rule is unit-testable without a device — the rule is the
 * whole defect, and it is invisible in a debugger because both id spaces are
 * strings that look alike.
 *
 * @param serverIds the grid's order, straight from the server endpoint.
 * @param mirror the rows `PhotoRepository.getPhotosByServerPhotoIds(serverIds)`
 *   returned — a subset, in arbitrary order.
 */
fun gridPhotoIds(serverIds: List<String>, mirror: List<PhotoEntity>): GridPhotoIds {
    val serverToLocal = mirror.asSequence()
        .mapNotNull { photo -> photo.serverPhotoId?.let { it to photo.localId } }
        .toMap()
    return GridPhotoIds(
        serverIds = serverIds,
        // mapNotNull, not map: an unmirrored id has nothing for the pager to
        // render, so carrying it would only make the handoff claim a page that
        // cannot exist. The grid still shows the tile.
        viewerIds = serverIds.mapNotNull { serverToLocal[it] },
        serverToLocal = serverToLocal,
    )
}

/**
 * Restore [requested]'s order over the rows the mirror returned.
 *
 * The Room `IN (...)` lookup answers in arbitrary order, and the handed-over
 * order *is* the thing E3a exists to preserve — sorting or re-querying here
 * would be a second derivation of the list, which is precisely the defect E3
 * removed.
 *
 * Drops ids with no row (the photo is not mirrored) and de-duplicates. The
 * de-duplication is not defensive tidiness: `HorizontalPager` is keyed on
 * `localId`, and a repeated key crashes it.
 */
fun orderPhotosBy(requested: List<String>, found: List<PhotoEntity>): List<PhotoEntity> {
    val byLocalId = found.associateBy { it.localId }
    val seen = HashSet<String>(requested.size)
    return requested.mapNotNull { if (seen.add(it)) byLocalId[it] else null }
}
