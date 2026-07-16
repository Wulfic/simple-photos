/**
 * Repository for album CRUD operations, including creation, renaming,
 * deletion, and managing photo-to-album associations via the server API.
 */
package com.simplephotos.data.repository

import androidx.room.withTransaction
import com.simplephotos.crypto.CryptoManager
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoAlbumXRef
import com.simplephotos.data.local.entities.PhotoEntity
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
import java.security.MessageDigest
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Maximum ids to bind into a single `WHERE x IN (...)` query.
 *
 * SQLite rejects a statement with more than `SQLITE_MAX_VARIABLE_NUMBER` bound
 * parameters — 999 on the API levels we support. An album can hold far more
 * members than that, so every batch lookup keyed on a member list has to chunk
 * or it throws on exactly the large libraries this code exists to make fast.
 */
const val SQLITE_VARIABLE_CHUNK = 900

/**
 * Manages album CRUD and server synchronisation.
 *
 * Each album is represented as an encrypted manifest blob on the server
 * (blob_type = "album_manifest"). The manifest contains the album name,
 * cover photo, and a list of photo blob IDs.
 *
 * Two invariants hold this together, and both exist because the server cannot
 * read a manifest and therefore cannot referee a disagreement between devices:
 *
 *  1. [AlbumEntity.photoBlobIds] is the membership of record — stored verbatim
 *     from the manifest and the only source an upload is built from. The xref
 *     table is a derived projection of it onto the local photo mirror.
 *  2. Manifest blobs are immutable (an edit uploads a *new* blob id), so a blob
 *     id we already have stored proves our copy is current and the download can
 *     be skipped entirely.
 */
