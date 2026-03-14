/**
 * Authentication request/response DTOs — login, registration, TOTP 2FA,
 * token refresh, and password management payloads.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class RegisterRequest(val username: String, val password: String)
data class RegisterResponse(@SerializedName("user_id") val userId: String, val username: String)

data class LoginRequest(val username: String, val password: String)
data class LoginResponse(
    @SerializedName("access_token") val accessToken: String? = null,
    @SerializedName("refresh_token") val refreshToken: String? = null,
    @SerializedName("expires_in") val expiresIn: Long? = null,
    @SerializedName("requires_totp") val requiresTotp: Boolean? = null,
    @SerializedName("totp_session_token") val totpSessionToken: String? = null
)

data class TotpLoginRequest(
    @SerializedName("totp_session_token") val totpSessionToken: String,
    @SerializedName("totp_code") val totpCode: String? = null,
    @SerializedName("backup_code") val backupCode: String? = null
)

data class RefreshRequest(@SerializedName("refresh_token") val refreshToken: String)
data class RefreshResponse(
    @SerializedName("access_token") val accessToken: String,
    @SerializedName("expires_in") val expiresIn: Long
)

data class LogoutRequest(@SerializedName("refresh_token") val refreshToken: String)

data class TotpSetupResponse(
    @SerializedName("otpauth_uri") val otpauthUri: String,
    @SerializedName("backup_codes") val backupCodes: List<String>
)

data class TotpConfirmRequest(@SerializedName("totp_code") val totpCode: String)
data class TotpDisableRequest(@SerializedName("totp_code") val totpCode: String)
