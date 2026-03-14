package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.AddTagRequest
import com.simplephotos.data.remote.dto.PhotoTagsResponse
import com.simplephotos.data.remote.dto.RemoveTagRequest
import com.simplephotos.data.remote.dto.TagListResponse
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Repository for photo tag operations.
 *
 * Tags are server-side only (plain mode) — they attach user-defined labels
 * to photos for filtering and search. In encrypted mode the server has no
 * photo metadata, so tags are not available.
 */
@Singleton
class TagRepository @Inject constructor(
    private val api: ApiService
) {
    /** Fetch all tags attached to a specific photo. */
    suspend fun getPhotoTags(photoId: String): PhotoTagsResponse =
        api.getPhotoTags(photoId)

    /** Fetch the global list of all user-created tags. */
    suspend fun listTags(): TagListResponse =
        api.listTags()

    /** Add a tag to a photo. The tag string should already be sanitised by the caller. */
    suspend fun addTag(photoId: String, tag: String) {
        api.addTag(photoId, AddTagRequest(tag))
    }

    /** Remove a tag from a photo. */
    suspend fun removeTag(photoId: String, tag: String) {
        api.removeTag(photoId, RemoveTagRequest(tag))
    }
}
