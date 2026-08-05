package com.simplephotos.ui.screens.library

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Rename decision for person / pet clusters (#39).
 *
 * `runBlocking` rather than `runTest` — `kotlinx-coroutines-test` is not a
 * dependency of this module and nothing under test delays, so adding one to
 * await a lambda that returns immediately would buy nothing.
 */
class ClusterRenameTest {

    /** Records what the "server" was asked to do, and can be told to fail. */
    private class FakeRename(private val throws: Exception? = null) {
        val calls = mutableListOf<String>()
        suspend fun invoke(name: String) {
            calls += name
            throws?.let { throw it }
        }
    }

    @Test
    fun `commits a trimmed name`() = runBlocking {
        val fake = FakeRename()
        val outcome = performClusterRename("  Rex  ", fake::invoke)

        assertEquals(RenameOutcome.Renamed("Rex"), outcome)
        assertEquals(listOf("Rex"), fake.calls)
    }

    @Test
    fun `blank input sends no request at all`() = runBlocking {
        val fake = FakeRename()
        val outcome = performClusterRename("", fake::invoke)

        assertEquals(RenameOutcome.Skipped, outcome)
        assertEquals(emptyList<String>(), fake.calls)
    }

    @Test
    fun `whitespace-only input would blank the label, so it is skipped`() = runBlocking {
        // The dialog disables Save while blank, but "   " is not blank to that
        // check — it is only blank after the trim that happens here. Without
        // this guard the request goes out with an empty name and wipes the
        // cluster's label.
        val fake = FakeRename()
        val outcome = performClusterRename("   \t \n ", fake::invoke)

        assertEquals(RenameOutcome.Skipped, outcome)
        assertEquals(emptyList<String>(), fake.calls)
    }

    @Test
    fun `a failed request reports failure so the caller keeps the old label`() = runBlocking {
        val fake = FakeRename(throws = RuntimeException("503 Service Unavailable"))
        val outcome = performClusterRename("Rex", fake::invoke)

        assertEquals(RenameOutcome.Failed("503 Service Unavailable"), outcome)
        // The attempt was made — this is a server failure, not a skip. The two
        // must stay distinguishable: a skip leaves the label correct, a failure
        // leaves it stale and has to be surfaced.
        assertEquals(listOf("Rex"), fake.calls)
    }

    @Test
    fun `a null-message exception still produces a non-empty error`() = runBlocking {
        // The pre-#39 person path assigned `e.message` straight to the error
        // banner, so any exception without a message surfaced as blank — the
        // user saw an error state with nothing in it.
        val outcome = performClusterRename("Rex", FakeRename(throws = NullPointerException())::invoke)

        assertTrue("expected a Failed outcome, got $outcome", outcome is RenameOutcome.Failed)
        assertEquals("NullPointerException", (outcome as RenameOutcome.Failed).message)
    }

    @Test
    fun `renaming to the currently displayed label is still a real rename`() = runBlocking {
        // Guards against "optimising" this with a `trimmed == current` skip.
        // PetDetailViewModel.label falls back to the SPECIES when the cluster
        // has no stored label, so a pet shown as "Dog" with a null label typed
        // as "Dog" is a genuine rename. Comparing against the displayed string
        // would silently drop it.
        val fake = FakeRename()
        val outcome = performClusterRename("Dog", fake::invoke)

        assertEquals(RenameOutcome.Renamed("Dog"), outcome)
        assertEquals(listOf("Dog"), fake.calls)
    }
}
