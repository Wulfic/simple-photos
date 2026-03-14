package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Repository for shared album operations.
 *
 * Thin wrapper around the 10 `/api/sharing/` Retrofit endpoints.
 * All methods are suspend-only (no local caching) — shared albums
 * are always fetched live from the server.
 */
@Singleton
class SharingRepository @Inject constructor(
    private val api: ApiService
) {
    // ── Albums ───────────────────────────────────────────────────────────

    /** List all shared albums the current user owns or is a member of. */
    suspend fun listAlbums(): List<SharedAlbumInfo> = api.listSharedAlbums()

    /** Create a new shared album. Returns the created album's info. */
    suspend fun createAlbum(name: String): CreateSharedAlbumResponse =
        api.createSharedAlbum(CreateSharedAlbumRequest(name))

    /** Delete a shared album (owner only). */
    suspend fun deleteAlbum(albumId: String) = api.deleteSharedAlbum(albumId)

    // ── Members ──────────────────────────────────────────────────────────

    /** List members of a shared album. */
    suspend fun listMembers(albumId: String): List<SharedAlbumMember> =
        api.listSharedAlbumMembers(albumId)

    /** Add a user to a shared album (owner only). */
    suspend fun addMember(albumId: String, userId: String): AddMemberResponse =
        api.addSharedAlbumMember(albumId, AddMemberRequest(userId))

    /** Remove a user from a shared album (owner only). */
    suspend fun removeMember(albumId: String, userId: String) =
        api.removeSharedAlbumMember(albumId, userId)

    // ── Photos ───────────────────────────────────────────────────────────

    /** List photos in a shared album. */
    suspend fun listPhotos(albumId: String): List<SharedAlbumPhoto> =
        api.listSharedAlbumPhotos(albumId)

    /** Add a photo to a shared album. */
    suspend fun addPhoto(albumId: String, photoRef: String, refType: String = "plain"): AddSharedPhotoResponse =
        api.addSharedAlbumPhoto(albumId, AddSharedPhotoRequest(photoRef, refType))

    /** Remove a photo from a shared album. */
    suspend fun removePhoto(albumId: String, photoId: String) =
        api.removeSharedAlbumPhoto(albumId, photoId)

    // ── User picker ──────────────────────────────────────────────────────

    /** List all users on this server (for member picker UI). */
    suspend fun listUsersForSharing(): List<ShareableUser> =
        api.listUsersForSharing()
}
