/**
 * Buffered diagnostic logger that collects structured log entries during
 * backup and flushes them to the server.
 */
package com.simplephotos.sync

import android.util.Log
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.ClientLogBatch
import com.simplephotos.data.remote.dto.ClientLogEntry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.time.Instant
import java.util.UUID
import java.util.concurrent.CopyOnWriteArrayList

/**
 * Collects structured diagnostic log entries during a session and flushes them
 * to the server. Entries are sent in batches to `POST /api/client-logs`.
 *
 * Flushing happens on two triggers:
 *  1. **Threshold** — once the buffer reaches [FLUSH_THRESHOLD] entries an
 *     async flush is kicked off automatically. This is what keeps a long
 *     backup session from buffering hundreds of entries and only delivering
 *     them at the very end (the "Android logs are scarce" report): during a
 *     library backup, entries now stream to the server as the work progresses.
 *  2. **Explicit** — call [flush] at the end of a session to drain whatever
 *     remains (e.g. in `doWork`'s finally block).
 *
 * Usage:
 *   val logger = DiagnosticLogger(api)
 *   logger.info("BackupWorker", "Starting backup")
 *   logger.error("BackupWorker", "Upload failed", mapOf("photoId" to id, "error" to msg))
 *   logger.flush()   // sends any remaining buffered entries
 *
 * All flushes are best-effort — a failure (network error, auth expired, etc.)
 * drops the affected entries and is logged locally, but never throws.
 * Diagnostic logging must never interfere with the actual backup flow.
 */
class DiagnosticLogger(
    private val api: ApiService,
    private val enabled: Boolean = true,
    // Injectable so tests can supply a controllable scope; defaults to a
    // self-contained IO scope. Threshold flushes launch here and complete on
    // their own (one network round-trip each) — no long-lived timer is kept,
    // so short-lived loggers (e.g. one-shot diagnostic reports) don't leak.
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO),
) {

    companion object {
        private const val TAG = "DiagnosticLogger"
        private const val MAX_ENTRIES = 500
        /** Auto-flush once the buffer reaches this many entries. */
        private const val FLUSH_THRESHOLD = 25
    }

    val sessionId: String = UUID.randomUUID().toString()
    private val entries = CopyOnWriteArrayList<ClientLogEntry>()
    // Serialises concurrent flushes (threshold-triggered + explicit) so a batch
    // is never sent twice and the drain-then-send window can't lose entries.
    private val flushMutex = Mutex()

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

        // Stream to the server as work progresses instead of only at the end.
        if (entries.size >= FLUSH_THRESHOLD) {
            scope.launch { flush() }
        }
    }

    /**
     * Send buffered entries to the server. Safe to call repeatedly and
     * concurrently — flushes are serialised, and entries are drained before the
     * network call so anything logged during the send stays queued for next time.
     *
     * Best-effort — failures are logged locally but never thrown.
     */
    suspend fun flush() {
        flushMutex.withLock {
            // Snapshot then remove exactly what we're sending, so entries added
            // concurrently (during the network call) survive for the next flush.
            val snapshot = entries.toList()
            if (snapshot.isEmpty()) return
            entries.removeAll(snapshot)

            val batch = ClientLogBatch(sessionId = sessionId, entries = snapshot)
            try {
                api.submitClientLogs(batch)
                Log.i(TAG, "Flushed ${snapshot.size} diagnostic log entries (session=$sessionId)")
            } catch (e: Exception) {
                Log.w(TAG, "Failed to flush diagnostic logs: ${e.message}")
            }
        }
    }
}
