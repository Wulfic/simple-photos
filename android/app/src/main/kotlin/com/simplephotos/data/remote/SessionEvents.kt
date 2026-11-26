/**
 * Process-wide "the server refused our refresh token" signal, from the OkHttp
 * authenticator to whatever owns navigation (N1c).
 */
package com.simplephotos.data.remote

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import javax.inject.Inject
import javax.inject.Singleton

/**
 * A [StateFlow] rather than an event stream, on purpose:
 *  - [report] is called under [SessionRefresher]'s lock on an OkHttp thread, and
 *    setting a value never suspends or blocks;
 *  - a report with no collector attached yet (no activity in the foreground) is
 *    still there when one attaches;
 *  - it is never consumed. Every window (#21) reads it and decides for itself by
 *    comparing the dead token with the stored one (`SessionPrefs.expire`), so a
 *    window opened after a re-login sees an old dead token and simply ignores it.
 */
@Singleton
class SessionEvents @Inject constructor() {
    private val _deadRefreshToken = MutableStateFlow<String?>(null)

    /** The most recent refresh token the server rejected; null if none has been. */
    val deadRefreshToken: StateFlow<String?> = _deadRefreshToken.asStateFlow()

    fun report(deadRefreshToken: String) {
        _deadRefreshToken.value = deadRefreshToken
    }
}
