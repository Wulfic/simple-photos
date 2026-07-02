/**
 * Reports this device's queued-upload count to the server so the unified
 * encryption banner total reflects local backup work the server can't see yet
 * (TODO #2). The server namespaces contributions under the authenticated user
 * and expires them after ~90 s, so a stale/offline device can't inflate the
 * total indefinitely.
 */
package com.simplephotos.sync

import android.content.Context
import android.provider.Settings
import android.util.Log
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.EncryptionContributeRequest

object EncryptionContribution {
    private const val TAG = "EncryptionContribution"

    /**
     * Stable per-install source id. `ANDROID_ID` is scoped to the app signing
     * key + device, which is exactly the granularity we want: one contribution
     * slot per device. Prefixed so the server-side debug breakdown reads clearly.
     */
    fun sourceId(context: Context): String {
        val androidId = try {
            Settings.Secure.getString(context.contentResolver, Settings.Secure.ANDROID_ID)
        } catch (_: Exception) {
            null
        }
        return "android-" + (androidId?.takeIf { it.isNotBlank() } ?: "unknown")
    }

    /**
     * Best-effort report of [pending] queued items. Never throws — a failed
     * status report must never disrupt uploads or the UI. Pass 0 to clear this
     * device's contribution once its queue drains.
     */
    suspend fun report(api: ApiService, context: Context, pending: Int) {
        try {
            api.contributeEncryption(
                EncryptionContributeRequest(source = sourceId(context), pending = pending)
            )
        } catch (e: Exception) {
            Log.d(TAG, "contribute report failed (non-fatal): ${e.message}")
        }
    }
}
