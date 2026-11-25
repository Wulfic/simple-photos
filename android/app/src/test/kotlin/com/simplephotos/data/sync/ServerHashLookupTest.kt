package com.simplephotos.data.sync

import com.simplephotos.data.remote.dto.HashMatchDto
import com.simplephotos.data.remote.dto.LookupHashesResponse
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import okhttp3.ResponseBody.Companion.toResponseBody
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import retrofit2.HttpException
import retrofit2.Response
import java.io.IOException

/** TODO.md F2b — ask the server before uploading bytes it already holds. */
class ServerHashLookupTest {

    private val hash = "aaaaaaaaaaaa"

    private fun http(code: Int) = HttpException(Response.error<Any>(code, "".toResponseBody(null)))

    /** A lookup that records every call and answers from [answer]. */
    private class Fake(val answer: (List<String>) -> LookupHashesResponse) {
        val calls = mutableListOf<List<String>>()
        val failures = mutableListOf<Pair<Throwable, Boolean>>()
        val lookup = ServerHashLookup(
            lookup = { hashes -> calls += hashes; answer(hashes) },
            onFailure = { e, disabled -> failures += e to disabled },
        )
    }

    @Test
    fun `a held hash is adopted with the server's photo and blob ids`() = runBlocking {
        val fake = Fake {
            LookupHashesResponse(mapOf(hash to HashMatchDto("photo-1", "blob-1", "thumb-1")))
        }

        val got = fake.lookup.find(hash)

        assertEquals(HashLookup.Held("photo-1", "blob-1", "thumb-1"), got)
        assertEquals(listOf(listOf(hash)), fake.calls)
    }

    @Test
    fun `a hash the server lacks is uploaded`() = runBlocking {
        val fake = Fake { LookupHashesResponse(emptyMap()) }
        assertSame(HashLookup.Absent, fake.lookup.find(hash))
    }

    @Test
    fun `a network error uploads, and the next photo still asks`() = runBlocking {
        var first = true
        val fake = Fake {
            if (first) { first = false; throw IOException("reset") }
            LookupHashesResponse(emptyMap())
        }

        assertSame(HashLookup.Unknown, fake.lookup.find(hash))
        assertSame(HashLookup.Absent, fake.lookup.find(hash))

        assertEquals(2, fake.calls.size)
        assertEquals("the failure must reach the log", 1, fake.failures.size)
        assertFalse(fake.failures.single().second)
    }

    /** Every server before 87927fd. One wasted call per pass, not one per photo. */
    @Test
    fun `a server without the route disables the lookup for the rest of the pass`() = runBlocking {
        for (code in listOf(404, 405)) {
            val fake = Fake { throw http(code) }

            assertSame(HashLookup.Unknown, fake.lookup.find(hash))
            assertSame(HashLookup.Unknown, fake.lookup.find("bbbbbbbbbbbb"))

            assertEquals("HTTP $code: asked again after the server said it has no such route",
                1, fake.calls.size)
            assertTrue(fake.lookup.disabled)
            assertTrue("the log must say the lookup is off", fake.failures.single().second)
        }
    }

    @Test
    fun `a server error uploads but does not disable the lookup`() = runBlocking {
        val fake = Fake { throw http(500) }

        fake.lookup.find(hash)
        fake.lookup.find(hash)

        assertEquals(2, fake.calls.size)
        assertFalse(fake.lookup.disabled)
    }

    /** Adopting a hit with no blob would leave a row nothing can download. */
    @Test
    fun `a hit without a photo or blob id is not adopted`() = runBlocking {
        for (bad in listOf(HashMatchDto(null, "blob-1"), HashMatchDto("photo-1", null), HashMatchDto("photo-1", ""))) {
            val fake = Fake { LookupHashesResponse(mapOf(hash to bad)) }
            assertSame("$bad", HashLookup.Unknown, fake.lookup.find(hash))
        }
    }

    @Test
    fun `a response with no matches field is a miss`() = runBlocking {
        val fake = Fake { LookupHashesResponse(null) }
        assertSame(HashLookup.Absent, fake.lookup.find(hash))
    }

    @Test
    fun `cancellation is not swallowed as a lookup failure`() = runBlocking {
        val fake = Fake { throw CancellationException("worker stopped") }
        try {
            fake.lookup.find(hash)
            fail("a cancelled worker must stop, not fall through to an upload")
        } catch (e: CancellationException) {
            assertEquals("worker stopped", e.message)
            assertTrue(fake.failures.isEmpty())
        }
    }
}
