package com.simplephotos.data.remote

import com.simplephotos.data.remote.dto.*
import okhttp3.RequestBody
import okhttp3.ResponseBody
import retrofit2.Response
import retrofit2.http.*

interface ApiService {
    // Auth
    @POST("api/auth/register")
    suspend fun register(@Body request: RegisterRequest): RegisterResponse

    @POST("api/auth/login")
    suspend fun login(@Body request: LoginRequest): LoginResponse

    @POST("api/auth/login/totp")
    suspend fun loginTotp(@Body request: TotpLoginRequest): LoginResponse

    @POST("api/auth/refresh")
    suspend fun refresh(@Body request: RefreshRequest): RefreshResponse

    @POST("api/auth/logout")
    suspend fun logout(@Body request: LogoutRequest): Response<Unit>

    // 2FA
    @POST("api/auth/2fa/setup")
    suspend fun setup2fa(): TotpSetupResponse

    @POST("api/auth/2fa/confirm")
    suspend fun confirm2fa(@Body request: TotpConfirmRequest): Response<Unit>

    @POST("api/auth/2fa/disable")
    suspend fun disable2fa(@Body request: TotpDisableRequest): Response<Unit>

    // Blobs
    @POST("api/blobs")
    suspend fun uploadBlob(
        @Body body: RequestBody,
        @Header("X-Blob-Type") blobType: String,
        @Header("X-Blob-Size") blobSize: String,
        @Header("X-Client-Hash") clientHash: String? = null
    ): BlobUploadResponse

    @GET("api/blobs")
    suspend fun listBlobs(
        @Query("blob_type") blobType: String? = null,
        @Query("after") after: String? = null,
        @Query("limit") limit: Int? = null
    ): BlobListResponse

    @GET("api/blobs/{id}")
    @Streaming
    suspend fun downloadBlob(@Path("id") blobId: String): ResponseBody

    @DELETE("api/blobs/{id}")
    suspend fun deleteBlob(@Path("id") blobId: String): Response<Unit>

    // Health
    @GET("health")
    suspend fun health(): Map<String, String>
}
