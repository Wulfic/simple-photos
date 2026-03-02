package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Client Diagnostic Logs ───────────────────────────────────────────────────

data class ClientLogEntry(
    val level: String,
    val tag: String,
    val message: String,
    val context: Map<String, String>? = null,
    @SerializedName("client_ts") val clientTs: String
)

data class ClientLogBatch(
    @SerializedName("session_id") val sessionId: String,
    val entries: List<ClientLogEntry>
)

// ── Diagnostics Configuration ────────────────────────────────────────────────

data class DiagnosticsConfigResponse(
    @SerializedName("diagnostics_enabled") val diagnosticsEnabled: Boolean,
    @SerializedName("client_diagnostics_enabled") val clientDiagnosticsEnabled: Boolean
)

data class UpdateDiagnosticsConfigRequest(
    @SerializedName("diagnostics_enabled") val diagnosticsEnabled: Boolean? = null,
    @SerializedName("client_diagnostics_enabled") val clientDiagnosticsEnabled: Boolean? = null
)
