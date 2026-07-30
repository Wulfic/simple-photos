/**
 * Repository for PIN-protected secure gallery operations including
 * creation, unlock, and item management.
 */
package com.simplephotos.data.repository

import android.util.Log
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringSetPreferencesKey
import com.simplephotos.data.SecureBlobIds
import com.simplephotos.data.foldSecureBlobIds
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.SecureGalleryAddItemRequest
import com.simplephotos.data.remote.dto.SecureGalleryAddItemResponse
import com.simplephotos.data.remote.dto.SecureGalleryCreateRequest
import com.simplephotos.data.remote.dto.SecureGalleryCreateResponse
import com.simplephotos.data.remote.dto.SecureGalleryItemsResponse
import com.simplephotos.data.remote.dto.SecureGalleryListResponse
import com.simplephotos.data.remote.dto.SecureGalleryMoveItemRequest
import com.simplephotos.data.remote.dto.SecureGallerySetCropRequest
import com.simplephotos.data.remote.dto.SecureGalleryUnlockRequest
import com.simplephotos.data.remote.dto.SecureGalleryUnlockResponse
import kotlinx.coroutines.flow.first
import retrofit2.HttpException
import javax.inject.Inject
import javax.inject.Singleton

private const val TAG = "SecureGalleryRepo"

/**
 * The last successfully-loaded secure id set, kept across process death.
 *
 * Persisting it is what lets an offline cold start still hide what it hid
 * yesterday. Without it, "fail closed" would mean the gallery refuses to render
 * whenever the server is unreachable — which would regress the offline browsing
 * #3/#8 deliberately added, and would be its own kind of broken.
 *
 * The tradeoff, stated rather than hidden: this writes the *ids* of secured
 * blobs into ordinary app preferences. It reveals which blob ids are hidden to
 * anything that can already read the app's private storage — which can also read
 * the Room mirror. The secure gallery's threat model is a person holding the
 * unlocked phone, and no id here is reachable from the UI.
 */
private val KEY_SECURE_BLOB_IDS = stringSetPreferencesKey("secureBlobIds")

/**
 * Repository for password-protected secure galleries.
 *
 * Secure galleries require a separate unlock step (with the user's account
 * password) before browsing is allowed.  All operations are server-side
 * only — no local caching.
 */
