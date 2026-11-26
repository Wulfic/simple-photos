/**
 * The decision behind the OkHttp 401 authenticator: given a request that got a
 * 401, return the access token to retry it with, or null to let the 401 stand.
 */
package com.simplephotos.data.remote

import android.util.Log
import com.simplephotos.data.remote.dto.RefreshResponse
import retrofit2.HttpException

/**
 * Pure (token I/O, network and clock are injected) so the refresh policy is
 * testable on the JVM. Every method that touches state is `@Synchronized`: a
 * thumbnail grid produces dozens of concurrent 401s, and the server rotates the
 * refresh token on use, so exactly one of them may call `/auth/refresh`.
 *
 * [onSessionDead] runs once per refresh token the server rejects with 401, with
 * that token, on an OkHttp thread while the lock is held — it must only post a
 * signal, never block.
 */
class SessionRefresher(
    private val readTokens: () -> Tokens,
    private val callRefresh: (refreshToken: String) -> RefreshResponse,
    private val saveTokens: (access: String, refresh: String?) -> Unit,
    private val now: () -> Long = System::currentTimeMillis,
    private val onSessionDead: (deadRefreshToken: String) -> Unit = {},
) {
    data class Tokens(val access: String?, val refresh: String?)

    /**
     * @param requestAccess the access token the failed request was sent with.
     * @return the access token to retry with, or null to give up.
     */
    @Synchronized
    fun authenticate(requestAccess: String?): String? {
        val tokens = readTokens()

        // Another request already refreshed while we waited on the lock. Retry
        // with the fresh token — refreshing again would replay a rotated token.
        if (tokens.access != null && tokens.access != requestAccess) return tokens.access

        val refreshToken = tokens.refresh ?: return null

        // The server already said no to this exact token. Asking again is what
        // turned one dead session into 155 refresh calls in 22 s (N1).
        if (refreshToken == deadRefreshToken) return null

        // A transient failure moments ago: every queued 401 retrying it is a
        // storm, and against a timing-out server each one holds a thread ~10 s.
        if (now() < cooldownUntil) return null

        return try {
            val response = callRefresh(refreshToken)
            // Persist BOTH rotated tokens. Keeping the old refresh token would
            // make the next refresh replay a revoked one.
            saveTokens(response.accessToken, response.refreshToken)
            cooldownUntil = 0L
            response.accessToken
        } catch (e: HttpException) {
            if (e.code() == 401) {
                // Definitive: unknown/expired, or revoked as theft. Only a login
                // replaces this token, so stop spending requests on it.
                Log.w(TAG, "Refresh token rejected (401) — session is dead until re-login")
                deadRefreshToken = refreshToken
                onSessionDead(refreshToken)
            } else {
                coolDown(retryAfterMs(e))
                Log.w(TAG, "Token refresh failed: HTTP ${e.code()}; tokens kept, retry after cooldown")
            }
            null
        } catch (e: Exception) {
            // IO / timeout / unparseable body: the server may have rotated and we
            // lost the answer — the server's lost-response recovery (N1a) makes
            // replaying this same token later safe, so keep it.
            coolDown(DEFAULT_COOLDOWN_MS)
            Log.w(TAG, "Token refresh failed: ${e.javaClass.simpleName}; tokens kept, retry after cooldown")
            null
        }
    }

    /** The refresh token that got a 401; null when none has. */
    private var deadRefreshToken: String? = null

    /** Epoch ms before which a 401 does not call `/auth/refresh`. */
    private var cooldownUntil = 0L

    private fun coolDown(ms: Long) {
        cooldownUntil = now() + ms
    }

    private fun retryAfterMs(e: HttpException): Long {
        val seconds = e.response()?.headers()?.get("Retry-After")?.trim()?.toLongOrNull()
            ?: return DEFAULT_COOLDOWN_MS
        return (seconds * 1000).coerceIn(MIN_COOLDOWN_MS, MAX_COOLDOWN_MS)
    }

    companion object {
        private const val TAG = "SessionRefresher"
        const val DEFAULT_COOLDOWN_MS = 10_000L
        const val MIN_COOLDOWN_MS = 5_000L
        const val MAX_COOLDOWN_MS = 60_000L
    }
}
