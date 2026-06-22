/**
 * Tag and search DTOs — tag CRUD payloads and combined tag + text
 * search result shapes.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Tags ─────────────────────────────────────────────────────────────────────

data class TagListResponse(
    val tags: List<String>
)

data class PhotoTagsResponse(
    @SerializedName("photo_id") val photoId: String,
    val tags: List<String>
)

data class AddTagRequest(
    val tag: String
)

data class RemoveTagRequest(
    val tag: String
)

// ── Search ───────────────────────────────────────────────────────────────────

data class SearchResult(
    val id: String,
    val filename: String,
    @SerializedName("media_type") val mediaType: String,
    @SerializedName("mime_type") val mimeType: String,
    @SerializedName("thumb_path") val thumbPath: String?,
    @SerializedName("created_at") val createdAt: String,
    @SerializedName("taken_at") val takenAt: String? = null,
    val latitude: Double? = null,
    val longitude: Double? = null,
    val width: Int? = null,
    val height: Int? = null,
    val tags: List<String>,
    // Burst grouping: the server collapses burst stacks to a single result and
    // reports the group's frame count, so search matches the gallery/secure
    // grids (one tile per burst) instead of showing every frame.
    @SerializedName("burst_id") val burstId: String? = null,
    @SerializedName("burst_count") val burstCount: Int? = null
)

data class SearchResponse(
    val results: List<SearchResult>
)