@Singleton
class SecureGalleryRepository @Inject constructor(
    private val api: ApiService,
    private val dataStore: DataStore<Preferences>,
) {
    /** In-memory mirror of [KEY_SECURE_BLOB_IDS]; `null` until one load succeeds
     *  or the persisted set is read back. `@Volatile` because the polling loop
     *  and a screen load can reach it from different dispatcher threads. */
    @Volatile
    private var lastKnownBlobIds: Set<String>? = null

    /** Verify password and obtain a short-lived gallery access token. */
    suspend fun unlock(password: String): SecureGalleryUnlockResponse =
        api.unlockSecureGalleries(SecureGalleryUnlockRequest(password))

    /** List all secure galleries for the current user. */
    suspend fun listGalleries(): SecureGalleryListResponse =
        api.listSecureGalleries()

    /** Create a new secure gallery with the given name. */
    suspend fun createGallery(name: String): SecureGalleryCreateResponse =
        api.createSecureGallery(SecureGalleryCreateRequest(name))

    /** Permanently delete a secure gallery and its item associations. */
    suspend fun deleteGallery(galleryId: String) {
        api.deleteSecureGallery(galleryId)
    }

    /** List items inside a secure gallery (requires gallery token from [unlock]). */
    suspend fun listItems(galleryId: String, galleryToken: String): SecureGalleryItemsResponse =
        api.listSecureGalleryItems(galleryId, galleryToken)

    /**
     * List items across ALL secure galleries in one request (requires gallery
     * token). Feeds the built-in secure smart albums. Each item carries its
     * owning [SecureGalleryItem.galleryId].
     */
    suspend fun listAllItems(galleryToken: String): SecureGalleryItemsResponse =
        api.listAllSecureGalleryItems(galleryToken)

    /** Add a blob to a secure gallery. */
    suspend fun addItem(galleryId: String, blobId: String): SecureGalleryAddItemResponse =
        api.addSecureGalleryItem(galleryId, SecureGalleryAddItemRequest(blobId))

    /**
     * Remove a single item from a secure gallery. The server deletes the
     * cloned blob and the original photo returns to the regular gallery.
     */
    suspend fun removeItem(galleryId: String, itemId: String) {
        api.deleteSecureGalleryItem(galleryId, itemId)
    }

    /**
     * All blob ids that belong to any secure gallery for the current user — the
     * set every "regular" surface filters on via [com.simplephotos.data.excludeSecure].
     *
     * **The only public way to read the set, and it cannot report a failure as
     * "nothing is secured"** (todo B5). It used to return a bare `Set<String>`
     * and swallow every exception into `emptySet()`, which un-hid the whole
     * secure gallery on one bad request; see [SecureBlobIds] for the full
     * account and for what each caller owes on a failure.
     */
    suspend fun secureBlobIds(): SecureBlobIds {
        val known = lastKnownBlobIds ?: readPersistedBlobIds()
        val result = foldSecureBlobIds(known) { fetchSecureBlobIds() }
        if (result is SecureBlobIds.Known && !result.stale) rememberBlobIds(result.ids)
        return result
    }

    /**
     * Drop the fallback set on logout.
     *
     * `AuthRepository.logout()` clears the whole preference store, so the
     * persisted copy goes with it — but this repository is a `@Singleton` and
     * its in-memory mirror outlives the account inside one process. Left behind,
     * the next user's first failed fetch would fall back to the *previous*
     * user's ids. Nothing of theirs would leak (their blob ids cannot match this
     * user's rows, so the effect is over-hiding nothing), and that is exactly
     * why it would never be noticed — same shape as the #38 cursor two lines
     * above it in `logout()`.
     */
    fun forgetSecureBlobIds() {
        lastKnownBlobIds = null
    }

    /**
     * The raw fetch. Throws on anything the caller cannot treat as an answer.
     *
     * A **404 is an answer**: a server predating the secure-gallery endpoint has
     * no secure galleries, so nothing is secured. Folding it in as a failure
     * instead would leave every client on an older server permanently behind its
     * "waiting for the secure filter" gate, which is a worse bug than the one
     * being fixed. Every other status — 401, 5xx, a timeout — is genuinely
     * unknown and propagates.
     */
    private suspend fun fetchSecureBlobIds(): Set<String> = try {
        api.getSecureBlobIds().blobIds.toSet()
    } catch (e: HttpException) {
        if (e.code() == 404) {
            Log.i(TAG, "secure blob-ids endpoint absent (404) — server secures nothing")
            emptySet()
        } else {
            throw e
        }
    }

    /** Restore the set persisted by the last successful load, or null if there
     *  has never been one (or the read itself failed — which is also "unknown",
     *  never "empty"). */
    private suspend fun readPersistedBlobIds(): Set<String>? = try {
        dataStore.data.first()[KEY_SECURE_BLOB_IDS]?.also { lastKnownBlobIds = it }
    } catch (e: Exception) {
        Log.w(TAG, "could not read the persisted secure id set", e)
        null
    }

    /** Record a fresh set for the next failed fetch to fall back on. Persisting
     *  is best-effort: losing it only costs a cold start the fallback. */
    private suspend fun rememberBlobIds(ids: Set<String>) {
        // `GalleryViewModel` polls this every 3 seconds and the set almost never
        // changes, so writing unconditionally would rewrite the preferences file
        // ~1,200 times an hour on a completely idle device — the exact
        // steady-state disk thrash migration 031 exists to prevent. Equality
        // against the in-memory mirror is the whole guard.
        if (ids == lastKnownBlobIds) return
        lastKnownBlobIds = ids
        try {
            dataStore.edit { it[KEY_SECURE_BLOB_IDS] = ids }
        } catch (e: Exception) {
            Log.w(TAG, "could not persist the secure id set; a cold start " +
                "before the next successful fetch will have no fallback", e)
        }
    }

    /**
     * Move an item from [sourceGalleryId] into [targetGalleryId] (#31,
     * cross-secure-album picker). A photo lives in at most one secure album, so
     * this reassigns membership rather than copying.
     */
    suspend fun moveItem(sourceGalleryId: String, itemId: String, targetGalleryId: String) {
        api.moveSecureGalleryItem(
            sourceGalleryId, itemId, SecureGalleryMoveItemRequest(targetGalleryId)
        )
    }

    /**
     * Persist (or clear, with null) non-destructive crop/edit metadata for a
     * secure item (#31). Stored on the item row; applied at display time.
     */
    suspend fun setItemCrop(galleryId: String, itemId: String, cropMetadata: String?) {
        api.setSecureGalleryItemCrop(
            galleryId, itemId, SecureGallerySetCropRequest(cropMetadata)
        )
    }
}
