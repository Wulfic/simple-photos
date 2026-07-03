/**
 * Repository for album CRUD operations, including creation, renaming,
 * deletion, and managing photo-to-album associations via the server API.
 */
package com.simplephotos.data.repository

import com.simplephotos.crypto.CryptoManager
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoAlbumXRef
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import kotlinx.coroutines.flow.Flow
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Manages album CRUD and server synchronisation.
 *
 * Each album is represented as an encrypted manifest blob on the server
 * (blob_type = "album_manifest"). The manifest contains the album name,
 * cover photo, and a list of photo blob IDs.
 */
@Singleton
class AlbumRepository @Inject constructor(
    private val api: ApiService,
    private val db: AppDatabase,
    private val crypto: CryptoManager
) {
    fun getAllAlbums(): Flow<List<AlbumEntity>> = db.albumDao().getAllAlbums()

    suspend fun getAlbum(id: String): AlbumEntity? = db.albumDao().getById(id)

    suspend fun createAlbum(name: String): AlbumEntity {
        val album = AlbumEntity(
            localId = java.util.UUID.randomUUID().toString(),
            name = name,
            syncStatus = SyncStatus.PENDING
        )
        db.albumDao().insert(album)
        return album
    }

    suspend fun deleteAlbum(album: AlbumEntity) {
        // Delete manifest blob from server
        album.serverManifestBlobId?.let { blobId ->
            try { api.deleteBlob(blobId) } catch (_: Exception) {}
        }
        db.albumDao().delete(album)
    }

    suspend fun addPhotoToAlbum(photoLocalId: String, albumLocalId: String) {
        db.albumDao().insertXRef(PhotoAlbumXRef(photoLocalId, albumLocalId))
    }

    suspend fun removePhotoFromAlbum(photoLocalId: String, albumLocalId: String) {
        db.albumDao().deleteXRef(photoLocalId, albumLocalId)
    }

    suspend fun getPhotoIdsForAlbum(albumId: String): List<String> =
        db.albumDao().getPhotoIdsForAlbum(albumId)

    /**
     * Upload an album manifest blob to the server.
     * Encrypts the manifest JSON and uploads as blob_type = album_manifest.
     */
    suspend fun syncAlbum(album: AlbumEntity) {
        val photoIds = db.albumDao().getPhotoIdsForAlbum(album.localId)

        // Build the server's photo blob IDs from the local-to-server mapping
        val photoBlobIds = photoIds.mapNotNull { localId ->
            db.photoDao().getById(localId)?.serverBlobId
        }

        val coverBlobId = album.coverPhotoLocalId?.let { localId ->
            db.photoDao().getById(localId)?.serverBlobId
        }

        // Delete old manifest if exists
        album.serverManifestBlobId?.let { oldBlobId ->
            try { api.deleteBlob(oldBlobId) } catch (_: Exception) {}
        }

        // Build manifest payload
        val payload = JSONObject().apply {
            put("v", 1)
            put("album_id", album.localId)
            put("name", album.name)
            put("created_at", java.time.Instant.ofEpochMilli(album.createdAt).toString())
            put("cover_photo_blob_id", coverBlobId ?: JSONObject.NULL)
            put("photo_blob_ids", JSONArray(photoBlobIds))
        }.toString()

        val encrypted = crypto.encrypt(payload.toByteArray())
        val hash = crypto.sha256Hex(encrypted)
        val body = encrypted.toRequestBody("application/octet-stream".toMediaType())
        val res = api.uploadBlob(body, "album_manifest", encrypted.size.toString(), hash)

        // Update local DB with server blob ID
        db.albumDao().update(
            album.copy(
                serverManifestBlobId = res.blobId,
                syncStatus = SyncStatus.SYNCED
            )
        )
    }

    /**
     * Download all album_manifest blobs from the server, decrypt, and sync
     * into the local Room DB. Albums that no longer exist on the server are
     * removed locally. This brings web-created albums into the Android app
     * and vice-versa.
     */
    suspend fun syncAlbumsFromServer() {
        val blobList = api.listBlobs(blobType = "album_manifest")
        val serverAlbumIds = mutableSetOf<String>()

        for (blob in blobList.blobs) {
            try {
                // Download and decrypt the manifest blob
                val encryptedBody = api.downloadBlob(blob.id)
                val encryptedBytes = encryptedBody.bytes()
                val decryptedBytes = crypto.decrypt(encryptedBytes)
                val payload = JSONObject(String(decryptedBytes))

                val albumId = payload.getString("album_id")
                val albumName = payload.getString("name")
                val createdAtStr = payload.optString("created_at", "")
                val coverBlobId = if (payload.isNull("cover_photo_blob_id")) null
                    else payload.getString("cover_photo_blob_id")
                val photoBlobIds = mutableListOf<String>()
                val arr = payload.optJSONArray("photo_blob_ids")
                if (arr != null) {
                    for (i in 0 until arr.length()) {
                        photoBlobIds.add(arr.getString(i))
                    }
                }

                serverAlbumIds.add(albumId)

                // Parse created_at to epoch millis
                val createdAt = try {
                    java.time.Instant.parse(createdAtStr).toEpochMilli()
                } catch (_: Exception) {
                    System.currentTimeMillis()
                }

                // Map cover blob ID → local photo ID
                val coverLocalId = coverBlobId?.let { bId ->
                    db.photoDao().getByServerBlobId(bId)?.localId
                }

                // Upsert the album entry
                val existing = db.albumDao().getById(albumId)
                if (existing != null) {
                    db.albumDao().update(
                        existing.copy(
                            name = albumName,
                            serverManifestBlobId = blob.id,
                            coverPhotoLocalId = coverLocalId ?: existing.coverPhotoLocalId,
                            syncStatus = SyncStatus.SYNCED
                        )
                    )
                } else {
                    db.albumDao().insert(
                        AlbumEntity(
                            localId = albumId,
                            serverManifestBlobId = blob.id,
                            name = albumName,
                            coverPhotoLocalId = coverLocalId,
                            syncStatus = SyncStatus.SYNCED,
                            createdAt = createdAt
                        )
                    )
                }

                // Rebuild photo ↔ album cross-references
                db.albumDao().deleteAllXRefsForAlbum(albumId)
                for (blobId in photoBlobIds) {
                    val photo = db.photoDao().getByServerBlobId(blobId)
                    if (photo != null) {
                        db.albumDao().insertXRef(
                            PhotoAlbumXRef(photo.localId, albumId)
                        )
                    }
                }
            } catch (_: Exception) {
                // Skip manifests we can't decrypt (e.g. different key)
            }
        }

        // Remove local albums that no longer exist on the server
        // (only those that have a serverManifestBlobId — i.e. were synced)
        val allLocalIds = db.albumDao().getAllAlbumIds()
        for (localId in allLocalIds) {
            val album = db.albumDao().getById(localId) ?: continue
            if (album.serverManifestBlobId != null && localId !in serverAlbumIds) {
                db.albumDao().deleteAllXRefsForAlbum(localId)
                db.albumDao().deleteById(localId)
            }
        }
    }

    /** Outcome of a Takeout source-album rebuild (Issue 2). */
    data class SourceAlbumRebuildResult(
        val albumsCreated: Int = 0,
        val albumsUpdated: Int = 0,
        val photosAdded: Int = 0,
        /** Albums whose photos aren't in the local mirror yet (still syncing). */
        val albumsUnmatched: Int = 0,
        /** Individual photo ids not yet synced locally (skipped, re-run to fill). */
        val photosUnmatched: Int = 0,
    )

    /**
     * Rebuild local albums from the server's **authoritative** Takeout
     * source-album mapping (`GET /api/photos/source-albums`), keyed by photo id
     * (Issue 2). This is the Android equivalent of the web
     * `recreateAlbumsFromServer` and the deterministic, cross-platform
     * replacement for filename matching: the server captured each album folder
     * at import time, so it survives `-edited` dedup and `(1)` collision renames
     * and needs no folder re-selection.
     *
     * The album id is derived deterministically from `(source, name)` with the
     * **same formula the web client uses**, so a rebuild on either platform
     * converges into one album after sync instead of duplicating. A photo id not
     * yet in the local Room mirror is skipped and counted; re-run after the sync
     * finishes to fill it in. Idempotent — re-running adds nothing new.
     */
    suspend fun recreateAlbumsFromServer(): SourceAlbumRebuildResult {
        val resp = api.sourceAlbums()
        var created = 0
        var updated = 0
        var photosAdded = 0
        var albumsUnmatched = 0
        var photosUnmatched = 0

        for (album in resp.albums) {
            // Resolve server photo ids → local photos (batched). Ids not yet in
            // the local mirror are counted and skipped.
            val localPhotos = db.photoDao().getByServerPhotoIds(album.photoIds)
            val foundServerIds = localPhotos.mapNotNull { it.serverPhotoId }.toSet()
            photosUnmatched += album.photoIds.count { it !in foundServerIds }
            val localPhotoIds = localPhotos.map { it.localId }
            if (localPhotoIds.isEmpty()) {
                albumsUnmatched++
                continue
            }

            // Deterministic id — MUST match web (utils/takeoutAlbums.ts):
            // "src-" + sha256Hex(utf8("<source> <name>")).
            val albumId = "src-" + crypto.sha256Hex("${album.source} ${album.name}".toByteArray())

            val existing = db.albumDao().getById(albumId)
            if (existing == null) {
                db.albumDao().insert(
                    AlbumEntity(
                        localId = albumId,
                        name = album.name,
                        coverPhotoLocalId = localPhotoIds.first(),
                        syncStatus = SyncStatus.PENDING,
                    )
                )
                for (id in localPhotoIds) db.albumDao().insertXRef(PhotoAlbumXRef(id, albumId))
                created++
                photosAdded += localPhotoIds.size
                db.albumDao().getById(albumId)?.let { syncAlbum(it) }
            } else {
                // Only add xrefs not already present, so we never double-insert
                // the composite-key row (idempotent merge).
                val alreadyIn = db.albumDao().getPhotoIdsForAlbum(albumId).toSet()
                val toAdd = localPhotoIds.filter { it !in alreadyIn }
                if (toAdd.isEmpty()) continue
                for (id in toAdd) db.albumDao().insertXRef(PhotoAlbumXRef(id, albumId))
                updated++
                photosAdded += toAdd.size
                syncAlbum(existing)
            }
        }
        return SourceAlbumRebuildResult(created, updated, photosAdded, albumsUnmatched, photosUnmatched)
    }
}
