/**
 * AlbumPhotoResolver — the single resolution of "which photos does this surface
 * show, and in which order" (#52 follow-up, E3).
 *
 * ## Why this exists
 *
 * The album grid and the full-screen pager used to derive their lists
 * separately from the same query. `AlbumDetailViewModel` ran
 * `getAlbumPhotos → excludeSecure → sortAlbumItems`; `PhotoViewerViewModel` ran
 * `getAlbumPhotos → collapseBursts` and nothing else. Three consequences, all
 * shipped:
 *
 *  1. **The pager ignored the #52 sort entirely.** The tapped photo is located
 *     by id, so the FIRST frame was always right and every swipe after it was
 *     wrong — exactly the shape of the bug report.
 *  2. **The pager did not exclude secure photos**, so swiping inside an album
 *     could page into a photo the secure gallery exists to hide, and every index
 *     past it was shifted. A confidentiality defect, not an ordering one — and
 *     it applied to the main gallery's viewer too, which resolved
 *     `getAllPhotos()` with no filter at all.
 *  3. **The pager collapsed bursts unconditionally** while the grid collapsed
 *     them only for smart albums, so a regular album holding a burst rendered
 *     every frame and paged one page per burst.
 *
 * The fix is the shape this repo keeps arriving at: one function, called twice.
 * Neither ViewModel calls [PhotoRepository.getAlbumPhotos] any more — they have
 * no way left to build a divergent list, so the two agree **by construction**
 * rather than by convention. Convention is what failed here: twice silently, and
 * once visibly.
 *
 * ## The burst policy is web's, and it is the opposite of what E3 assumed
 *
 * E3 read divergence 3 as "the grid should collapse like the pager". It is the
 * other way round. `web/src/hooks/useAlbumPhotos.ts` collapses bursts for smart
 * albums **only**, deliberately: *"regular albums keep every frame a user
 * explicitly added, so removal/secure-add over the rendered list stays faithful
 * to the manifest"*. [PhotoRepository.getAlbumPhotos] already encodes precisely
 * that policy, so the pager's unconditional collapse is the side that was wrong.
 * Collapsing the regular-album grid instead would have made Android diverge from
 * web *and* broken the manifest-faithfulness that album removal depends on.
 *
 * That policy stays inside `getAlbumPhotos` rather than moving here because
 * `smart-recents` must collapse BEFORE it caps to 100, or a 46-shot burst eats
 * 46 of the 100 slots. Collapse-then-cap cannot be expressed as a post-filter,
 * so it cannot be lifted out of the query.
 */
package com.simplephotos.data.album

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import com.simplephotos.data.collapseBursts
import com.simplephotos.data.excludeSecure
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SecureGalleryRepository
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import javax.inject.Inject
import javax.inject.Singleton

/**
 * One resolved surface. The grid renders [photos]; the pager pages [photos].
 * There is no third list.
 */
data class ResolvedPhotos(
    /**
     * Membership *before* burst collapse, secure-excluded. The viewer's burst
     * filmstrip resolves a stack's frames out of this without a network
     * round-trip; the grid never renders it.
     *
     * For smart albums `getAlbumPhotos` has already collapsed by the time this
     * sees the list, so the filmstrip is a no-op there — that is unchanged by
     * this refactor, not introduced by it.
     */
    val members: List<PhotoEntity>,
    /**
     * [photos] before the sort — the album's intrinsic order after exclusion and
     * the burst policy.
     *
     * Exists so the grid can re-order on a sort tap without a reload while
     * preserving the invariant `photos == sortAlbumPhotos(tiles, sort)`. Sorting
     * [members] instead would give the same answer today only because albums do
     * not collapse, and would silently drift the day that changed.
     */
    val tiles: List<PhotoEntity>,
    /**
     * Exactly what the grid renders and what the pager pages. Element-for-element
     * equal across both surfaces because it *is* one list, resolved once.
     */
    val photos: List<PhotoEntity>,
    /**
     * The album's persisted sort. `null` means intrinsic order — the pre-#52
     * ordering, which is what preserves "Recently Added"'s add-order — and is
     * also always the value for the main gallery, which has no sort control.
     */
    val sort: AlbumSort?,
    /**
     * Blob ids currently inside a secure gallery, as applied to [photos].
     * Exposed so a caller filtering a *different* list (the album's add-photos
     * picker) hides the same set instead of fetching its own and drifting.
     */
    val secureBlobIds: Set<String>,
)

/**
 * Apply a (possibly absent) album sort to photos.
 *
 * The one place the #52 comparator is applied to a `PhotoEntity` list. `null`
 * leaves the intrinsic order untouched rather than falling back to a default —
 * "the user has not chosen" and "the user chose date-desc" are different states
 * and only the former must preserve add-order.
 */
fun sortAlbumPhotos(photos: List<PhotoEntity>, sort: AlbumSort?): List<PhotoEntity> =
    if (sort == null) photos
    else sortAlbumItems(photos, sort, { it.takenAt }, { it.filename }, { it.localId })

/**
 * The DataStore key holding an album's chosen sort.
 *
 * Defined once because the grid writes it and the pager reads it: a second
 * spelling of this string would silently hand the pager a different order again,
 * which is the whole defect.
 */