@Singleton
class AlbumRepository @Inject constructor(
    private val api: ApiService,
    private val db: AppDatabase,
    private val crypto: CryptoManager
) {
    /**
     * Mirror size the last time this process derived xrefs (-1 = never).
     *
     * The xref projection is a function of (stored membership, mirror). A sync
     * that short-circuits every manifest has proved membership didn't change, so
     * if the mirror hasn't either, re-deriving is provably a no-op — and that is
     * the whole steady state, once per resume, across every album. Without this
     * the "skip unchanged manifests" win is spent on the projection instead.
     *
     * Size, not content: a mirror that swaps one photo for another between two
     * resumes keeps its size and is skipped. That's tolerable precisely because
     * counts no longer come from xrefs — they intersect the stored membership
     * with the mirror directly — so the only lag is the detail grid, and it
     * clears on the next resume in which anything at all synced.
     */
    private var xrefMirrorSize = -1

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

    suspend fun addPhotoToAlbum(photoLocalId: String, albumLocalId: String) =
        addPhotosToAlbum(listOf(photoLocalId), albumLocalId)

    suspend fun removePhotoFromAlbum(photoLocalId: String, albumLocalId: String) =
        removePhotosFromAlbum(listOf(photoLocalId), albumLocalId)

    /**
     * Add photos to an album: record them in the stored membership and project
     * them into the xref table, in one transaction.
     *
     * Batched deliberately. Per-photo writes would fire Room's invalidation
     * tracker once per photo, so adding a 500-shot selection would re-emit the
     * album list — and re-run every album's count — 500 times.
     */
    suspend fun addPhotosToAlbum(photoLocalIds: List<String>, albumLocalId: String) {
        if (photoLocalIds.isEmpty()) return
        val album = db.albumDao().getById(albumLocalId)
        if (album == null) {
            android.util.Log.w("AlbumRepository", "addPhotosToAlbum: no album '$albumLocalId'")
            return
        }
        // A photo with no serverBlobId hasn't been uploaded yet, so it cannot be
        // named in a manifest. It still gets an xref (it's in the album locally);
        // a later sync, once it has a blob id, puts it in the manifest.
        val blobIds = photosByLocalIds(photoLocalIds).mapNotNull { it.serverBlobId }
        val merged = (album.photoBlobIds + blobIds).distinct()
        db.withTransaction {
            db.albumDao().insertXRefs(photoLocalIds.map { PhotoAlbumXRef(it, albumLocalId) })
            if (merged.size != album.photoBlobIds.size) {
                db.albumDao().update(album.copy(photoBlobIds = merged))
            }
        }
    }

    /** Remove photos from an album — see [addPhotosToAlbum] for why it batches. */
    suspend fun removePhotosFromAlbum(photoLocalIds: List<String>, albumLocalId: String) {
        if (photoLocalIds.isEmpty()) return
        val album = db.albumDao().getById(albumLocalId)
        if (album == null) {
            android.util.Log.w("AlbumRepository", "removePhotosFromAlbum: no album '$albumLocalId'")
            return
        }
        val dropped = photosByLocalIds(photoLocalIds).mapNotNull { it.serverBlobId }.toSet()
        val remaining = album.photoBlobIds.filter { it !in dropped }
        db.withTransaction {
            for (id in photoLocalIds) db.albumDao().deleteXRef(id, albumLocalId)
            if (remaining.size != album.photoBlobIds.size) {
                db.albumDao().update(album.copy(photoBlobIds = remaining))
            }
        }
    }

    /**
     * Remember an album's visible count so the next cold start can render it
     * before the mirror has loaded. A no-op when unchanged — see
     * [com.simplephotos.data.local.dao.AlbumDao.updateCachedCount].
     */
    suspend fun setCachedCount(albumId: String, count: Int) =
        db.albumDao().updateCachedCount(albumId, count)

    private suspend fun photosByLocalIds(ids: List<String>): List<PhotoEntity> =
        ids.chunked(SQLITE_VARIABLE_CHUNK).flatMap { db.photoDao().getByIds(it) }

    private suspend fun photosByServerBlobIds(blobIds: List<String>): List<PhotoEntity> =
        blobIds.chunked(SQLITE_VARIABLE_CHUNK).flatMap { db.photoDao().getByServerBlobIds(it) }

    /**
     * Project an album's stored membership onto the local photo mirror, so the
     * detail grid's xref joins see the members this device can actually render.
     *
     * Idempotent and diff-guarded: when the projection already matches, this
     * writes nothing. That guard is what makes a steady-state resume silent —
     * the previous unconditional delete-all-then-reinsert fired Room
     * invalidations mid-rebuild, so observers sampled the album while it was
     * empty and the tile badge visibly fell to 0 before settling. The rewrite it
     * does perform is inside one transaction for the same reason: Room holds
     * invalidation until commit, so no observer can see the half-empty state.
     *
     * Runs on every sync, including the short-circuited path, because the
     * projection depends on the *mirror* — which changes on its own as photos
     * sync in — not on the manifest.
     */
    private suspend fun reconcileXRefs(album: AlbumEntity) {
        val desired = photosByServerBlobIds(album.photoBlobIds).map { it.localId }.toSet()
        val current = db.albumDao().getPhotoIdsForAlbum(album.localId).toSet()
        if (desired == current) return
        db.withTransaction {
            db.albumDao().deleteAllXRefsForAlbum(album.localId)
            db.albumDao().insertXRefs(desired.map { PhotoAlbumXRef(it, album.localId) })
        }
    }

    /**
     * Upload an album manifest blob to the server.
     * Encrypts the manifest JSON and uploads as blob_type = album_manifest.
     *
     * The payload is built from the album's **stored** membership, never from
     * the local mirror — see [AlbumManifest.payloadFor]. Membership is re-read
     * here rather than trusted from [album], because callers routinely hold an
     * entity they captured before their own edit landed.
     */
    suspend fun syncAlbum(album: AlbumEntity) {
        val current = db.albumDao().getById(album.localId) ?: album

        val coverBlobId = current.coverPhotoLocalId?.let { localId ->
            db.photoDao().getById(localId)?.serverBlobId
        }

        val payload = AlbumManifest.payloadFor(current, coverBlobId)
        val encrypted = crypto.encrypt(payload.toByteArray())
        val hash = crypto.sha256Hex(encrypted)
        val body = encrypted.toRequestBody("application/octet-stream".toMediaType())
        val res = api.uploadBlob(body, "album_manifest", encrypted.size.toString(), hash)

        // Update local DB with server blob ID
        db.albumDao().update(
            current.copy(
                serverManifestBlobId = res.blobId,
                syncStatus = SyncStatus.SYNCED
            )
        )

        // Only now is the old manifest unreferenced. This MUST come after the
        // upload: deleting first (as this used to) meant any failure in between
        // left the album with no manifest on the server at all, so it silently
        // vanished from every other device and no retry could recover it — the
        // bytes were already gone. An orphaned blob is the far cheaper failure.
        val oldBlobId = current.serverManifestBlobId
        if (oldBlobId != null && oldBlobId != res.blobId) {
            try {
                api.deleteBlob(oldBlobId)
            } catch (e: Exception) {
                android.util.Log.w(
                    "AlbumRepository",
                    "could not delete replaced manifest blob for '${current.name}': ${e.message}",
                )
            }
        }
    }

    /**
     * Download all album_manifest blobs from the server, decrypt, and sync
     * into the local Room DB. Albums that no longer exist on the server are
     * removed locally. This brings web-created albums into the Android app
     * and vice-versa.
     *
     * A manifest whose blob id we already have stored is skipped without being
     * downloaded: blob ids are immutable, so the id matching proves our stored
     * membership is byte-for-byte what the server holds. In the steady state
     * this reduces the whole pass to one `listBlobs` call.
     */
    suspend fun syncAlbumsFromServer() {
        val blobList = api.listBlobs(blobType = "album_manifest")
        val serverAlbumIds = mutableSetOf<String>()
        val mirrorSize = db.photoDao().countAll()
        val mirrorChanged = mirrorSize != xrefMirrorSize

        for (blob in blobList.blobs) {
            try {
                val cached = db.albumDao().getByManifestBlobId(blob.id)
                if (cached != null) {
                    serverAlbumIds.add(cached.localId)
                    // The manifest is unchanged, but the projection also tracks
                    // the mirror — so newly-synced photos still have to join the
                    // album. When the mirror hasn't moved either, there is by
                    // definition nothing to re-derive.
                    if (mirrorChanged) reconcileXRefs(cached)
                    continue
                }

                val encryptedBytes = api.downloadBlob(blob.id).bytes()
                val manifest = AlbumManifest.parse(String(crypto.decrypt(encryptedBytes)))
                serverAlbumIds.add(manifest.albumId)

                val coverLocalId = manifest.coverPhotoBlobId?.let { bId ->
                    db.photoDao().getByServerBlobId(bId)?.localId
                }

                // Only write when something actually changed: Room's
                // InvalidationTracker fires the getAllAlbums() Flow on ANY write
                // to the albums table — even a no-op update — so re-running this
                // sync re-emitted an unchanged list, which the album screen
                // re-rendered as the "constantly refreshing / tiles move then
                // settle" churn (#20).
                val existing = db.albumDao().getById(manifest.albumId)
                val entity = existing?.copy(
                    name = manifest.name,
                    serverManifestBlobId = blob.id,
                    coverPhotoLocalId = coverLocalId ?: existing.coverPhotoLocalId,
                    syncStatus = SyncStatus.SYNCED,
                    photoBlobIds = manifest.photoBlobIds,
                ) ?: AlbumEntity(
                    localId = manifest.albumId,
                    serverManifestBlobId = blob.id,
                    name = manifest.name,
                    coverPhotoLocalId = coverLocalId,
                    syncStatus = SyncStatus.SYNCED,
                    createdAt = manifest.createdAt ?: System.currentTimeMillis(),
                    photoBlobIds = manifest.photoBlobIds,
                )
                if (existing == null) db.albumDao().insert(entity)
                else if (entity != existing) db.albumDao().update(entity)

                reconcileXRefs(entity)
            } catch (e: Exception) {
                // Skip manifests we can't decrypt (e.g. different key), but say so:
                // silence here used to make a whole-library key mismatch look
                // identical to "you have no albums".
                android.util.Log.w(
                    "AlbumRepository",
                    "skipping unreadable album manifest '${blob.id}': ${e.message}",
                )
            }
        }

        // Remove local albums that no longer exist on the server
        // (only those that have a serverManifestBlobId — i.e. were synced)
        val allLocalIds = db.albumDao().getAllAlbumIds()
        for (localId in allLocalIds) {
            val album = db.albumDao().getById(localId) ?: continue
            if (album.serverManifestBlobId != null && localId !in serverAlbumIds) {
                db.withTransaction {
                    db.albumDao().deleteAllXRefsForAlbum(localId)
                    db.albumDao().deleteById(localId)
                }
            }
        }

        // Only once the pass completed: a mid-loop failure must not record the
        // mirror as fully projected, or the album it skipped stays stale until
        // the mirror happens to change size again.
        xrefMirrorSize = mirrorSize
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
     *
     * Merges into the stored membership; it never replaces it. A device that has
     * synced only part of the library contributes the members it can see and
     * leaves the rest of the album untouched.
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
            val localPhotos = album.photoIds
                .chunked(SQLITE_VARIABLE_CHUNK)
                .flatMap { db.photoDao().getByServerPhotoIds(it) }
            val foundServerIds = localPhotos.mapNotNull { it.serverPhotoId }.toSet()
            photosUnmatched += album.photoIds.count { it !in foundServerIds }
            // Manifests name photos by blob id, so a local photo that hasn't been
            // uploaded yet can't be contributed to one.
            val matchedBlobIds = localPhotos.mapNotNull { it.serverBlobId }
            if (matchedBlobIds.isEmpty()) {
                albumsUnmatched++
                continue
            }

            val albumId = sourceAlbumId(album.source, album.name)

            val existing = db.albumDao().getById(albumId)
            if (existing == null) {
                val ids = matchedBlobIds.distinct()
                val entity = AlbumEntity(
                    localId = albumId,
                    name = resolveAlbumDisplayName(album.name, album.title, null),
                    coverPhotoLocalId = localPhotos.first().localId,
                    syncStatus = SyncStatus.PENDING,
                    photoBlobIds = ids,
                )
                db.albumDao().insert(entity)
                reconcileXRefs(entity)
                created++
                photosAdded += ids.size
                toSync.add(entity)
            } else {
                val merged = (existing.photoBlobIds + matchedBlobIds).distinct()
                val added = merged.size - existing.photoBlobIds.size
                val name = resolveAlbumDisplayName(album.name, album.title, existing.name)
                val renamed = name != existing.name
                // No-op: nothing new, skip the upload. The rename is checked too,
                // because an album materialized under the mangled folder name by
                // an earlier run adds no photos — a photos-only check would leave
                // it wrongly named forever.
                if (added == 0 && !renamed) continue
                val entity = existing.copy(name = name, photoBlobIds = merged)
                // Persist before the upload, so a failed manifest sync doesn't
                // lose the merge or the rename (a later pass retries the upload).
                db.albumDao().update(entity)
                reconcileXRefs(entity)
                updated++
                photosAdded += added
                if (renamed) renamedCount++
                toSync.add(entity)
            }
        }

        // Phase 2 (expensive, parallel): each syncAlbum encrypts a manifest and does
        // an upload network round-trip. Running these sequentially is what made
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

        /**
         * Whether a reconstruction pass proved there is nothing left to
         * materialize, so the latch can be set and the pass skipped from here on.
         *
         * **Must stay identical to web's `takeoutSettled`** — both platforms run
         * the same reconstruction against the same server mapping, so a rule that
         * differs means one device keeps re-uploading manifests the other
         * considers finished.
         *
         * Normally settled means every source photo matched. But a photo that was
         * trashed or moved to the secure gallery never syncs into the mirror at
         * all, so `photosUnmatched` can never reach 0 and the pass would re-run
         * forever. The second clause catches that: a pass that changed nothing and
         * left exactly the same gap as the one before it has proved the gap is
         * permanent — more photos arrived, none of them were the missing ones.
         * Deliberately conservative; latching early means silently incomplete
         * albums, which is the bug this whole path exists to fix.
         */
        fun takeoutSettled(
            result: SourceAlbumRebuildResult,
            previousUnmatched: Int,
        ): Boolean {
            if (result.photosUnmatched == 0) return true
            val noop = result.albumsCreated == 0 &&
                result.albumsUpdated == 0 &&
                result.photosAdded == 0
            return noop && result.photosUnmatched == previousUnmatched
        }

        /**
         * An album's visible member count: stored membership, intersected with the
         * local mirror, minus anything in a secure gallery.
         *
         * The same predicate as web's `countRegularAlbum` and as the album-detail
         * grid — deliberately, since a tile badge that disagrees with the grid it
         * opens is exactly bugs #12 and #20. Pure, so the rule is testable
         * without Room.
         */
        fun visibleMemberCount(
            photoBlobIds: List<String>,
            mirrorBlobIds: Set<String>,
            secureBlobIds: Set<String>,
        ): Int = photoBlobIds.count { it in mirrorBlobIds && it !in secureBlobIds }
    }
}
