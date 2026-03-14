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
 * In encrypted mode, each album is represented as an encrypted manifest blob
 * on the server (blob_type = "album_manifest"). The manifest contains the
 * album name, cover photo, and a list of photo blob IDs. In plain mode,
 * albums are stored locally only.
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
     * In plain mode (no encryption key), this is a no-op — albums stay local only.
     */
    suspend fun syncAlbum(album: AlbumEntity) {
        // In plain mode the crypto key isn't available; skip manifest sync.
        try {
            val encSettings = api.getEncryptionSettings()
            if (encSettings.encryptionMode == "plain") {
                db.albumDao().update(album.copy(syncStatus = SyncStatus.SYNCED))
                return
            }
        } catch (_: Exception) {
            // Can't determine mode — try sync anyway
        }

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
     *
     * In plain mode (no encryption) this is a no-op — albums stay local.
     */
    suspend fun syncAlbumsFromServer() {
        // Skip in plain mode — manifests are encrypted
        try {
            val encSettings = api.getEncryptionSettings()
            if (encSettings.encryptionMode == "plain") return
        } catch (_: Exception) {
            return // can't determine mode — bail out
        }

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
}
