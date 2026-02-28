package com.simplephotos.data.remote

import com.simplephotos.data.remote.dto.*
import okhttp3.RequestBody
import okhttp3.ResponseBody
import retrofit2.Response
import retrofit2.http.*

interface ApiService {
    // ── Auth ──────────────────────────────────────────────────────────────
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

    @PUT("api/auth/password")
    suspend fun changePassword(@Body request: ChangePasswordRequest): Response<Unit>

    // ── 2FA ──────────────────────────────────────────────────────────────
    @POST("api/auth/2fa/setup")
    suspend fun setup2fa(): TotpSetupResponse

    @POST("api/auth/2fa/confirm")
    suspend fun confirm2fa(@Body request: TotpConfirmRequest): Response<Unit>

    @POST("api/auth/2fa/disable")
    suspend fun disable2fa(@Body request: TotpDisableRequest): Response<Unit>

    // ── Plain-mode Photos ────────────────────────────────────────────────
    @GET("api/photos")
    suspend fun listPhotos(
        @Query("after") after: String? = null,
        @Query("limit") limit: Int? = null,
        @Query("media_type") mediaType: String? = null
    ): PlainPhotoListResponse

    @GET("api/photos/{id}/thumb")
    @Streaming
    suspend fun photoThumbnail(@Path("id") photoId: String): ResponseBody

    @GET("api/photos/{id}/file")
    @Streaming
    suspend fun photoFile(@Path("id") photoId: String): ResponseBody

    @POST("api/photos/upload")
    suspend fun uploadPhoto(
        @Body body: RequestBody,
        @Header("X-Filename") filename: String,
        @Header("X-Mime-Type") mimeType: String
    ): PhotoUploadResponse

    @DELETE("api/photos/{id}")
    suspend fun deletePhoto(@Path("id") photoId: String): Response<Unit>

    // ── Encrypted Blobs ──────────────────────────────────────────────────
    @POST("api/blobs")
    suspend fun uploadBlob(
        @Body body: RequestBody,
        @Header("X-Blob-Type") blobType: String,
        @Header("X-Blob-Size") blobSize: String,
        @Header("X-Client-Hash") clientHash: String? = null,
        @Header("X-Content-Hash") contentHash: String? = null
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

    // ── Settings ─────────────────────────────────────────────────────────
    @GET("api/settings/encryption")
    suspend fun getEncryptionSettings(): EncryptionSettingsResponse

    @GET("api/settings/storage-stats")
    suspend fun getStorageStats(): StorageStatsResponse

    // ── Admin ────────────────────────────────────────────────────────────
    @POST("api/admin/photos/scan")
    suspend fun scanAndRegister(): ScanResponse

    @GET("api/admin/users")
    suspend fun listUsers(): List<AdminUser>

    @POST("api/admin/users")
    suspend fun createUser(@Body request: CreateUserRequest): CreateUserResponse

    @DELETE("api/admin/users/{id}")
    suspend fun deleteUser(@Path("id") userId: String): Response<Unit>

    @PUT("api/admin/users/{id}/role")
    suspend fun updateUserRole(
        @Path("id") userId: String,
        @Body request: UpdateRoleRequest
    ): UpdateRoleResponse

    @PUT("api/admin/users/{id}/password")
    suspend fun resetUserPassword(
        @Path("id") userId: String,
        @Body request: ResetPasswordRequest
    ): MessageResponse

    @DELETE("api/admin/users/{id}/2fa")
    suspend fun resetUser2fa(@Path("id") userId: String): MessageResponse

    // ── Trash ─────────────────────────────────────────────────────────────
    @GET("api/trash")
    suspend fun listTrash(
        @Query("after") after: String? = null,
        @Query("limit") limit: Int? = null
    ): TrashListResponse

    @HTTP(method = "DELETE", path = "api/trash", hasBody = false)
    suspend fun emptyTrash(): Response<Unit>

    @HTTP(method = "DELETE", path = "api/trash/{id}", hasBody = false)
    suspend fun permanentDeleteTrash(@Path("id") id: String): Response<Unit>

    @POST("api/trash/{id}/restore")
    suspend fun restoreFromTrash(@Path("id") id: String): Response<Unit>

    // ── Health ────────────────────────────────────────────────────────────
    @GET("health")
    suspend fun health(): Map<String, String>

    // ── Tags ──────────────────────────────────────────────────────────────
    @GET("api/tags")
    suspend fun listTags(): TagListResponse

    @GET("api/photos/{id}/tags")
    suspend fun getPhotoTags(@Path("id") photoId: String): PhotoTagsResponse

    @POST("api/photos/{id}/tags")
    suspend fun addTag(@Path("id") photoId: String, @Body request: AddTagRequest): Response<Unit>

    @HTTP(method = "DELETE", path = "api/photos/{id}/tags", hasBody = true)
    suspend fun removeTag(@Path("id") photoId: String, @Body request: RemoveTagRequest): Response<Unit>

    // ── Search ────────────────────────────────────────────────────────────
    @GET("api/search")
    suspend fun searchPhotos(
        @Query("q") query: String,
        @Query("limit") limit: Int? = null
    ): SearchResponse

    // ── Favorites ─────────────────────────────────────────────────────────
    @PUT("api/photos/{id}/favorite")
    suspend fun toggleFavorite(@Path("id") photoId: String): FavoriteToggleResponse

    // ── Crop Metadata ─────────────────────────────────────────────────────
    @PUT("api/photos/{id}/crop")
    suspend fun setCrop(@Path("id") photoId: String, @Body request: SetCropRequest): CropResponse

    // ── Secure Galleries ──────────────────────────────────────────────────
    @GET("api/galleries/secure")
    suspend fun listSecureGalleries(): SecureGalleryListResponse

    @POST("api/galleries/secure")
    suspend fun createSecureGallery(@Body request: SecureGalleryCreateRequest): SecureGalleryCreateResponse

    @DELETE("api/galleries/secure/{id}")
    suspend fun deleteSecureGallery(@Path("id") galleryId: String): retrofit2.Response<Unit>

    @POST("api/galleries/secure/unlock")
    suspend fun unlockSecureGalleries(@Body request: SecureGalleryUnlockRequest): SecureGalleryUnlockResponse

    @GET("api/galleries/secure/{id}/items")
    suspend fun listSecureGalleryItems(
        @Path("id") galleryId: String,
        @Header("X-Gallery-Token") galleryToken: String
    ): SecureGalleryItemsResponse

    @POST("api/galleries/secure/{id}/items")
    suspend fun addSecureGalleryItem(
        @Path("id") galleryId: String,
        @Body request: SecureGalleryAddItemRequest
    ): SecureGalleryAddItemResponse

    @GET("api/galleries/secure/blob-ids")
    suspend fun getSecureBlobIds(): SecureBlobIdsResponse

    // ── Client Diagnostic Logs ───────────────────────────────────────────
    @POST("api/client-logs")
    suspend fun submitClientLogs(@Body batch: ClientLogBatch): Map<String, Any>
}
