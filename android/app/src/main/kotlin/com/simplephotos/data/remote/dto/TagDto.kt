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
    val tags: List<String>
)

data class SearchResponse(
    val results: List<SearchResult>
)
