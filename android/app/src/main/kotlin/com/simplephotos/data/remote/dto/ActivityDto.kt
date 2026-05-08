/**
 * Activity / processing-status DTOs — combined progress info for upload,
 * conversion, encryption, AI, and geo pipelines.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class ActivityStatusResponse(
    @SerializedName("upload") val upload: ActivityUpload? = null,
    @SerializedName("conversion") val conversion: ActivityConversion? = null,
    @SerializedName("encryption") val encryption: ActivityEncryption? = null,
    @SerializedName("ai") val ai: ActivityAi? = null,
    @SerializedName("geo") val geo: ActivityGeo? = null,
    @SerializedName("ai_enabled") val aiEnabled: Boolean = false,
    @SerializedName("geo_enabled") val geoEnabled: Boolean = false,
)

data class ActivityUpload(
    val active: Boolean = false,
    val total: Int = 0,
    val done: Int = 0,
    @SerializedName("bytes_uploaded") val bytesUploaded: Long = 0,
    @SerializedName("bytes_total") val bytesTotal: Long = 0,
)

data class ActivityConversion(
    val active: Boolean = false,
    val total: Int = 0,
    val done: Int = 0,
    @SerializedName("current_filename") val currentFilename: String? = null,
)

data class ActivityEncryption(
    val active: Boolean = false,
    val total: Int = 0,
    val done: Int = 0,
)

data class ActivityAi(
    val active: Boolean = false,
    val total: Int = 0,
    val done: Int = 0,
    val stage: String? = null,
)

data class ActivityGeo(
    val active: Boolean = false,
    val total: Int = 0,
    val done: Int = 0,
)

data class TranscodeStatusResponse(
    val active: Boolean = false,
    val queue: Int = 0,
    val done: Int = 0,
    @SerializedName("gpu_available") val gpuAvailable: Boolean = false,
    val backend: String? = null,
)
