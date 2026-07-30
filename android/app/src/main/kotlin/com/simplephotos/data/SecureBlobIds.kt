/**
 * Reading the secure-gallery id set — and the single rule that governs it:
 * **failing to read the set is not the same as "nothing is secured"** (todo B5).
 *
 * ## What was wrong
 *
 * `SecureGalleryRepository.getSecureBlobIds()` used to be
 * `try { … } catch (_: Exception) { emptySet() }`. Every caller reads the result
 * as the complete set of hidden blob ids, and [excludeSecure] short-circuits on
 * an empty set — so one failed request un-hid the **entire** secure gallery
 * across the main grid, every album grid, the smart-album counts and (since E3)
 * the viewer's pager, until the next successful load. A confidentiality filter
 * that fails OPEN is not a filter.
 *
 * It also made recovery code unreachable: `GalleryViewModel`, `AlbumViewModel`
 * and `AlbumDetailViewModel` each carried a `catch { keep existing set }` that
 * could never fire, because the repository had already eaten the throw. Three
 * places believed they were handling this failure; none of them was.
 *
 * ## The shape of the fix
 *
 * There is no longer any way to obtain a bare `Set<String>` from the fetch. The
 * only public read returns [SecureBlobIds], where "the server said nothing is
 * secured" ([SecureBlobIds.Known] with an empty set) and "we do not know what is
 * secured" ([SecureBlobIds.Unavailable]) are different values that the type
 * system will not let a caller confuse. `?: emptySet()` is not expressible.
 *
 * Callers fail **closed**, in the order the todo sanctions — *keep the previous
 * set, or refuse to render*:
 *
 *  - a surface holding a previous set keeps it and logs (the polling loops);
 *  - a stateless surface ([com.simplephotos.data.album.AlbumPhotoResolver]) gets
 *    the last set that loaded — including one persisted across process death, so
 *    an offline cold start still hides what it hid yesterday instead of showing
 *    everything;
 *  - only when *no* set has ever loaded does a surface get [Unavailable], and
 *    then it renders its error/loading state rather than an unfiltered library.
 */
package com.simplephotos.data

import android.util.Log
import kotlinx.coroutines.CancellationException

private const val TAG = "SecureBlobIds"

/**
 * The outcome of one attempt to read the blob ids that live in a secure gallery.
 *
 * Two cases, deliberately not collapsible into `Set<String>`: the whole defect
 * this type exists to prevent was spelling the second one as the first.
 */
sealed interface SecureBlobIds {
    /**
     * A usable answer. Filter on [ids].
     *
     * @param stale true when the fetch failed and these are the last ids that
     *   loaded successfully. Still correct to filter on — it is the previous
     *   answer, which is what failing closed means — but worth a log, because a
     *   photo secured on another device since then is not in it yet.
     */
    data class Known(val ids: Set<String>, val stale: Boolean) : SecureBlobIds

    /**
     * The fetch failed and no set has ever loaded, so the caller genuinely does
     * not know what is hidden. **Not** an empty set: rendering the library
     * unfiltered here is exactly the leak.
     */
    data object Unavailable : SecureBlobIds
}

/**
 * Thrown by a surface that cannot resolve its list without knowing what to hide.
 *
 * Carries a user-facing message because the ViewModels that catch it put
 * `e.message` straight on screen; "refuse to render" has to say why, or it reads
 * as a blank screen with no cause.
 */
class SecureFilterUnavailableException : Exception(
    "Can't confirm which photos are in a secure gallery — not showing the " +
        "library until the server answers."
)

/**
 * Fold one fetch attempt, plus whatever was last known, into the set to filter
 * on. Failing closed lives here so it is decided **once** rather than at each of
 * the five call sites — the "two derivations of one list will drift" trap this
 * repo keeps re-learning.
 *
 * Pure apart from logging (`android.util.Log` is a no-op under
 * `testOptions.unitTests.returnDefaultValues`, which this module sets precisely
 * so recovery paths can log *and* be unit-tested), so
 * `SecureBlobIdsTest` pins the policy without a device or a fake `ApiService`.
 *
 * @param lastKnown the newest set that has loaded successfully, or null if none
 *   ever has — in-memory this process, or restored from disk.
 */
suspend fun foldSecureBlobIds(
    lastKnown: Set<String>?,
    fetch: suspend () -> Set<String>,
): SecureBlobIds = try {
    SecureBlobIds.Known(fetch(), stale = false)
} catch (e: CancellationException) {
    // Structured concurrency: the caller's scope was cancelled, which is not a
    // filter failure and must not be reported as one.
    throw e
} catch (e: Exception) {
    if (lastKnown == null) {
        Log.e(TAG, "secure id fetch failed and no set has ever loaded — " +
            "failing CLOSED, the caller must not render an unfiltered list", e)
        SecureBlobIds.Unavailable
    } else {
        Log.w(TAG, "secure id fetch failed — keeping the last known set of " +
            "${lastKnown.size} id(s); anything secured elsewhere since is not " +
            "in it yet", e)
        SecureBlobIds.Known(lastKnown, stale = true)
    }
}
