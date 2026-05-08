/**
 * Setup wizard DTOs — discover info, setup status/init/finalize, pair
 * with existing primary, verify backup server.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class DiscoverInfoResponse(
    val name: String,
    val version: String,
    @SerializedName("setup_complete") val setupComplete: Boolean = false,
    @SerializedName("operating_mode") val operatingMode: String? = null,
    @SerializedName("registration_open") val registrationOpen: Boolean? = null,
)

data class SetupStatusResponse(
    @SerializedName("setup_complete") val setupComplete: Boolean,
    @SerializedName("registration_open") val registrationOpen: Boolean,
    val version: String,
)

data class SetupInitRequest(val username: String, val password: String)

data class SetupInitResponse(
    @SerializedName("user_id") val userId: String,
    val username: String,
    val message: String? = null,
)

data class SetupFinalizeRequest(
    @SerializedName("install_type") val installType: String? = null,
    @SerializedName("server_role") val serverRole: String? = null,
)

data class SetupFinalizeResponse(val message: String)

data class SetupDiscoverServer(
    val address: String,
    val name: String,
    val version: String,
)

data class SetupDiscoverResponse(
    val servers: List<SetupDiscoverServer>,
)

data class SetupPairRequest(
    @SerializedName("main_server_url") val mainServerUrl: String,
    val username: String,
    val password: String,
)

data class SetupPairResponse(
    val message: String? = null,
    @SerializedName("user_id") val userId: String,
    val username: String,
    @SerializedName("access_token") val accessToken: String,
    @SerializedName("refresh_token") val refreshToken: String,
    @SerializedName("main_server_url") val mainServerUrl: String,
)

data class VerifyBackupRequest(
    val address: String,
    val username: String,
    val password: String,
)

data class VerifyBackupResponse(
    val address: String,
    val name: String,
    val version: String,
    @SerializedName("api_key") val apiKey: String? = null,
    @SerializedName("photo_count") val photoCount: Long = 0,
)
