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
}
