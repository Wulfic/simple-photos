/**
 * Global uncaught-exception capture.
 *
 * Historically the app had **no** crash capture at all: the only diagnostics
 * came from the [DiagnosticLogger] buffer inside [com.simplephotos.sync.BackupWorker]'s
 * try/catch, so any crash on the UI thread (or any thread outside that worker)
 * died silently — leaving "the app keeps stopping" completely undiagnosable.
 *
 * [CrashHandler] installs a process-wide [Thread.UncaughtExceptionHandler] that:
 *  1. Writes the full stack trace + device/app metadata to a local file
 *     **synchronously** before the process dies (network I/O mid-crash is
 *     unreliable — the looper may already be torn down).
 *  2. Chains to the previously-installed handler (usually the Android default,
 *     which shows the "app has stopped" dialog and kills the process), so we
 *     never swallow the crash or change its user-visible behaviour.
 *
 * On the next successful launch, [uploadPendingCrashes] drains the crash-log
 * directory to the server via [ApiService.submitClientLogs] and deletes the
 * files. This is best-effort and must never itself crash the app.
 */
package com.simplephotos.sync

import android.content.Context
import android.os.Build
import android.util.Log
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.ClientLogBatch
import com.simplephotos.data.remote.dto.ClientLogEntry
import java.io.File
import java.io.PrintWriter
import java.io.StringWriter
import java.time.Instant
import java.util.UUID

object CrashHandler {

    private const val TAG = "CrashHandler"
    private const val CRASH_DIR = "crashlogs"
    private const val MAX_STORED_CRASHES = 20

    @Volatile
    private var installed = false

    /**
     * Install the global uncaught-exception handler. Idempotent — safe to call
     * from [android.app.Application.onCreate]. Never throws.
     */
    fun install(context: Context) {
        if (installed) return
        installed = true

        val appContext = context.applicationContext
        val previous = Thread.getDefaultUncaughtExceptionHandler()

        Thread.setDefaultUncaughtExceptionHandler { thread, throwable ->
            try {
                writeCrashFile(appContext, thread, throwable)
            } catch (t: Throwable) {
                // A failure to persist the crash must not mask the original
                // crash — log locally and fall through to the default handler.
                Log.e(TAG, "Failed to persist crash log", t)
            } finally {
                // Preserve default behaviour: show the system dialog and kill
                // the process. Without this the app would hang in a zombie state.
                previous?.uncaughtException(thread, throwable)
            }
        }
        Log.i(TAG, "Global uncaught-exception handler installed")
    }

    private fun crashDir(context: Context): File =
        File(context.filesDir, CRASH_DIR).apply { mkdirs() }

    private fun writeCrashFile(context: Context, thread: Thread, throwable: Throwable) {
        val stack = StringWriter().also { sw ->
            PrintWriter(sw).use { throwable.printStackTrace(it) }
        }.toString()

        val ts = Instant.now().toString()
        val body = buildString {
            appendLine("timestamp=$ts")
            appendLine("thread=${thread.name}")
            appendLine("device=${Build.MANUFACTURER} ${Build.MODEL}")
            appendLine("androidSdk=${Build.VERSION.SDK_INT}")
            appendLine("release=${Build.VERSION.RELEASE}")
            appendLine("----- STACK -----")
            append(stack)
        }

        val dir = crashDir(context)
        // Bound the directory so a fast crash loop can't fill storage.
        prune(dir)
        val file = File(dir, "crash-${System.currentTimeMillis()}-${UUID.randomUUID()}.txt")
        file.writeText(body)
        Log.e(TAG, "Captured uncaught exception to ${file.absolutePath}")
    }

    private fun prune(dir: File) {
        val files = dir.listFiles()?.sortedBy { it.lastModified() } ?: return
        val excess = files.size - (MAX_STORED_CRASHES - 1)
        if (excess > 0) {
            files.take(excess).forEach { runCatching { it.delete() } }
        }
    }

    /**
     * Best-effort: upload any crash files captured on previous runs, then delete
     * them. Call from a background coroutine after the app is up and authenticated.
     * Never throws.
     */
    suspend fun uploadPendingCrashes(context: Context, api: ApiService) {
        val dir = File(context.filesDir, CRASH_DIR)
        val files = dir.listFiles()?.filter { it.isFile } ?: return
        if (files.isEmpty()) return

        val entries = files.mapNotNull { file ->
            runCatching {
                ClientLogEntry(
                    level = "error",
                    tag = "AndroidCrash",
                    message = "Uncaught exception (captured ${Instant.ofEpochMilli(file.lastModified())})",
                    context = mapOf("stack" to file.readText().take(8000)),
                    clientTs = Instant.now().toString()
                )
            }.getOrNull()
        }
        if (entries.isEmpty()) return

        try {
            api.submitClientLogs(ClientLogBatch(sessionId = UUID.randomUUID().toString(), entries = entries))
            files.forEach { runCatching { it.delete() } }
            Log.i(TAG, "Uploaded and cleared ${files.size} pending crash log(s)")
        } catch (e: Exception) {
            // Leave the files in place to retry on the next launch.
            Log.w(TAG, "Failed to upload pending crash logs: ${e.message}")
        }
    }
}
