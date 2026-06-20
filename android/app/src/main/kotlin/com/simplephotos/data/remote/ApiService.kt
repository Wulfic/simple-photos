/**
 * Retrofit interface defining all server API endpoints.
 *
 * Each method maps to a server route under `/api/`. Authentication is handled
 * by the OkHttp interceptor in [com.simplephotos.di.NetworkModule] which
 * injects `Authorization: Bearer <token>` on every request and handles
 * transparent token refresh on 401 responses.
 */
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

    @POST("api/auth/verify-password")
    suspend fun verifyPassword(@Body request: VerifyPasswordRequest): Response<Unit>

    // ── 2FA ──────────────────────────────────────────────────────────────
    @GET("api/auth/2fa/status")
    suspend fun get2faStatus(): TwoFactorStatusResponse

    @POST("api/auth/2fa/setup")
    suspend fun setup2fa(): TotpSetupResponse

    @POST("api/auth/2fa/confirm")
    suspend fun confirm2fa(@Body request: TotpConfirmRequest): Response<Unit>

    @POST("api/auth/2fa/disable")
    suspend fun disable2fa(@Body request: TotpDisableRequest): Response<Unit>

    // ── Photos ───────────────────────────────────────────────────────────
    @GET("api/photos")
    suspend fun listPhotos(
        @Query("after") after: String? = null,
        @Query("limit") limit: Int? = null,
        @Query("media_type") mediaType: String? = null
    ): PhotoListResponse

    @GET("api/photos/encrypted-sync")
    suspend fun encryptedSync(
        @Query("after") after: String? = null,
        @Query("limit") limit: Int? = null
    ): EncryptedSyncResponse

    @GET("api/photos/crop-sync")
    suspend fun cropSync(): List<CropSyncRecord>

    @GET("api/photos/favorite-sync")
    suspend fun favoriteSync(): List<FavSyncRecord>

    @GET("api/photos/{id}/thumb")
    @Streaming
    suspend fun photoThumbnail(@Path("id") photoId: String): ResponseBody

    @GET("api/photos/{id}/file")
    @Streaming
    suspend fun photoFile(@Path("id") photoId: String): ResponseBody

    /**
     * Serve the embedded motion-photo video as a ready-to-play MP4. The server
     * resolves it by photo id + subtype=="motion": it serves a separately
     * stored motion blob if present, otherwise extracts the MP4 trailer from
     * the (server-side-decrypted) photo on the fly. Returns decrypted
     * `video/mp4` — no client-side decryption required. Mirrors the web's
     * `api.photos.motionVideoUrl`.
     */
    @GET("api/photos/{id}/motion-video")
    @Streaming
    suspend fun serveMotionVideo(@Path("id") photoId: String): ResponseBody

    @POST("api/photos/upload")
    suspend fun uploadPhoto(
        @Body body: RequestBody,
        @Header("X-Filename") filename: String,
        @Header("X-Mime-Type") mimeType: String
    ): PhotoUploadResponse

    @DELETE("api/photos/{id}")
    suspend fun deletePhoto(@Path("id") photoId: String): Response<Unit>

    /**
     * Trigger server-side timestamp-based burst grouping for the current user.
     * Called after a backup batch so bursts that carry no XMP BurstID (Samsung
     * et al.) get stacked once all their frames have been registered.
     */
    @POST("api/photos/detect-bursts")
    suspend fun detectBursts(): DetectBurstsResponse

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

    @GET("api/blobs/{id}/thumb")
    @Streaming
    suspend fun downloadThumbBlob(@Path("id") blobId: String): ResponseBody

    @DELETE("api/blobs/{id}")
    suspend fun deleteBlob(@Path("id") blobId: String): Response<Unit>

    @POST("api/blobs/{id}/trash")
    suspend fun softDeleteBlob(
        @Path("id") blobId: String,
        @Body request: SoftDeleteBlobRequest
    ): SoftDeleteBlobResponse

    // ── Register encrypted photo ─────────────────────────────────────────
    @POST("api/photos/register-encrypted")
    suspend fun registerEncryptedPhoto(
        @Body request: RegisterEncryptedPhotoRequest
    ): RegisterEncryptedPhotoResponse

    // ── Settings ─────────────────────────────────────────────────────────
    @GET("api/settings/storage-stats")
    suspend fun getStorageStats(): StorageStatsResponse

    // ── Encryption key (admin) ───────────────────────────────────────────
    @POST("api/admin/encryption/store-key")
    suspend fun storeEncryptionKey(@Body request: StoreEncryptionKeyRequest): StoreEncryptionKeyResponse


    // ── Backup servers (admin) ───────────────────────────────────────────
    @GET("api/admin/backup/servers")
    suspend fun listBackupServers(): BackupServerListResponse

    @POST("api/admin/backup/servers")
    suspend fun addBackupServer(@Body request: AddBackupServerRequest): BackupServer

    @POST("api/admin/backup/servers/{id}/recover")
    suspend fun recoverFromBackup(@Path("id") serverId: String): RecoverResponse

    // ── Audio Backup (admin) ─────────────────────────────────────────────
    @GET("api/settings/audio-backup")
    suspend fun getAudioBackupSetting(): AudioBackupResponse

    @PUT("api/admin/audio-backup")
    suspend fun setAudioBackupSetting(@Body request: SetAudioBackupRequest): AudioBackupResponse

    // ── SSL/TLS (admin) ──────────────────────────────────────────────────
    @GET("api/admin/ssl")
    suspend fun getSslStatus(): SslStatusResponse

    // ── Conversion / encryption status ───────────────────────────────────
    @GET("api/admin/conversion-status")
    suspend fun getConversionStatus(): ConversionStatusResponse

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

    @GET("api/trash/{id}/thumb")
    @Streaming
    suspend fun trashThumbnail(@Path("id") trashId: String): ResponseBody

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

    // ── Batch dimension update ────────────────────────────────────────────
    @PATCH("api/photos/dimensions")
    suspend fun batchUpdateDimensions(@Body request: BatchDimensionUpdateRequest): BatchDimensionUpdateResponse

    // ── Duplicate (Save Copy) ─────────────────────────────────────────────
    @POST("api/photos/{id}/duplicate")
    suspend fun duplicatePhoto(
        @Path("id") photoId: String,
        @Body request: DuplicatePhotoRequest
    ): DuplicatePhotoResponse

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

    // ── Shared Albums ────────────────────────────────────────────────────

    @GET("api/sharing/albums")
    suspend fun listSharedAlbums(): List<SharedAlbumInfo>

    @POST("api/sharing/albums")
    suspend fun createSharedAlbum(@Body request: CreateSharedAlbumRequest): CreateSharedAlbumResponse

    @DELETE("api/sharing/albums/{id}")
    suspend fun deleteSharedAlbum(@Path("id") albumId: String): retrofit2.Response<Unit>

    @GET("api/sharing/albums/{id}/members")
    suspend fun listSharedAlbumMembers(@Path("id") albumId: String): List<SharedAlbumMember>

    @POST("api/sharing/albums/{id}/members")
    suspend fun addSharedAlbumMember(
        @Path("id") albumId: String,
        @Body request: AddMemberRequest
    ): AddMemberResponse

    @DELETE("api/sharing/albums/{id}/members/{userId}")
    suspend fun removeSharedAlbumMember(
        @Path("id") albumId: String,
        @Path("userId") userId: String
    ): retrofit2.Response<Unit>

    @GET("api/sharing/albums/{id}/photos")
    suspend fun listSharedAlbumPhotos(@Path("id") albumId: String): List<SharedAlbumPhoto>

    @POST("api/sharing/albums/{id}/photos")
    suspend fun addSharedAlbumPhoto(
        @Path("id") albumId: String,
        @Body request: AddSharedPhotoRequest
    ): AddSharedPhotoResponse

    @DELETE("api/sharing/albums/{albumId}/photos/{photoId}")
    suspend fun removeSharedAlbumPhoto(
        @Path("albumId") albumId: String,
        @Path("photoId") photoId: String
    ): retrofit2.Response<Unit>

    @GET("api/sharing/users")
    suspend fun listUsersForSharing(): List<ShareableUser>

    // ── Diagnostics Config (admin) ───────────────────────────────────────
    @GET("api/admin/diagnostics/config")
    suspend fun getDiagnosticsConfig(): DiagnosticsConfigResponse

    @PUT("api/admin/diagnostics/config")
    suspend fun updateDiagnosticsConfig(@Body request: UpdateDiagnosticsConfigRequest): DiagnosticsConfigResponse

    // ── Diagnostics (admin) — full report ────────────────────────────────
    @GET("api/admin/diagnostics")
    suspend fun getDiagnostics(): DiagnosticsResponse

    // ── Audit logs (admin) ───────────────────────────────────────────────
    @GET("api/admin/audit-logs")
    suspend fun listAuditLogs(
        @Query("event_type") eventType: String? = null,
        @Query("user_id") userId: String? = null,
        @Query("ip_address") ipAddress: String? = null,
        @Query("after") after: String? = null,
        @Query("before") before: String? = null,
        @Query("limit") limit: Int? = null,
    ): AuditLogListResponse

    // ── AI ───────────────────────────────────────────────────────────────
    @GET("api/ai/status")
    suspend fun getAiStatus(): AiStatusResponse

    @POST("api/ai/toggle")
    suspend fun toggleAi(@Body request: AiToggleRequest): Response<Unit>

    @POST("api/ai/reprocess")
    suspend fun reprocessAi(@Body request: AiReprocessRequest): AiReprocessResponse

    @GET("api/ai/faces")
    suspend fun listFaceClusters(): List<FaceCluster>

    @POST("api/ai/faces/merge")
    suspend fun mergeFaceClusters(@Body request: FaceClusterMergeRequest): Response<Unit>

    @POST("api/ai/faces/split")
    suspend fun splitFaceCluster(@Body request: FaceClusterSplitRequest): Response<Unit>

    @GET("api/ai/faces/{cluster_id}/photos")
    suspend fun listFaceClusterPhotos(@Path("cluster_id") clusterId: String): List<FaceClusterPhotoEntry>

    @PUT("api/ai/faces/{cluster_id}/name")
    suspend fun renameFaceCluster(
        @Path("cluster_id") clusterId: String,
        @Body request: FaceClusterRenameRequest,
    ): Response<Unit>

    @GET("api/ai/objects")
    suspend fun listObjectClasses(): List<ObjectClass>

    @GET("api/ai/objects/{class_name}/photos")
    suspend fun listObjectClassPhotos(@Path("class_name") className: String): List<ObjectClassPhotoEntry>

    @GET("api/ai/pets")
    suspend fun listPetClusters(): List<PetCluster>

    @POST("api/ai/pets/merge")
    suspend fun mergePetClusters(@Body request: PetClusterMergeRequest): Response<Unit>

    @GET("api/ai/pets/{cluster_id}/photos")
    suspend fun listPetClusterPhotos(@Path("cluster_id") clusterId: String): List<PetClusterPhotoEntry>

    @PUT("api/ai/pets/{cluster_id}/name")
    suspend fun renamePetCluster(
        @Path("cluster_id") clusterId: String,
        @Body request: PetClusterRenameRequest,
    ): Response<Unit>

    // ── Geo ──────────────────────────────────────────────────────────────
    @GET("api/settings/geo")
    suspend fun getGeoSettings(): GeoSettings

    @POST("api/settings/geo")
    suspend fun updateGeoSettings(@Body request: UpdateGeoSettingsRequest): Response<Unit>

    @POST("api/geo/scrub")
    suspend fun scrubGeoData(@Body request: GeoScrubRequest): GeoScrubResponse

    @GET("api/geo/countries")
    suspend fun listGeoCountries(): List<GeoCountry>

    @GET("api/geo/locations")
    suspend fun listGeoLocations(): List<GeoLocation>

    @GET("api/geo/locations/{country}/{city}")
    suspend fun listGeoLocationPhotos(
        @Path("country") country: String,
        @Path("city") city: String,
    ): List<GeoPhotoSummary>

    @GET("api/geo/map")
    suspend fun listGeoMapPhotos(): List<GeoMapPhoto>

    @GET("api/geo/timeline")
    suspend fun listGeoTimeline(): List<GeoTimelineEntry>

    @GET("api/geo/timeline/{year}")
    suspend fun listGeoTimelineYear(@Path("year") year: Int): List<GeoTimelineEntry>

    @GET("api/geo/timeline/{year}/{month}")
    suspend fun listGeoTimelineMonthPhotos(
        @Path("year") year: Int,
        @Path("month") month: Int,
    ): List<GeoPhotoSummary>

    @GET("api/geo/memories")
    suspend fun listGeoMemories(): List<GeoMemory>

    @GET("api/geo/memories/{memory_id}/photos")
    suspend fun listGeoMemoryPhotos(@Path("memory_id") memoryId: String): List<GeoPhotoSummary>

    @GET("api/geo/trips")
    suspend fun listGeoTrips(): List<GeoTrip>

    @GET("api/geo/trips/{trip_id}/photos")
    suspend fun listGeoTripPhotos(@Path("trip_id") tripId: String): List<GeoPhotoSummary>

    // ── Activity / processing status ─────────────────────────────────────
    @GET("api/status/activity")
    suspend fun getActivityStatus(): ActivityStatusResponse

    @GET("api/transcode/status")
    suspend fun getTranscodeStatus(): TranscodeStatusResponse

    // ── Edit copies ──────────────────────────────────────────────────────
    @GET("api/photos/{id}/copies")
    suspend fun listEditCopies(@Path("id") photoId: String): EditCopyListResponse

    @POST("api/photos/{id}/copies")
    suspend fun createEditCopy(
        @Path("id") photoId: String,
        @Body request: CreateEditCopyRequest,
    ): CreateEditCopyResponse

    @DELETE("api/photos/{id}/copies/{copy_id}")
    suspend fun deleteEditCopy(
        @Path("id") photoId: String,
        @Path("copy_id") copyId: String,
    ): DeleteEditCopyResponse

    @POST("api/photos/{id}/render")
    @Streaming
    suspend fun renderPhoto(
        @Path("id") photoId: String,
        @Body request: RenderPhotoRequest,
    ): ResponseBody

    @GET("api/photos/{id}/source-file")
    @Streaming
    suspend fun photoSourceFile(@Path("id") photoId: String): ResponseBody

    @GET("api/photos/{id}/web")
    @Streaming
    suspend fun photoWebFile(@Path("id") photoId: String): ResponseBody

    // ── Photo metadata sidecars ──────────────────────────────────────────
    @POST("api/import/metadata")
    suspend fun importMetadata(@Body request: ImportMetadataRequest): ImportMetadataResponse

    @POST("api/import/metadata/batch")
    suspend fun importMetadataBatch(@Body request: ImportMetadataBatchRequest): ImportMetadataBatchResponse

    @POST("api/import/metadata/upload")
    suspend fun importMetadataUpload(
        @Body body: RequestBody,
        @Header("X-Photo-Id") photoId: String? = null,
        @Header("X-Blob-Id") blobId: String? = null,
    ): ImportMetadataResponse

    @GET("api/photos/{id}/metadata")
    suspend fun listPhotoMetadata(@Path("id") photoId: String): PhotoMetadataListResponse

    @DELETE("api/photos/{id}/metadata")
    suspend fun deletePhotoMetadata(@Path("id") photoId: String): Response<Unit>

    // ── Export ───────────────────────────────────────────────────────────
    @POST("api/export")
    suspend fun startExport(@Body request: ExportStartRequest): ExportStartResponse

    @GET("api/export/status")
    suspend fun getExportStatus(): ExportStatusResponse

    @GET("api/export/files")
    suspend fun listExportFiles(): ExportFileListResponse

    @GET("api/export/files/{id}/download")
    @Streaming
    suspend fun downloadExportFile(@Path("id") fileId: String): ResponseBody

    // ── Setup wizard ─────────────────────────────────────────────────────
    @GET("api/discover/info")
    suspend fun discoverInfo(): DiscoverInfoResponse

    @GET("api/setup/status")
    suspend fun getSetupStatus(): SetupStatusResponse

    @POST("api/setup/init")
    suspend fun setupInit(@Body request: SetupInitRequest): SetupInitResponse

    @POST("api/setup/finalize")
    suspend fun setupFinalize(@Body request: SetupFinalizeRequest): SetupFinalizeResponse

    @GET("api/setup/discover")
    suspend fun setupDiscover(): SetupDiscoverResponse

    @POST("api/setup/pair")
    suspend fun setupPair(@Body request: SetupPairRequest): SetupPairResponse

    @POST("api/setup/verify-backup")
    suspend fun setupVerifyBackup(@Body request: VerifyBackupRequest): VerifyBackupResponse

    // ── Admin server controls ────────────────────────────────────────────
    @GET("api/admin/storage")
    suspend fun getStoragePath(): StoragePathResponse

    @PUT("api/admin/storage")
    suspend fun updateStoragePath(@Body request: UpdateStoragePathRequest): StoragePathResponse

    @GET("api/admin/browse")
    suspend fun browseDirectory(@Query("path") path: String? = null): BrowseResponse

    @GET("api/admin/port")
    suspend fun getServerPort(): PortResponse

    @PUT("api/admin/port")
    suspend fun updateServerPort(@Body request: UpdatePortRequest): PortResponse

    @POST("api/admin/restart")
    suspend fun restartServer(): RestartResponse

    @PUT("api/admin/ssl")
    suspend fun updateSslConfig(@Body request: UpdateSslRequest): SslStatusResponse

    @GET("api/admin/ssl/local-ca/bundle")
    @Streaming
    suspend fun downloadLocalCaBundle(): ResponseBody

    @POST("api/admin/photos/auto-scan")
    suspend fun autoScanPhotos(): AutoScanResponse

    // ── Server-side import (admin) ───────────────────────────────────────
    @GET("api/admin/import/scan")
    suspend fun adminImportScan(@Query("path") path: String? = null): ImportScanResponse

    @GET("api/admin/import/file")
    @Streaming
    suspend fun adminImportFile(@Query("path") path: String): ResponseBody

    @GET("api/admin/import/google-photos/scan")
    suspend fun adminGooglePhotosScan(@Query("path") path: String): GooglePhotosScanResponse

    @POST("api/admin/import/google-photos")
    suspend fun adminGooglePhotosImport(@Body request: GooglePhotosImportRequest): GooglePhotosImportResponse

    // ── Admin 2FA management ─────────────────────────────────────────────
    @POST("api/admin/users/{id}/2fa/setup")
    suspend fun adminSetup2fa(@Path("id") userId: String): TotpSetupResponse

    @POST("api/admin/users/{id}/2fa/confirm")
    suspend fun adminConfirm2fa(
        @Path("id") userId: String,
        @Body request: TotpConfirmRequest,
    ): MessageResponse

    // ── Backup mode + extended backup-server admin ───────────────────────
    @GET("api/admin/backup/mode")
    suspend fun getBackupMode(): BackupModeResponse

    @POST("api/admin/backup/mode")
    suspend fun setBackupMode(@Body request: SetBackupModeRequest): BackupModeResponse

    @PUT("api/admin/backup/servers/{id}")
    suspend fun updateBackupServer(
        @Path("id") serverId: String,
        @Body request: UpdateBackupServerRequest,
    ): MessageResponse

    @DELETE("api/admin/backup/servers/{id}")
    suspend fun deleteBackupServer(@Path("id") serverId: String): Response<Unit>

    @GET("api/admin/backup/servers/{id}/status")
    suspend fun backupServerStatus(@Path("id") serverId: String): BackupServerStatusResponse

    @GET("api/admin/backup/servers/{id}/logs")
    suspend fun backupServerLogs(@Path("id") serverId: String): List<BackupSyncLog>

    @POST("api/admin/backup/servers/{id}/sync")
    suspend fun triggerBackupSync(@Path("id") serverId: String): BackupSyncStartedResponse

    @GET("api/admin/backup/discover")
    suspend fun discoverBackupServers(): BackupDiscoverResponse

    // ── Photo register (non-encrypted) ───────────────────────────────────
    @POST("api/photos/register")
    suspend fun registerPhoto(@Body request: RegisterPhotoRequest): RegisterPhotoResponse

    // ── Secure gallery item delete ───────────────────────────────────────
    @DELETE("api/galleries/secure/{id}/items/{item_id}")
    suspend fun deleteSecureGalleryItem(
        @Path("id") galleryId: String,
        @Path("item_id") itemId: String,
    ): Response<Unit>

    // ── External diagnostics (Basic auth) ────────────────────────────────
    // Not declared here; called via separate ApiService instance with Basic
    // auth interceptor when admin enables external diagnostics access.
}
