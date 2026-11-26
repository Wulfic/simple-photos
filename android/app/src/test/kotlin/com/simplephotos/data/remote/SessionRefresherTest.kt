package com.simplephotos.data.remote

import com.simplephotos.data.remote.dto.RefreshResponse
import okhttp3.Headers.Companion.headersOf
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.ResponseBody.Companion.toResponseBody
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import retrofit2.HttpException
import retrofit2.Response
import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/**
 * N1b. Live repro 2026-11-26: a phone holding a dead refresh token sent ≥155
 * `/auth/refresh` calls in 22 s (one per queued 401) until the server's rate
 * limiter answered 429. One definitive "no" must be the last network call for
 * that token, and a transient failure must not be retried by every 401 behind it.
 */
class SessionRefresherTest {

    private class Fixture(
        var access: String? = "access-0",
        var refresh: String? = "refresh-0",
    ) {
        val calls = AtomicInteger()
        var clock = 1_000_000L
        var deadSignals = 0
        var respond: (String) -> RefreshResponse = { error("no response staged") }

        val refresher = SessionRefresher(
            readTokens = { SessionRefresher.Tokens(access, refresh) },
            callRefresh = { rt -> calls.incrementAndGet(); respond(rt) },
            saveTokens = { a, r -> access = a; if (r != null) refresh = r },
            now = { clock },
            onSessionDead = { deadSignals++ },
        )
    }

    private fun httpError(code: Int, headers: Map<String, String> = emptyMap()): HttpException {
        val raw = okhttp3.Response.Builder()
            .request(Request.Builder().url("http://server/api/auth/refresh").build())
            .protocol(Protocol.HTTP_1_1)
            .code(code)
            .message("x")
            .headers(headersOf(*headers.flatMap { listOf(it.key, it.value) }.toTypedArray()))
            .build()
        val body = "{}".toResponseBody("application/json".toMediaTypeOrNull())
        return HttpException(Response.error<Any>(body, raw))
    }

    private fun rotated(n: Int) = RefreshResponse("access-$n", "refresh-$n", 3600)

    @Test
    fun `a successful refresh saves both rotated tokens and retries with the new access token`() {
        val f = Fixture()
        f.respond = { rotated(1) }

        assertEquals("access-1", f.refresher.authenticate("access-0"))
        assertEquals("access-1", f.access)
        assertEquals("refresh-1", f.refresh)
        assertEquals(1, f.calls.get())
    }

    @Test
    fun `a 401 from refresh is final for that token - no further network calls`() {
        val f = Fixture()
        f.respond = { throw httpError(401) }

        repeat(50) { assertNull(f.refresher.authenticate("access-0")) }

        assertEquals("one definitive 401 must be the last refresh for this token", 1, f.calls.get())
        assertEquals("tokens are left for the login flow to replace", "refresh-0", f.refresh)
        assertEquals("the dead session is announced exactly once", 1, f.deadSignals)
    }

    @Test
    fun `a new refresh token after a dead one is tried again`() {
        val f = Fixture()
        f.respond = { throw httpError(401) }
        f.refresher.authenticate("access-0")

        // The user logged in again: new pair in storage.
        f.access = "access-L"
        f.refresh = "refresh-L"
        f.respond = { rotated(2) }

        assertEquals("access-2", f.refresher.authenticate("access-L"))
        assertEquals(2, f.calls.get())
    }

    @Test
    fun `a 429 keeps the tokens and is not retried by the 401s behind it`() {
        val f = Fixture()
        f.respond = { throw httpError(429, mapOf("Retry-After" to "30")) }

        repeat(50) { assertNull(f.refresher.authenticate("access-0")) }

        assertEquals(1, f.calls.get())
        assertEquals("access-0", f.access)
        assertEquals("refresh-0", f.refresh)
        assertEquals("a transient failure is not a dead session", 0, f.deadSignals)

        // Honours Retry-After: still cooling at 29 s, tries again at 30 s.
        f.clock += 29_000
        assertNull(f.refresher.authenticate("access-0"))
        assertEquals(1, f.calls.get())

        f.clock += 1_000
        f.respond = { rotated(1) }
        assertEquals("access-1", f.refresher.authenticate("access-0"))
        assertEquals(2, f.calls.get())
    }

    @Test
    fun `a network failure keeps the tokens and cools down`() {
        val f = Fixture()
        f.respond = { throw IOException("timeout") }

        repeat(20) { assertNull(f.refresher.authenticate("access-0")) }

        assertEquals(1, f.calls.get())
        assertEquals("refresh-0", f.refresh)
        assertEquals(0, f.deadSignals)

        f.clock += SessionRefresher.DEFAULT_COOLDOWN_MS
        f.respond = { rotated(1) }
        assertEquals("access-1", f.refresher.authenticate("access-0"))
    }

    @Test
    fun `a 5xx is transient, not a dead session`() {
        val f = Fixture()
        f.respond = { throw httpError(503) }

        assertNull(f.refresher.authenticate("access-0"))
        assertEquals(0, f.deadSignals)
        assertEquals("refresh-0", f.refresh)
    }

    @Test
    fun `an absurd Retry-After is clamped`() {
        val f = Fixture()
        f.respond = { throw httpError(429, mapOf("Retry-After" to "86400")) }
        f.refresher.authenticate("access-0")

        f.clock += SessionRefresher.MAX_COOLDOWN_MS
        f.respond = { rotated(1) }
        assertEquals("access-1", f.refresher.authenticate("access-0"))
    }

    @Test
    fun `a 401 sent with an older access token retries with the current one without refreshing`() {
        val f = Fixture(access = "access-5")

        assertEquals("access-5", f.refresher.authenticate("access-4"))
        assertEquals(0, f.calls.get())
    }

    @Test
    fun `concurrent 401s refresh once`() {
        val f = Fixture()
        f.respond = { Thread.sleep(50); rotated(1) }
        val pool = Executors.newFixedThreadPool(16)
        val start = CountDownLatch(1)
        val results = (1..32).map {
            pool.submit<String?> { start.await(); f.refresher.authenticate("access-0") }
        }
        start.countDown()

        results.forEach { assertEquals("access-1", it.get(5, TimeUnit.SECONDS)) }
        pool.shutdown()
        assertEquals(1, f.calls.get())
    }
}
