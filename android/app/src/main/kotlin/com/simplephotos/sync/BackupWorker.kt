/**
 * WorkManager-based background worker that scans, deduplicates, and uploads
 * new photos and videos to the server.
 */
package com.simplephotos.sync

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.media.MediaMetadataRetriever
import android.media.ThumbnailUtils
import android.util.Log
import androidx.exifinterface.media.ExifInterface
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.hilt.work.HiltWorker
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import com.simplephotos.crypto.ChunkedBlob
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SyncRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_DIAGNOSTIC_LOGGING
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_WIFI_ONLY_BACKUP
import dagger.assisted.Assisted
import dagger.assisted.AssistedInject
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.InputStream

/**
 * WorkManager worker that performs the automatic photo/video backup pipeline:
 *
 * 1. Scan MediaStore for new media in user-selected folders.
 * 2. Reset any uploads stuck at UPLOADING (from a previous crash).
 * 3. Deduplicate by content hash.
 * 4. Generate JPEG thumbnails (256×256) for each new item.
 * 5. Upload via [PhotoRepository] in encrypted mode.
 * 6. Re-register the reactive MediaStore content-URI observer for next trigger.
 *
 * Retries on failure using WorkManager's exponential backoff.
 */
@HiltWorker
class BackupWorker @AssistedInject constructor(
    @Assisted context: Context,
    @Assisted params: WorkerParameters,
    private val photoRepository: PhotoRepository,
    private val syncRepository: SyncRepository,
    private val db: AppDatabase,
    private val api: ApiService,
    private val dataStore: DataStore<Preferences>
) : CoroutineWorker(context, params) {

    companion object {
        private const val TAG = "BackupWorker"
        private const val MAX_ATTEMPTS = 5
        private val EXIF_DIM_REPAIR_DONE = booleanPreferencesKey("exif_dim_repair_done")
    }

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val prefs = dataStore.data.first()
        val loggingEnabled = prefs[KEY_DIAGNOSTIC_LOGGING] ?: false
        val diag = DiagnosticLogger(api, loggingEnabled)
        try {
            diag.info(TAG, "Backup worker started", mapOf(
                "runAttemptCount" to runAttemptCount.toString()
            ))

            // Step 1: Scan MediaStore for new photos and videos
            diag.info(TAG, "Scanning MediaStore for new media")
            syncRepository.scanForNewMedia()

            // Step 1.5: One-time EXIF dimension repair for previously uploaded photos
            repairExifDimensions(diag)

            // Step 1.6: Reconcile — detect server reset / data loss.
            // If the server's encrypted-sync returns zero photos but we have
            // locally SYNCED entries with server blob IDs, the server was reset.
            // Reset those items to PENDING so they get re-uploaded.
            reconcileSyncedWithServer(diag)

            // Step 2: Recover stuck uploads — photos left at UPLOADING from a crash
            // are reset to PENDING so they get retried.
            db.photoDao().resetStuckUploading()
            diag.debug(TAG, "Reset any stuck UPLOADING photos back to PENDING")

            // Step 3: Mark exhausted queue entries as failed
            db.blobQueueDao().markExhaustedAsFailed()

            // Step 4: Process photos that need uploading:
            //   - PENDING (newly scanned or manually imported)
            //   - FAILED  (previous attempt hit an error — retry)
            val pending = db.photoDao().getByStatus(SyncStatus.PENDING) +
                          db.photoDao().getByStatus(SyncStatus.FAILED)

            // Pre-filter: skip photos that already have a server ID from a
            // previous successful upload (shouldn't normally happen, but guards
            // against edge cases where status was reset despite a completed upload)
            val genuinelyPending = pending.filter { photo ->
                val alreadyUploaded = photo.serverBlobId != null
                if (alreadyUploaded) {
                    diag.debug(TAG, "Skipping ${photo.filename} — already has server ID", mapOf(
                        "localId" to photo.localId,
                        "serverPhotoId" to (photo.serverPhotoId ?: ""),
                        "serverBlobId" to (photo.serverBlobId ?: "")
                    ))
                    // Fix status back to SYNCED
                    db.photoDao().updateSyncStatus(photo.localId, SyncStatus.SYNCED)
                    false
                } else true
            }

            diag.info(TAG, "Found ${genuinelyPending.size} photos to process", mapOf(
                "pendingCount" to db.photoDao().getByStatus(SyncStatus.PENDING).size.toString(),
                "failedCount" to db.photoDao().getByStatus(SyncStatus.FAILED).size.toString()
            ))

            // Track content hashes uploaded in this session to prevent
            // re-uploading the same content under different filenames.
            val uploadedHashes = mutableSetOf<String>()

            var uploaded = 0
            var skipped = 0
            var failed = 0

            for (photo in genuinelyPending) {
                val localPath = photo.localPath
                if (localPath == null) {
                    diag.warn(TAG, "Photo has no localPath, skipping", mapOf(
                        "localId" to photo.localId,
                        "filename" to photo.filename
                    ))
                    failed++
                    continue
                }

                try {
                    val uri = android.net.Uri.parse(localPath)

                    // Decide the upload path by size BEFORE reading the file. Large
                    // media (videos, the occasional huge photo) streams as a v2
                    // chunked container with bounded heap; buffering it whole here
                    // (read → base64 → JSON → encrypt, ~5×) is the phone-OOM bug
                    // this guards against. Small media keeps the simple v1 path.
                    val sizeBytes = querySize(uri)
                    val useChunked = sizeBytes >= ChunkedBlob.CHUNKED_THRESHOLD_BYTES
                    if (sizeBytes < 0) {
                        diag.warn(TAG, "Could not determine media size — using in-memory v1 path", mapOf(
                            "localId" to photo.localId,
                            "filename" to photo.filename,
                            "uri" to localPath
                        ))
                    }

                    // Small media is read once here (needed for thumbnail, EXIF,
                    // subtype, and the v1 upload). Large media is never buffered —
                    // photoData stays null and every step streams from the URI.
                    val photoData: ByteArray? = if (useChunked) null else {
                        val input = applicationContext.contentResolver.openInputStream(uri)
                        if (input == null) {
                            diag.warn(TAG, "Cannot open content URI — inaccessible", mapOf(
                                "localId" to photo.localId,
                                "filename" to photo.filename,
                                "uri" to localPath
                            ))
                            failed++
                            continue
                        }
                        input.use { it.readBytes() }
                    }

                    // ── Content hash dedup ─────────────────────────────
                    // Short content hash (sha256(original)[..6]) keys cross-session
                    // dedup. Small media hashes the buffered bytes; large media
                    // streams a hash pass so it still never holds the whole file.
                    val digestBytes = if (photoData != null) {
                        java.security.MessageDigest.getInstance("SHA-256").digest(photoData)
                    } else {
                        streamDigest(uri)
                    }
                    if (digestBytes == null) {
                        diag.warn(TAG, "Cannot read content URI for hashing — inaccessible", mapOf(
                            "localId" to photo.localId,
                            "filename" to photo.filename,
                            "uri" to localPath
                        ))
                        failed++
                        continue
                    }
                    val contentHash = digestBytes.take(6).joinToString("") { "%02x".format(it) }

                    val isDupSession = contentHash in uploadedHashes
                    val existingSynced = if (!isDupSession) db.photoDao().getSyncedByHash(contentHash) else null
                    if (isDupSession || existingSynced != null) {
                        // Content already uploaded. If this entity has no server
                        // photo record yet, register it reusing the existing blob
                        // — this handles "Save Copy" duplicates that share the
                        // same raw content but need their own serverPhotoId.
                        val donor = existingSynced ?: db.photoDao().getSyncedByHash(contentHash)
                        if (donor?.serverBlobId != null && photo.serverPhotoId == null) {
                            try {
                                // Identical content ⇒ same subtype. Detect from the
                                // plaintext bytes so dedup copies still classify
                                // (mirrors the main upload path in PhotoRepository).
                                val dedupSubtype = if (photoData != null && (photo.mediaType ?: "photo") == "photo") {
                                    com.simplephotos.data.media.MediaSubtypeDetector.detect(
                                        photoData, photo.width, photo.height
                                    )
                                } else com.simplephotos.data.media.MediaSubtype()
                                val regReq = com.simplephotos.data.remote.dto.RegisterEncryptedPhotoRequest(
                                    filename = photo.filename,
                                    mimeType = photo.mimeType,
                                    mediaType = photo.mediaType ?: "photo",
                                    width = photo.width,
                                    height = photo.height,
                                    durationSecs = photo.durationSecs?.toDouble(),
                                    takenAt = java.time.Instant.ofEpochMilli(photo.takenAt).toString(),
                                    encryptedBlobId = donor.serverBlobId!!,
                                    encryptedThumbBlobId = donor.thumbnailBlobId,
                                    photoHash = contentHash,
                                    photoSubtype = dedupSubtype.photoSubtype,
                                    burstId = dedupSubtype.burstId,
                                    cameraModel = donor.cameraModel
                                )
                                val regRes = api.registerEncryptedPhoto(regReq)
                                db.photoDao().markSynced(
                                    photo.localId, regRes.photoId,
                                    donor.serverBlobId!!, donor.thumbnailBlobId, contentHash
                                )
                                diag.debug(TAG, "Registered copy via hash-dedup: ${photo.filename} → ${regRes.photoId}", mapOf(
                                    "localId" to photo.localId,
                                    "donorId" to donor.localId,
                                    "contentHash" to contentHash
                                ))
                                uploaded++
                            } catch (e: Exception) {
                                diag.warn(TAG, "Failed to register hash-dedup copy: ${e.message}", mapOf(
                                    "localId" to photo.localId,
                                    "contentHash" to contentHash
                                ))
                                db.photoDao().updateSyncStatus(photo.localId, SyncStatus.SYNCED)
                                skipped++
                            }
                        } else {
                            diag.debug(TAG, "Skipping ${photo.filename} — duplicate content hash", mapOf(
                                "localId" to photo.localId,
                                "contentHash" to contentHash
                            ))
                            db.photoDao().updateSyncStatus(photo.localId, SyncStatus.SYNCED)
                            skipped++
                        }
                        if (photo.photoHash == null) {
                            db.photoDao().update(photo.copy(photoHash = contentHash))
                        }
                        continue
                    }

                    // Store hash in entity for future dedup
                    if (photo.photoHash == null) {
                        db.photoDao().update(photo.copy(photoHash = contentHash))
                    }

                    diag.debug(TAG, "Prepared ${photo.filename} for upload", mapOf(
                        "localId" to photo.localId,
                        "filename" to photo.filename,
                        "sizeBytes" to sizeBytes.toString(),
                        "path" to (if (useChunked) "chunked-v2" else "inline-v1"),
                        "mediaType" to (photo.mediaType ?: "unknown"),
                        "mimeType" to photo.mimeType
                    ))

                    // Generate thumbnail based on media type. Thumbnails preserve
                    // the original aspect ratio (longest edge scaled to 512px) so
                    // the justified grid doesn't double-crop portrait photos. Large
                    // (chunked) media has no buffered bytes, so it decodes a
                    // downsampled thumbnail straight from the URI stream.
                    val thumbnailData = when (photo.mediaType) {
                        "video" -> generateVideoThumbnail(uri)
                        "gif" -> if (photoData != null) {
                            // For GIFs, decode first frame
                            val opts = BitmapFactory.Options().apply { inSampleSize = 1 }
                            val bitmap = BitmapFactory.decodeByteArray(photoData, 0, photoData.size, opts)
                            // fitThumbnail recycles the original if it creates a new scaled bitmap;
                            // bitmapToJpeg recycles whatever it receives — no extra recycle needed.
                            bitmapToJpeg(fitThumbnail(bitmap, 512))
                        } else generateDownsampledImageThumbnail(uri)
                        else -> if (photoData != null) {
                            // Photos: decode, apply EXIF rotation, then thumbnail
                            val bitmap = BitmapFactory.decodeByteArray(photoData, 0, photoData.size)
                            val rotated = applyExifRotation(bitmap, photoData)
                            bitmapToJpeg(fitThumbnail(rotated, 512))
                        } else generateDownsampledImageThumbnail(uri)
                    }

                    if (thumbnailData != null) {
                        diag.debug(TAG, "Generated thumbnail (${thumbnailData.size} bytes)", mapOf(
                            "localId" to photo.localId
                        ))
                    } else {
                        diag.warn(TAG, "Thumbnail generation returned null", mapOf(
                            "localId" to photo.localId,
                            "mediaType" to (photo.mediaType ?: "unknown")
                        ))
                    }

                    // Cache thumbnail locally if we were able to generate one
                    if (thumbnailData != null) {
                        val thumbPath = photoRepository.saveThumbnailToDisk(photo.localId, thumbnailData)
                        db.photoDao().updateThumbnailPath(photo.localId, thumbPath)
                    }

                    // Correct width/height for EXIF orientation before uploading.
                    // scanImages() already swaps dimensions for 90°/270° EXIF
                    // rotation, so only swap here if they still appear to be raw
                    // sensor dimensions (width > height for a portrait-EXIF photo).
                    val correctedPhoto = if (photo.mediaType == "photo") {
                        try {
                            val orient = readExifOrientation(uri, photoData)
                            val needsSwap = orient == androidx.exifinterface.media.ExifInterface.ORIENTATION_ROTATE_90
                                    || orient == androidx.exifinterface.media.ExifInterface.ORIENTATION_ROTATE_270
                                    || orient == androidx.exifinterface.media.ExifInterface.ORIENTATION_TRANSPOSE
                                    || orient == androidx.exifinterface.media.ExifInterface.ORIENTATION_TRANSVERSE
                            if (needsSwap && photo.width > 0 && photo.height > 0 && photo.width > photo.height) {
                                photo.copy(width = photo.height, height = photo.width).also {
                                    db.photoDao().update(it)
                                }
                            } else photo
                        } catch (_: Exception) { photo }
                    } else photo

                    // Upload (encrypted mode bundles thumbnail inside the
                    // encrypted payload, so we must have a thumbnail to proceed)
                    if (thumbnailData != null) {
                        diag.info(TAG, "Uploading photo (encrypted, ${if (useChunked) "chunked v2" else "inline v1"})", mapOf(
                            "localId" to correctedPhoto.localId,
                            "filename" to correctedPhoto.filename,
                            "sizeBytes" to sizeBytes.toString()
                        ))
                        if (useChunked) {
                            // Stream straight from the URI to a v2 chunked container —
                            // peak heap ≈ one 4 MiB chunk, no matter the file size.
                            photoRepository.uploadPhotoChunked(
                                correctedPhoto,
                                sizeBytes,
                                openSource = {
                                    applicationContext.contentResolver.openInputStream(uri)
                                        ?: throw java.io.IOException("Cannot open $uri for chunked upload")
                                },
                                thumbnailData = thumbnailData,
                            )
                        } else {
                            // Small media: the buffered bytes are guaranteed present
                            // on this path (useChunked == false ⇒ photoData != null).
                            val data = requireNotNull(photoData) { "inline upload path requires buffered bytes" }
                            photoRepository.uploadPhoto(correctedPhoto, data, thumbnailData)
                        }
                        uploadedHashes.add(contentHash)
                        diag.info(TAG, "Upload succeeded", mapOf(
                            "localId" to photo.localId,
                            "filename" to photo.filename
                        ))
                        uploaded++
                    } else {
                        diag.warn(TAG, "Skipping upload — requires thumbnail", mapOf(
                            "localId" to photo.localId,
                            "filename" to photo.filename
                        ))
                        failed++
                    }
                } catch (e: Exception) {
                    diag.error(TAG, "Upload failed: ${e.message}", mapOf(
                        "localId" to photo.localId,
                        "filename" to photo.filename,
                        "exception" to (e::class.simpleName ?: "Unknown"),
                        "errorDetail" to (e.message ?: "no message")
                    ))
                    failed++
                    // uploadPhoto already sets FAILED status internally
                }
            }

            diag.info(TAG, "Backup worker finished", mapOf(
                "uploaded" to uploaded.toString(),
                "skipped" to skipped.toString(),
                "failed" to failed.toString(),
                "totalProcessed" to genuinelyPending.size.toString()
            ))

            // Now that this batch's frames are all registered (with camera_model),
            // ask the server to timestamp-group bursts that carry no XMP BurstID
            // (Samsung et al.). Best-effort — a failure here must not fail backup.
            if (uploaded > 0) {
                try {
                    val res = api.detectBursts()
                    diag.info(TAG, "Burst detection done", mapOf(
                        "burstGroupsCreated" to res.burstGroupsCreated.toString()
                    ))
                } catch (e: Exception) {
                    diag.warn(TAG, "Burst detection request failed: ${e.message}", emptyMap())
                }
            }

            // Re-register the reactive content-URI observer so the next
            // new photo/video also triggers a backup automatically.
            val wifiOnly = prefs[KEY_WIFI_ONLY_BACKUP] ?: true
            SyncScheduler.rescheduleReactive(applicationContext, wifiOnly)

            diag.flush()
            Result.success()
        } catch (e: Exception) {
            diag.error(TAG, "Backup worker crashed: ${e.message}", mapOf(
                "exception" to (e::class.simpleName ?: "Unknown"),
                "errorDetail" to (e.stackTraceToString().take(1000))
            ))
            diag.flush()
            Log.e(TAG, "Backup worker failed", e)
            Result.retry()
        }
    }

    /**
     * Generate a JPEG thumbnail from a video by seeking to ~10% of duration.
     */
    private fun generateVideoThumbnail(uri: android.net.Uri): ByteArray? {
        return try {
            val retriever = MediaMetadataRetriever()
            retriever.setDataSource(applicationContext, uri)
            val durationMs = retriever.extractMetadata(
                MediaMetadataRetriever.METADATA_KEY_DURATION
            )?.toLongOrNull() ?: 0L
            // Seek to 10% of duration, minimum 1 second
            val seekTimeUs = (maxOf(durationMs * 100, 1000_000L)) // microseconds
            val frame = retriever.getFrameAtTime(seekTimeUs, MediaMetadataRetriever.OPTION_CLOSEST_SYNC)
            retriever.release()

            frame?.let { bitmapToJpeg(fitThumbnail(it, 512)) }
        } catch (e: Exception) {
            Log.w(TAG, "Video thumbnail generation failed", e)
            null
        }
    }

    /**
     * Scale a bitmap so the longest edge is at most [maxEdge] pixels,
     * preserving the original aspect ratio.  Unlike ThumbnailUtils.extractThumbnail
     * (which center-crops to a square), this produces a non-square thumbnail
     * that matches the source's proportions — avoiding double-crop in the
     * justified grid.
     */
    private fun fitThumbnail(bitmap: Bitmap?, maxEdge: Int): Bitmap? {
        bitmap ?: return null
        val w = bitmap.width
        val h = bitmap.height
        if (w <= 0 || h <= 0) return bitmap
        val scale = maxEdge.toFloat() / maxOf(w, h)
        if (scale >= 1f) return bitmap
        val tw = (w * scale).toInt().coerceAtLeast(1)
        val th = (h * scale).toInt().coerceAtLeast(1)
        val scaled = Bitmap.createScaledBitmap(bitmap, tw, th, true)
        if (scaled !== bitmap) bitmap.recycle()
        return scaled
    }

    /**
     * Read EXIF orientation from raw image bytes and apply the matching
     * rotation/flip to the decoded bitmap so thumbnails are correctly oriented.
     */
    private fun applyExifRotation(bitmap: Bitmap?, imageBytes: ByteArray): Bitmap? {
        bitmap ?: return null
        val orientation = try {
            ExifInterface(java.io.ByteArrayInputStream(imageBytes))
                .getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
        } catch (_: Exception) {
            return bitmap
        }
        return rotateBitmap(bitmap, orientation)
    }

    /**
     * Apply an EXIF [orientation] (rotation/flip) to [bitmap], recycling the
     * source when a new bitmap is produced. Shared by the in-memory
     * ([applyExifRotation]) and streamed ([generateDownsampledImageThumbnail])
     * thumbnail paths.
     */
    private fun rotateBitmap(bitmap: Bitmap?, orientation: Int): Bitmap? {
        bitmap ?: return null
        val matrix = Matrix()
        when (orientation) {
            ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> matrix.setScale(-1f, 1f)
            ExifInterface.ORIENTATION_ROTATE_180 -> matrix.setRotate(180f)
            ExifInterface.ORIENTATION_FLIP_VERTICAL -> matrix.setScale(1f, -1f)
            ExifInterface.ORIENTATION_TRANSPOSE -> { matrix.setRotate(90f); matrix.postScale(-1f, 1f) }
            ExifInterface.ORIENTATION_ROTATE_90 -> matrix.setRotate(90f)
            ExifInterface.ORIENTATION_TRANSVERSE -> { matrix.setRotate(270f); matrix.postScale(-1f, 1f) }
            ExifInterface.ORIENTATION_ROTATE_270 -> matrix.setRotate(270f)
            else -> return bitmap
        }
        return try {
            val rotated = Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, matrix, true)
            if (rotated !== bitmap) bitmap.recycle()
            rotated
        } catch (_: Exception) {
            bitmap
        }
    }

    /**
     * Read EXIF orientation either from already-buffered [buffered] bytes (small
     * media) or by streaming the header from [uri] (large media) — ExifInterface
     * only reads the file head, so a multi-GB source is never buffered.
     */
    private fun readExifOrientation(uri: android.net.Uri, buffered: ByteArray?): Int {
        return try {
            val stream = if (buffered != null) {
                java.io.ByteArrayInputStream(buffered)
            } else {
                applicationContext.contentResolver.openInputStream(uri)
                    ?: return ExifInterface.ORIENTATION_NORMAL
            }
            stream.use {
                ExifInterface(it).getAttributeInt(
                    ExifInterface.TAG_ORIENTATION,
                    ExifInterface.ORIENTATION_NORMAL
                )
            }
        } catch (_: Exception) {
            ExifInterface.ORIENTATION_NORMAL
        }
    }

    /**
     * Decode a downsampled, EXIF-rotated thumbnail straight from a large image's
     * URI stream — without ever holding the full-resolution bitmap (or the file
     * bytes) in memory. Two passes: bounds-only to pick an [calculateInSampleSize]
     * factor, then a sampled decode capped near 1024px before fitting to 512px.
     */
    private fun generateDownsampledImageThumbnail(uri: android.net.Uri): ByteArray? {
        return try {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            applicationContext.contentResolver.openInputStream(uri)?.use {
                BitmapFactory.decodeStream(it, null, bounds)
            }
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
                Log.w(TAG, "generateDownsampledImageThumbnail: could not read bounds for $uri")
                return null
            }
            val opts = BitmapFactory.Options().apply {
                inSampleSize = calculateInSampleSize(bounds.outWidth, bounds.outHeight, 1024)
            }
            val sampled = applicationContext.contentResolver.openInputStream(uri)?.use {
                BitmapFactory.decodeStream(it, null, opts)
            } ?: return null
            val rotated = rotateBitmap(sampled, readExifOrientation(uri, null))
            bitmapToJpeg(fitThumbnail(rotated, 512))
        } catch (e: Exception) {
            Log.w(TAG, "generateDownsampledImageThumbnail failed for $uri", e)
            null
        }
    }

    /**
     * Largest power-of-two sample factor that keeps the decoded longest edge at or
     * above [targetMaxEdge] — the standard BitmapFactory downsample idiom.
     */
    private fun calculateInSampleSize(width: Int, height: Int, targetMaxEdge: Int): Int {
        var sample = 1
        var longest = maxOf(width, height)
        while (longest / 2 >= targetMaxEdge) {
            longest /= 2
            sample *= 2
        }
        return sample
    }

    /**
     * Resolve a media item's byte size without reading its contents. Prefers the
     * MediaStore `SIZE` column, falling back to the file-descriptor length.
     * Returns `-1` when the size can't be determined (caller treats that as the
     * conservative in-memory v1 path).
     */
    private fun querySize(uri: android.net.Uri): Long {
        try {
            applicationContext.contentResolver.query(
                uri, arrayOf(android.provider.OpenableColumns.SIZE), null, null, null
            )?.use { c ->
                val idx = c.getColumnIndex(android.provider.OpenableColumns.SIZE)
                if (idx >= 0 && c.moveToFirst() && !c.isNull(idx)) return c.getLong(idx)
            }
        } catch (e: Exception) {
            Log.w(TAG, "querySize: column query failed for $uri: ${e.message}")
        }
        return try {
            applicationContext.contentResolver.openAssetFileDescriptor(uri, "r")?.use { afd ->
                val len = afd.length
                if (len == android.content.res.AssetFileDescriptor.UNKNOWN_LENGTH) -1L else len
            } ?: -1L
        } catch (e: Exception) {
            Log.w(TAG, "querySize: fd length failed for $uri: ${e.message}")
            -1L
        }
    }

    /**
     * Stream the full SHA-256 digest of a media item from [uri] without buffering
     * it — used to compute the dedup content hash for large media. Returns null if
     * the URI can't be opened.
     */
    private fun streamDigest(uri: android.net.Uri): ByteArray? {
        return try {
            applicationContext.contentResolver.openInputStream(uri)?.use { input ->
                val md = java.security.MessageDigest.getInstance("SHA-256")
                val buf = ByteArray(64 * 1024)
                while (true) {
                    val n = input.read(buf)
                    if (n < 0) break
                    md.update(buf, 0, n)
                }
                md.digest()
            }
        } catch (e: Exception) {
            Log.w(TAG, "streamDigest failed for $uri: ${e.message}")
            null
        }
    }

    private fun bitmapToJpeg(bitmap: Bitmap?): ByteArray? {
        bitmap ?: return null
        val stream = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.JPEG, 80, stream)
        bitmap.recycle()
        return stream.toByteArray()
    }

    /**
     * Detect server reset / data loss: if the server has no encrypted photos
     * but we have locally SYNCED entries with serverBlobId, the server was
     * wiped after our last sync.  Reset those entries to PENDING (clear their
     * server IDs) so they get re-uploaded on this run.
     */
    private suspend fun reconcileSyncedWithServer(diag: DiagnosticLogger) {
        val synced = db.photoDao().getByStatus(SyncStatus.SYNCED)
        val withServerBlob = synced.filter { it.serverBlobId != null && it.localPath != null }
        if (withServerBlob.isEmpty()) return  // nothing to reconcile

        try {
            // Quick check: fetch the first page of the server's encrypted-sync.
            // If the server has zero encrypted photos, all local SYNCED entries
            // are stale.
            val page = api.encryptedSync(limit = 1)
            if (page.photos.isNotEmpty()) return  // server still has data — nothing to do

            // Server has zero photos — reset local entries
            diag.info(TAG, "Server appears empty but we have ${withServerBlob.size} SYNCED photos — resetting to PENDING for re-upload")
            for (photo in withServerBlob) {
                db.photoDao().update(photo.copy(
                    syncStatus = SyncStatus.PENDING,
                    serverBlobId = null,
                    thumbnailBlobId = null,
                    serverPhotoId = null
                ))
            }
        } catch (e: Exception) {
            diag.warn(TAG, "Reconcile check failed: ${e.message}")
            // Non-fatal — skip reconciliation this run
        }
    }

    /**
     * One-time repair: scan all SYNCED photos in the local DB, read EXIF
     * orientation from the original file, and correct width/height if the
     * raw pixel dimensions were stored instead of display dimensions.
     * Sends corrected dimensions to the server in a batch update.
     */
    private suspend fun repairExifDimensions(diag: DiagnosticLogger) {
        val prefs = dataStore.data.first()
        val alreadyDone = prefs[EXIF_DIM_REPAIR_DONE] ?: false
        if (alreadyDone) return

        diag.info(TAG, "Starting one-time EXIF dimension repair")

        val synced = db.photoDao().getByStatus(SyncStatus.SYNCED)
        val photosToFix = synced.filter {
            it.localPath != null && it.serverBlobId != null
                    && it.mediaType == "photo"
                    && it.width > 0 && it.height > 0
        }

        if (photosToFix.isEmpty()) {
            diag.info(TAG, "EXIF dimension repair: no photos to check")
            dataStore.edit { it[EXIF_DIM_REPAIR_DONE] = true }
            return
        }

        val serverUpdates = mutableListOf<com.simplephotos.data.remote.dto.DimensionUpdateItem>()

        for (photo in photosToFix) {
            try {
                val uri = android.net.Uri.parse(photo.localPath)
                val inputStream = applicationContext.contentResolver.openInputStream(uri) ?: continue
                val bytes = inputStream.use { it.readBytes() }

                val exif = ExifInterface(java.io.ByteArrayInputStream(bytes))
                val orient = exif.getAttributeInt(
                    ExifInterface.TAG_ORIENTATION,
                    ExifInterface.ORIENTATION_NORMAL
                )
                val needsSwap = orient == ExifInterface.ORIENTATION_ROTATE_90
                        || orient == ExifInterface.ORIENTATION_ROTATE_270
                        || orient == ExifInterface.ORIENTATION_TRANSPOSE
                        || orient == ExifInterface.ORIENTATION_TRANSVERSE

                if (needsSwap && photo.width > photo.height) {
                    // Width/height are currently raw pixels (W > H for a portrait
                    // EXIF photo); swap to display dimensions
                    val corrected = photo.copy(width = photo.height, height = photo.width)
                    db.photoDao().update(corrected)
                    serverUpdates.add(
                        com.simplephotos.data.remote.dto.DimensionUpdateItem(
                            blobId = photo.serverBlobId,
                            width = corrected.width,
                            height = corrected.height
                        )
                    )
                }
            } catch (e: Exception) {
                Log.w(TAG, "EXIF dim repair: failed for ${photo.filename}: ${e.message}")
            }
        }

        // Batch-send to server
        if (serverUpdates.isNotEmpty()) {
            try {
                val resp = api.batchUpdateDimensions(
                    com.simplephotos.data.remote.dto.BatchDimensionUpdateRequest(serverUpdates)
                )
                diag.info(TAG, "EXIF dimension repair: fixed ${serverUpdates.size} locally, ${resp.updated} on server")
            } catch (e: Exception) {
                diag.warn(TAG, "EXIF dimension repair: server update failed: ${e.message}")
                // Don't mark as done so it retries next time
                return
            }
        } else {
            diag.info(TAG, "EXIF dimension repair: all dimensions already correct")
        }

        dataStore.edit { it[EXIF_DIM_REPAIR_DONE] = true }
    }
}
