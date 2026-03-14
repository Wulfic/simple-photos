/**
 * Repository providing diagnostic logging and server health-check capabilities.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.ClientLogBatch
import com.simplephotos.sync.DiagnosticLogger
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Repository providing diagnostic logging and server health-check capabilities.
 *
 * Centralises access to the server health endpoint and [DiagnosticLogger]
 * creation so ViewModels don't need to inject [ApiService] directly for
 * diagnostic purposes.
 */
@Singleton
class DiagnosticRepository @Inject constructor(
    private val api: ApiService
) {
    /** Create a [DiagnosticLogger] session for structured client-side log collection. */
    fun createLogger(enabled: Boolean): DiagnosticLogger =
        DiagnosticLogger(api, enabled)

    /** Query `GET /health` — returns a map of component → status strings. */
    suspend fun getHealthInfo(): Map<String, String> =
        api.health()
}
