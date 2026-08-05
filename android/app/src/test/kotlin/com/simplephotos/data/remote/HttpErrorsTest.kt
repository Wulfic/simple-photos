package com.simplephotos.data.remote

import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.ResponseBody.Companion.toResponseBody
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import retrofit2.HttpException
import retrofit2.Response

/**
 * Mirrors web's `isConflict`. The status, not the message text: the message is
 * authored in Rust and would be compared in Kotlin, which is one fact derived in
 * two languages with nothing keeping them in step.
 */
class HttpErrorsTest {

    private fun httpError(code: Int) = HttpException(
        Response.error<Any>(code, "".toResponseBody("application/json".toMediaTypeOrNull()))
    )

    @Test
    fun `a 409 is a conflict`() {
        assertTrue(isConflict(httpError(409)))
    }

    @Test
    fun `other HTTP errors are not conflicts`() {
        // 401 and 500 are real failures on the secure-add path; folding them in
        // would report a dropped photo as "already in that album".
        assertFalse(isConflict(httpError(401)))
        assertFalse(isConflict(httpError(500)))
    }

    @Test
    fun `a non-HTTP failure is not a conflict`() {
        // A socket timeout must never be read as "already there" — the add did
        // not happen and the user has to be told.
        assertFalse(isConflict(java.io.IOException("timeout")))
    }
}
