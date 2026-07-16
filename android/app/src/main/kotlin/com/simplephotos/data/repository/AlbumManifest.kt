/**
 * The album-manifest wire format: the one place its JSON is written and read.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.local.entities.AlbumEntity
import org.json.JSONArray
import org.json.JSONObject

/** An album manifest as it came off the server, decrypted and parsed. */
data class ParsedAlbumManifest(
    val albumId: String,
    val name: String,
    /** Null when the manifest carried no parsable `created_at`. */
    val createdAt: Long?,
    val coverPhotoBlobId: String?,
    val photoBlobIds: List<String>,
)

/**
 * Encodes and decodes the encrypted `album_manifest` blob payload.
 *
 * Regular albums are end-to-end encrypted: the server stores opaque bytes and
 * cannot read, validate, or repair a manifest. This format is therefore the
 * *entire* contract between web and Android, and it is only ever verified by
 * the two of them agreeing — keep in step with `web/src/utils/albumManifest.ts`.
 */
object AlbumManifest {
    fun build(
        albumId: String,
        name: String,
        createdAtMillis: Long,
        coverPhotoBlobId: String?,
        photoBlobIds: List<String>,
    ): String = JSONObject().apply {
        put("v", 1)
        put("album_id", albumId)
        put("name", name)
        put("created_at", java.time.Instant.ofEpochMilli(createdAtMillis).toString())
        put("cover_photo_blob_id", coverPhotoBlobId ?: JSONObject.NULL)
        put("photo_blob_ids", JSONArray(photoBlobIds))
    }.toString()

    /**
     * The manifest payload for [album], built from the membership the album
     * stores ([AlbumEntity.photoBlobIds]) — **never** from the local photo
     * mirror.
     *
     * That distinction is the whole point of this function existing. Building the
     * upload by mapping local xrefs back to server blob ids silently drops every
     * member this device hasn't synced yet, and since this payload *replaces* the
     * album's manifest for every device, a partially-synced phone would shrink
     * the album to whatever subset it happened to hold. Take the stored list as
     * given; the mirror has no say in what the album contains.
     */
    fun payloadFor(album: AlbumEntity, coverPhotoBlobId: String?): String =
        build(
            albumId = album.localId,
            name = album.name,
            createdAtMillis = album.createdAt,
            coverPhotoBlobId = coverPhotoBlobId,
            photoBlobIds = album.photoBlobIds,
        )

    /** Throws if the payload isn't a manifest we understand. */
    fun parse(json: String): ParsedAlbumManifest {
        val payload = JSONObject(json)
        val ids = mutableListOf<String>()
        payload.optJSONArray("photo_blob_ids")?.let { arr ->
            for (i in 0 until arr.length()) ids.add(arr.getString(i))
        }
        return ParsedAlbumManifest(
            albumId = payload.getString("album_id"),
            name = payload.getString("name"),
            createdAt = try {
                java.time.Instant.parse(payload.optString("created_at", "")).toEpochMilli()
            } catch (_: Exception) {
                null
            },
            coverPhotoBlobId = if (payload.isNull("cover_photo_blob_id")) null
                else payload.optString("cover_photo_blob_id").ifEmpty { null },
            photoBlobIds = ids,
        )
    }
}
