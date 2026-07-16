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
import com.simplephotos.data.remote.dto.DismissSourceAlbumRequest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest
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
        // Tombstone FIRST for a Takeout-reconstructed album, otherwise this
        // delete is undone: the next rebuild recreates the album from the
        // untouched server-side membership, on this device and every other.
        // Doing it before the local delete means a failure here leaves the album
        // intact rather than deleting it locally only for it to reappear.
        if (album.localId.startsWith(SOURCE_ALBUM_ID_PREFIX)) {
            api.dismissSourceAlbum(DismissSourceAlbumRequest(album.localId))
        }
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

        // Only now is the old manifest unreferenced. This MUST come after the
        // upload: deleting first (as this used to) meant any failure in between
        // left the album with no manifest on the server at all, so it silently
        // vanished from every other device and no retry could recover it — the
        // bytes were already gone. An orphaned blob is the far cheaper failure.
        val oldBlobId = album.serverManifestBlobId
        if (oldBlobId != null && oldBlobId != res.blobId) {
            try {
                api.deleteBlob(oldBlobId)
            } catch (e: Exception) {
                android.util.Log.w(
                    "AlbumRepository",
                    "could not delete replaced manifest blob for '${album.name}': ${e.message}",
                )
            }
        }
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

                // Upsert the album entry. Only write when something actually
                // changed: Room's InvalidationTracker fires the getAllAlbums()
                // Flow on ANY write to the albums table — even a no-op update —
                // so re-running this sync on every ON_RESUME re-emitted an
                // unchanged list, which the album screen re-rendered as the
                // "constantly refreshing / tiles move then settle" churn (#20).
                val existing = db.albumDao().getById(albumId)
                if (existing != null) {
                    val updated = existing.copy(
                        name = albumName,
                        serverManifestBlobId = blob.id,
                        coverPhotoLocalId = coverLocalId ?: existing.coverPhotoLocalId,
                        syncStatus = SyncStatus.SYNCED
                    )
                    if (updated != existing) db.albumDao().update(updated)
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
        /** Albums re-titled from the Takeout folder name to their real name. */
        val albumsRenamed: Int = 0,
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
        var renamedCount = 0
        var albumsUnmatched = 0
        var photosUnmatched = 0

        // Phase 1 (cheap, sequential): apply the idempotent local DB mutations and
        // collect the albums whose manifest actually changed. Albums already fully
        // materialized are skipped here so a re-run (every ON_RESUME) costs zero
        // network round-trips — the common steady state after the first pass.
        val toSync = mutableListOf<AlbumEntity>()
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

            val albumId = sourceAlbumId(album.source, album.name)

            val existing = db.albumDao().getById(albumId)
            if (existing == null) {
                db.albumDao().insert(
                    AlbumEntity(
                        localId = albumId,
                        name = resolveAlbumDisplayName(album.name, album.title, null),
                        coverPhotoLocalId = localPhotoIds.first(),
                        syncStatus = SyncStatus.PENDING,
                    )
                )
                for (id in localPhotoIds) db.albumDao().insertXRef(PhotoAlbumXRef(id, albumId))
                created++
                photosAdded += localPhotoIds.size
                db.albumDao().getById(albumId)?.let { toSync.add(it) }
            } else {
                // Only add xrefs not already present, so we never double-insert
                // the composite-key row (idempotent merge).
                val alreadyIn = db.albumDao().getPhotoIdsForAlbum(albumId).toSet()
                val toAdd = localPhotoIds.filter { it !in alreadyIn }
                val name = resolveAlbumDisplayName(album.name, album.title, existing.name)
                val renamed = name != existing.name
                // No-op: nothing new, skip the upload. The rename is checked too,
                // because an album materialized under the mangled folder name by
                // an earlier run adds no photos — a photos-only check would leave
                // it wrongly named forever.
                if (toAdd.isEmpty() && !renamed) continue
                for (id in toAdd) db.albumDao().insertXRef(PhotoAlbumXRef(id, albumId))
                // Persist the rename before the upload, so a failed manifest sync
                // doesn't lose it (a later pass retries the upload).
                val entity = if (renamed) {
                    existing.copy(name = name).also { db.albumDao().update(it) }
                } else {
                    existing
                }
                updated++
                photosAdded += toAdd.size
                if (renamed) renamedCount++
                toSync.add(entity)
            }
        }

        // Phase 2 (expensive, parallel): each syncAlbum encrypts a manifest and does
        // a delete+upload network round-trip. Running these sequentially is what made
        // reconstruction crawl (~an hour) on large libraries; a bounded pool collapses
        // it into ~toSync/CONCURRENCY waves. Bounded so we don't flood the server
        // mid-import. Mirrors the web worker-pool fix (utils/takeoutAlbums.ts).
        if (toSync.isNotEmpty()) {
            val gate = Semaphore(6)
            coroutineScope {
                toSync.map { album ->
                    async(Dispatchers.IO) {
                        gate.withPermit {
                            try {
                                syncAlbum(album)
                            } catch (e: Exception) {
                                // One album failing must not abort the rest; a later
                                // refresh retries it.
                                android.util.Log.w(
                                    "AlbumRepository",
                                    "takeout manifest sync failed for '${album.name}': ${e.message}",
                                )
                            }
                        }
                    }
                }.awaitAll()
            }
        }

        return SourceAlbumRebuildResult(
            created,
            updated,
            photosAdded,
            renamedCount,
            albumsUnmatched,
            photosUnmatched,
        )
    }

    companion object {
        /** Marks an album as reconstructed from Takeout rather than user-created. */
        const val SOURCE_ALBUM_ID_PREFIX = "src-"

        /**
         * The deterministic local id for a Takeout source album.
         *
         * `"src-" + sha256Hex(utf8("<source> <folderName>"))` — **must** match
         * web (`utils/takeoutAlbums.ts::sourceAlbumId`) and the server
         * (`import/takeout.rs::source_album_id`), which recomputes it to resolve
         * a deleted album back to its identity. All three are pinned to one
         * shared test vector, because every way they can disagree is silent:
         * albums duplicate instead of converging into one, and delete tombstones
         * stop matching so deleted albums come back.
         *
         * Keyed on the Takeout folder name, never the title — identity must
         * survive a retitle.
         */
        fun sourceAlbumId(source: String, folderName: String): String {
            val digest = MessageDigest.getInstance("SHA-256")
                .digest("$source $folderName".toByteArray())
            return SOURCE_ALBUM_ID_PREFIX + digest.joinToString("") { "%02x".format(it) }
        }

        /**
         * The name to show for a Takeout source album, given the local album it
         * maps onto (`existingName` — null when it's about to be created).
         *
         * **Must stay identical to the web rule** (`utils/takeoutAlbums.ts`
         * `resolveAlbumDisplayName`): both platforms rebuild the same albums from
         * the same server mapping, so a divergence here means the two devices
         * fight over an album's name on every sync.
         *
         * Takeout folder names are mangled ("Mum & Dad's 40th" exports as
         * "Mum _ Dad_s 40th"), so the real title from the album's `metadata.json`
         * wins for display. The folder name remains the album's *identity* (it
         * derives the deterministic album id), so renaming is purely cosmetic.
         *
         * The rename only ever supersedes a name **we** wrote (i.e. still exactly
         * the raw folder name). Any other name means the user renamed it — leave
         * their curation alone rather than stomping it on every rebuild.
         */
        fun resolveAlbumDisplayName(
            folderName: String,
            title: String?,
            existingName: String?,
        ): String {
            val display = title?.trim()?.takeIf { it.isNotEmpty() } ?: folderName
            if (existingName == null) return display
            return if (existingName == folderName) display else existingName
        }
    }
}