fun albumSortPrefKey(albumId: String) = stringPreferencesKey("albumSort:$albumId")

/**
 * The pipeline itself, with every input already fetched.
 *
 * Pure by design — no Room, no Hilt, no DataStore — so the exclusion, collapse
 * and ordering rules are JVM-unit-testable without a device. Same split
 * [AlbumSort] and `RenditionChoice.kt` use, and the reason `AlbumPhotoResolver`
 * below is a thin fetch-and-delegate with no logic of its own.
 *
 * **Order of operations is load-bearing:** `excludeSecure → collapse → sort`.
 *
 * - Excluding first means a secured photo can never become a burst's
 *   representative, and never occupies an index.
 * - Collapsing before the sort makes a stack's representative the frame the
 *   album's *intrinsic* order puts first — the same cover the gallery shows —
 *   and then reorders whole stacks. Sorting first would let the chosen order
 *   pick a different representative for the same burst, so the grid's cover
 *   image would change when you changed the sort.
 *
 * @param collapseBurstStacks true only for the main gallery. Album membership
 *   arrives from [PhotoRepository.getAlbumPhotos], which has already applied the
 *   per-kind policy described in this file's header.
 */
fun resolvePhotos(
    members: List<PhotoEntity>,
    secureBlobIds: Set<String>,
    sort: AlbumSort?,
    collapseBurstStacks: Boolean,
): ResolvedPhotos {
    val visible = members.excludeSecure(secureBlobIds)
    val tiles = if (collapseBurstStacks) visible.collapseBursts() else visible
    return ResolvedPhotos(
        members = visible,
        tiles = tiles,
        photos = sortAlbumPhotos(tiles, sort),
        sort = sort,
        secureBlobIds = secureBlobIds,
    )
}

/**
 * The page index [photoId] opens on, or `-1` when it is not on this surface.
 *
 * Lives next to the burst policy because the policy is what creates the need for
 * the fallback: a tap can name a frame that [photos] collapsed away, and the
 * page that actually renders it is its stack's cover.
 *
 * **`-1` is a real answer, not an error to swallow into `0`.** The non-album
 * viewer entry points (search, people, pets, memories, trips) resolve their grids
 * from server endpoints, so they can hand over an id that is secured, or simply
 * not in the local mirror yet. Coercing that to page 0 opens an unrelated photo
 * and calls it the one the user tapped — and for a secured id that is the
 * confidentiality leak this whole change closes, wearing a different hat.
 */
fun ResolvedPhotos.pageIndexOf(photoId: String): Int {
    val direct = photos.indexOfFirst { it.localId == photoId }
    if (direct >= 0) return direct
    val burstId = members.firstOrNull { it.localId == photoId }?.burstId
    if (burstId.isNullOrEmpty()) return -1
    return photos.indexOfFirst { it.burstId == burstId }
}

/**
 * Fetches the inputs [resolvePhotos] needs and delegates. Deliberately holds no
 * ordering or filtering logic of its own — everything testable lives in the pure
 * functions above.
 */
@Singleton
class AlbumPhotoResolver @Inject constructor(
    private val photoRepository: PhotoRepository,
    private val secureGalleryRepository: SecureGalleryRepository,
    private val dataStore: DataStore<Preferences>,
) {

    /**
     * Resolve what a surface shows.
     *
     * @param albumId the album to resolve, or `null` for the main gallery — the
     *   context every non-album viewer entry point (gallery, search, people,
     *   pets, memories, trips) lands in, because they all navigate with
     *   `Screen.PhotoViewer.createRoute(photoId)` and no list. The gallery branch
     *   mirrors `GalleryScreen` exactly: the whole mirror in `takenAt DESC,
     *   filename ASC`, secure-excluded, bursts collapsed, no sort control.
     */
    suspend fun resolve(albumId: String?): ResolvedPhotos = withContext(Dispatchers.IO) {
        // Note: getSecureBlobIds() swallows its own failures and returns an empty
        // set, so a server hiccup means "nothing is secured" and the filter fails
        // OPEN for one load. That is pre-existing behaviour shared with every
        // other caller, not something this resolver introduces — see todo B5.
        val secureBlobIds = secureGalleryRepository.getSecureBlobIds()
        if (albumId == null) {
            return@withContext resolvePhotos(
                members = photoRepository.getAllPhotos().first(),
                secureBlobIds = secureBlobIds,
                sort = null,
                collapseBurstStacks = true,
            )
        }
        resolvePhotos(
            members = photoRepository.getAlbumPhotos(albumId),
            secureBlobIds = secureBlobIds,
            sort = readSort(albumId),
            collapseBurstStacks = false,
        )
    }

    /** Persist the user's choice for [albumId]. Throws on a DataStore failure;
     *  the caller decides whether losing persistence is worth surfacing. */
    suspend fun persistSort(albumId: String, sort: AlbumSort) {
        dataStore.edit { it[albumSortPrefKey(albumId)] = sort.serialize() }
    }

    /** A missing or malformed stored value reads as "no choice" (null), which
     *  keeps the album in its intrinsic order. */
    private suspend fun readSort(albumId: String): AlbumSort? = try {
        parseAlbumSort(dataStore.data.first()[albumSortPrefKey(albumId)])
    } catch (_: Exception) {
        null
    }
}
