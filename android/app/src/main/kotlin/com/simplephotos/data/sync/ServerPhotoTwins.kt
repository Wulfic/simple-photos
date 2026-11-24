/**
 * One Room row per server photo (TODO.md D2).
 *
 * Pure — no Room, no Retrofit, no `android.util.Log` — so the decision that can
 * be wrong is JVM-unit-testable, like [decideSyncMode].
 *
 * ## How twins happen
 *
 * The gallery draws one tile per Room row. A server photo is supposed to be one
 * row, but two paths each create one:
 *
 * - the server→device sync inserts a **server-only** row (no `localPath`) for
 *   every library photo it has not seen, and
 * - the backup pass registers each **device** row (`localPath` set). When the
 *   server already holds that content, `register-encrypted` answers with the
 *   EXISTING photo id, and the device row is marked with it.
 *
 * Nothing ever merged the two: `deduplicateLocalEntities` only considers device
 * rows that have **no** `serverPhotoId` yet. Two device copies of the same file
 * (e.g. `DCIM` and `DCIM/Restored` both backed up) twin the same way. Measured on
 * the owner's phone 2026-11-17: 4,283 server photos drawn twice — 18,826 rows for
 * a ~14.5k-photo library.
 */
package com.simplephotos.data.sync

import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.dto.RegisterEncryptedPhotoResponse

/**
 * The `(blob, thumb)` a row should record after `register-encrypted`.
 *
 * On a duplicate answer that names the existing photo's blobs, those win: the
 * pair this client just uploaded is referenced by nothing server-side, and a row
 * keyed on it escapes the blob-keyed secure filter. Anything else — a new photo,
 * or an older server that does not name them — keeps what was uploaded.
 */
fun canonicalBlobIds(
    res: RegisterEncryptedPhotoResponse,
    uploadedBlob: String,
    uploadedThumb: String?,
): Pair<String, String?> {
    val canonical = res.encryptedBlobId
    return if (res.duplicate == true && canonical != null) {
        canonical to (res.encryptedThumbBlobId ?: uploadedThumb)
    } else {
        uploadedBlob to uploadedThumb
    }
}

/** What collapsing one group of rows that share a `serverPhotoId` should do. */
data class TwinCollapse(
    /** The single row that survives, with the merged state to write. */
    val keeper: PhotoEntity,
    /** Rows to delete. Never contains the keeper's `localId`. */
    val victims: List<PhotoEntity>,
    /** Cached thumbnail files only a victim used — safe to delete. */
    val orphanedThumbnails: List<String>,
)

/**
 * Plan the collapse of [rows] — every Room row holding one `serverPhotoId` — into
 * one. Returns null when there is nothing to collapse.
 *
 * ## Which row survives
 *
 * A **device** row (`localPath` set) over a server-only one: it is the row the
 * backup tracks, it views from local storage, and the existing sync rules already
 * protect it from server-side pruning (`tombstoneVictims` / the full walk only
 * remove rows with no `localPath`). Among several, the oldest (`createdAt`, then
 * `localId`) — any stable choice works; an unstable one would flip the keeper on
 * every pass and churn thumbnails.
 *
 * ## Whose server state it carries
 *
 * A server-only twin came from the server's own feed, so its blob ids are what the
 * server actually serves. A device row's `serverBlobId` may instead be the blob it
 * uploaded itself before `register-encrypted` mapped it onto an existing photo — a
 * blob nothing server-side references, and one the blob-keyed secure filter does
 * not know. So the server-only twin's blob ids, favourite, crop, subtype and ladder
 * win. Favourite/crop can still be stale on either side; the caller follows a
 * collapse with a FULL walk, whose whole-library crop/favourite pass is the
 * existing recovery path for exactly that.
 *
 * @throws IllegalArgumentException if the rows do not all share one non-null
 *   `serverPhotoId` — merging rows that are NOT the same photo would destroy a
 *   tile the user should see. Fail closed.
 */
fun planTwinCollapse(rows: List<PhotoEntity>): TwinCollapse? {
    if (rows.size < 2) return null
    val serverId = rows.first().serverPhotoId
    require(serverId != null && rows.all { it.serverPhotoId == serverId }) {
        "planTwinCollapse needs rows sharing one non-null serverPhotoId, got " +
            rows.map { it.serverPhotoId }.distinct()
    }
    require(rows.map { it.localId }.toSet().size == rows.size) {
        "planTwinCollapse got the same localId twice"
    }

    val stable = compareBy<PhotoEntity>({ it.createdAt }, { it.localId })
    val device = rows.filter { it.localPath != null }.sortedWith(stable)
    val serverOnly = rows.filter { it.localPath == null }.sortedWith(stable)
    val base = device.firstOrNull() ?: serverOnly.first()
    val canonical = serverOnly.firstOrNull { it.localId != base.localId }

    val merged = if (canonical == null) {
        base.copy(syncStatus = SyncStatus.SYNCED)
    } else {
        base.copy(
            serverBlobId = canonical.serverBlobId ?: base.serverBlobId,
            thumbnailBlobId = canonical.thumbnailBlobId ?: base.thumbnailBlobId,
            isFavorite = canonical.isFavorite,
            cropMetadata = canonical.cropMetadata,
            photoSubtype = canonical.photoSubtype ?: base.photoSubtype,
            burstId = canonical.burstId ?: base.burstId,
            motionVideoBlobId = canonical.motionVideoBlobId ?: base.motionVideoBlobId,
            renditions = canonical.renditions.ifEmpty { base.renditions },
            sizeBytes = canonical.sizeBytes ?: base.sizeBytes,
            photoHash = base.photoHash ?: canonical.photoHash,
            thumbnailPath = base.thumbnailPath ?: canonical.thumbnailPath,
            syncStatus = SyncStatus.SYNCED,
        )
    }

    val victims = rows.filter { it.localId != merged.localId }
    val orphaned = victims.mapNotNull { it.thumbnailPath }
        .filter { it != merged.thumbnailPath }
        .distinct()
    return TwinCollapse(keeper = merged, victims = victims, orphanedThumbnails = orphaned)
}
