/**
 * Reading meaning out of an HTTP status, for the cases where a non-2xx response
 * is not a failure.
 *
 * Mirrors `web/src/api/core.ts`'s `isConflict`. A 409 from a secure add means
 * "already in that album" — a no-op the caller reports as such, not an error to
 * show the user. The alternative was matching on the message text, a string
 * authored in Rust and compared in Kotlin: one fact derived in two languages
 * with nothing keeping them in step.
 */
package com.simplephotos.data.remote

import retrofit2.HttpException

/** True when [e] is an HTTP error carrying [status]. */
fun isHttpStatus(e: Throwable, status: Int): Boolean = e is HttpException && e.code() == status

/** True when [e] is a 409 Conflict — "this already exists", not a failure. */
fun isConflict(e: Throwable): Boolean = isHttpStatus(e, 409)
