package com.simplephotos.data.sync

import com.simplephotos.data.remote.dto.LookupHashesResponse
import kotlinx.coroutines.CancellationException
import retrofit2.HttpException

/**
 * What the server said about one content hash, asked before uploading it.
 *
 * TODO.md F2: a reinstalled phone re-uploaded every photo it held. The server
 * discarded each media blob as a content-hash duplicate but stored the
 * thumbnail, then answered `duplicate` — one orphan thumbnail per photo
 * (298/298 measured on CT132, 2026-11-25). Asking first sends no bytes at all.
 */
sealed interface HashLookup {
    /** The server already holds this content: adopt these ids, upload nothing. */
    data class Held(
        val photoId: String,
        val blobId: String,
        val thumbBlobId: String?,
    ) : HashLookup

    /** The server does not hold it: upload. */
    object Absent : HashLookup

    /**
     * No usable answer (network error, old server, malformed hit): upload, exactly
     * as before this lookup existed. Fails open on bandwidth, never on correctness —
     * a wrong "held" would mark an un-uploaded photo as backed up.
     */
    object Unknown : HashLookup
}

/**
 * Per-pass lookup. A server that has no `lookup-hashes` route (404/405 — every
 * server before `87927fd`) disables it for the rest of the pass, so a phone on an
 * older server costs one wasted request per pass, not one per photo.
 *
 * Plain JVM class: the network call and the logger are injected, because
 * `android.util.Log` throws in JVM tests.
 */
class ServerHashLookup(
    private val lookup: suspend (List<String>) -> LookupHashesResponse,
    private val onFailure: (error: Throwable, disabledForPass: Boolean) -> Unit,
) {
    var disabled: Boolean = false
        private set

    suspend fun find(hash: String): HashLookup {
        if (disabled) return HashLookup.Unknown
        val res = try {
            lookup(listOf(hash))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            disabled = e is HttpException && e.code() in UNSUPPORTED
            onFailure(e, disabled)
            return HashLookup.Unknown
        }
        val match = res.matches?.get(hash) ?: return HashLookup.Absent
        val photoId = match.photoId
        val blobId = match.encryptedBlobId
        if (photoId.isNullOrEmpty() || blobId.isNullOrEmpty()) return HashLookup.Unknown
        return HashLookup.Held(photoId, blobId, match.encryptedThumbBlobId)
    }

    private companion object {
        val UNSUPPORTED = setOf(404, 405)
    }
}
