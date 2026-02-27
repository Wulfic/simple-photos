package com.simplephotos.sync

import android.util.Log
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.ClientLogBatch
import com.simplephotos.data.remote.dto.ClientLogEntry
import java.time.Instant
import java.util.UUID
import java.util.concurrent.CopyOnWriteArrayList

/**
 * Collects structured diagnostic log entries during a backup session and
 * flushes them to the server in a single batch at the end.
 *
 * Usage:
 *   val logger = DiagnosticLogger(api)
 *   logger.info("BackupWorker", "Starting backup")
 *   logger.error("BackupWorker", "Upload failed", mapOf("photoId" to id, "error" to msg))
 *   logger.flush()   // sends all buffered entries to POST /api/client-logs
 *
 * If the flush fails (network error, auth expired, etc.) the entries are
 * simply discarded — diagnostic logging must never interfere with the
 * actual backup flow.
 */
class DiagnosticLogger(private val api: ApiService, private val enabled: Boolean = true) {

    companion object {
        private const val TAG = "DiagnosticLogger"
        private const val MAX_ENTRIES = 500
    }

    val sessionId: String = UUID.randomUUID().toString()
    private val entries = CopyOnWriteArrayList<ClientLogEntry>()

    fun debug(tag: String, message: String, context: Map<String, String>? = null) =
        add("debug", tag, message, context)

    fun info(tag: String, message: String, context: Map<String, String>? = null) =
        add("info", tag, message, context)

    fun warn(tag: String, message: String, context: Map<String, String>? = null) =
        add("warn", tag, message, context)

    fun error(tag: String, message: String, context: Map<String, String>? = null) =
        add("error", tag, message, context)

    private fun add(level: String, tag: String, message: String, context: Map<String, String>?) {
        // Always log locally so logcat works during development
        when (level) {
            "debug" -> Log.d(tag, message)
            "info"  -> Log.i(tag, message)
            "warn"  -> Log.w(tag, message)
            "error" -> Log.e(tag, message)
        }

        // Only buffer for server if diagnostic logging is enabled
        if (!enabled) return
        if (entries.size >= MAX_ENTRIES) return // cap to avoid OOM

        entries.add(
            ClientLogEntry(
                level = level,
                tag = tag,
                message = message,
                context = context,
                clientTs = Instant.now().toString()
            )
        )
    }

    /**
     * Send all buffered entries to the server. Call this at the end of
     * a backup session (in doWork's finally block, for example).
     *
     * This is best-effort — failures are logged locally but never thrown.
     */
    suspend fun flush() {
        if (entries.isEmpty()) return

        val batch = ClientLogBatch(
            sessionId = sessionId,
            entries = entries.toList()
        )

        try {
            api.submitClientLogs(batch)
            Log.i(TAG, "Flushed ${entries.size} diagnostic log entries (session=$sessionId)")
        } catch (e: Exception) {
            Log.w(TAG, "Failed to flush diagnostic logs: ${e.message}")
        } finally {
            entries.clear()
        }
    }
}
