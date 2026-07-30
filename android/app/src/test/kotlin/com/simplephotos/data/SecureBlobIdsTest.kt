package com.simplephotos.data

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Pins [foldSecureBlobIds] — the one place that decides what a *failed* read of
 * the secure id set means (todo B5).
 *
 * The defect was never in the filter: `SecureExclusionTest` already pins
 * [excludeSecure]. It was that a failed fetch was spelled `emptySet()`, and an
 * empty set is precisely the input that makes the filter hide nothing. So these
 * tests assert the **distinction between "nothing is secured" and "we do not
 * know"**, and every one of them is written against
 * [swallowedBeforeTheFix] — the old one-liner, kept verbatim — so they prove the
 * real defect rather than a hypothetical one.
 *
 * `runBlocking` rather than `runTest`: `kotlinx-coroutines-test` is not a
 * dependency of this module (same reason `ClusterRenameTest` uses it).
 */
class SecureBlobIdsTest {

    /**
     * `SecureGalleryRepository.getSecureBlobIds()` as it shipped:
     *
     * ```kotlin
     * try { api.getSecureBlobIds().blobIds.toSet() } catch (_: Exception) { emptySet() }
     * ```
     */
    private suspend fun swallowedBeforeTheFix(fetch: suspend () -> Set<String>): Set<String> =
        try { fetch() } catch (_: Exception) { emptySet() }

    private fun failing(): suspend () -> Set<String> = { throw java.io.IOException("timeout") }

    private fun photo(localId: String, serverBlobId: String? = localId) =
        com.simplephotos.data.local.entities.PhotoEntity(
            localId = localId,
            filename = "$localId.jpg",
            takenAt = 0L,
            mimeType = "image/jpeg",
            mediaType = "image",
            width = 100,
            height = 100,
            syncStatus = com.simplephotos.data.local.entities.SyncStatus.SYNCED,
            createdAt = 0L,
            isFavorite = false,
            serverBlobId = serverBlobId,
        )

    // ── The core distinction ────────────────────────────────────────────────

    @Test
    fun `a successful read is Known and fresh`() = runBlocking {
        val read = foldSecureBlobIds(lastKnown = null) { setOf("a", "b") }
        assertEquals(SecureBlobIds.Known(setOf("a", "b"), stale = false), read)
    }

    @Test
    fun `a server that secures nothing is Known-empty, not Unavailable`() = runBlocking {
        // The honest empty answer must stay usable: a user with no secure
        // gallery has to see their whole library, and must not be gated behind
        // a filter that never resolves.
        val read = foldSecureBlobIds(lastKnown = null) { emptySet() }
        assertEquals(SecureBlobIds.Known(emptySet(), stale = false), read)
    }

    @Test
    fun `a failed read with nothing known is Unavailable, NOT an empty set`() = runBlocking {
        val read = foldSecureBlobIds(lastKnown = null, fetch = failing())

        assertEquals(SecureBlobIds.Unavailable, read)
        // The whole bug in one line: the old code answered this case and the
        // "server secures nothing" case with the identical value, so no caller
        // could tell them apart no matter how carefully it was written.
        assertEquals(emptySet<String>(), swallowedBeforeTheFix(failing()))
        assertNotEquals(read, foldSecureBlobIds(lastKnown = null) { emptySet() })
    }

    @Test
    fun `a failed read keeps the last known set and marks it stale`() = runBlocking {
        val read = foldSecureBlobIds(lastKnown = setOf("secret"), fetch = failing())

        assertEquals(SecureBlobIds.Known(setOf("secret"), stale = true), read)
        // Before the fix the same failure returned an empty set, discarding a
        // set the caller already held.
        assertTrue(swallowedBeforeTheFix(failing()).isEmpty())
    }

    @Test
    fun `a successful read is never marked stale, even when it repeats the last known set`() =
        runBlocking {
            val read = foldSecureBlobIds(lastKnown = setOf("secret")) { setOf("secret") }
            assertFalse((read as SecureBlobIds.Known).stale)
        }

    @Test
    fun `a successful empty read un-hides, because the user really did unsecure everything`() =
        runBlocking {
            // The mirror image of the bug: removing the last photo from a secure
            // gallery must genuinely stop hiding it. Failing closed must not
            // become "hide forever".
            val read = foldSecureBlobIds(lastKnown = setOf("secret")) { emptySet() }
            assertEquals(SecureBlobIds.Known(emptySet(), stale = false), read)
        }

    // ── What the distinction is worth, measured through the real filter ─────

    @Test
    fun `swallowing a failure un-hid every secured photo`() = runBlocking {
        val library = listOf(photo("a"), photo("secret"), photo("b"))

        // Old behaviour: one bad request and the secure gallery is on screen.
        val swallowed = library.excludeSecure(swallowedBeforeTheFix(failing()))
        assertEquals(listOf("a", "secret", "b"), swallowed.map { it.localId })

        // New behaviour with a set already loaded: still hidden.
        val kept = foldSecureBlobIds(lastKnown = setOf("secret"), fetch = failing())
        assertEquals(
            listOf("a", "b"),
            library.excludeSecure((kept as SecureBlobIds.Known).ids).map { it.localId },
        )
    }

    @Test
    fun `Unavailable carries no ids a caller could accidentally filter on`() = runBlocking {
        // `SecureBlobIds.Unavailable` has no `ids` member, so the compiler
        // rejects the `?: emptySet()` shortcut that recreates the bug. This test
        // documents the intent; the type enforces it.
        val read = foldSecureBlobIds(lastKnown = null, fetch = failing())
        assertTrue(read !is SecureBlobIds.Known)
    }

    // ── Structured concurrency ──────────────────────────────────────────────

    @Test
    fun `cancellation propagates instead of being reported as a filter failure`() = runBlocking {
        try {
            foldSecureBlobIds(lastKnown = setOf("secret")) {
                throw CancellationException("scope closed")
            }
            fail("cancellation must not be folded into a SecureBlobIds value")
        } catch (_: CancellationException) {
            // Expected: a cancelled ViewModel scope is not a server failure, and
            // swallowing it would leave a coroutine reporting results into a
            // dead screen.
        }
    }
}
