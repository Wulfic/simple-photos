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
                    val inputStream = applicationContext.contentResolver.openInputStream(uri)
                    if (inputStream == null) {
                        diag.warn(TAG, "Cannot open content URI — inaccessible", mapOf(
                            "localId" to photo.localId,
                            "filename" to photo.filename,
                            "uri" to localPath
                        ))
                        failed++
                        continue
                    }
                    val photoData = inputStream.use { it.readBytes() }

                    // ── Content hash dedup ─────────────────────────────
                    // Compute a short content hash and check if we've already
                    // uploaded identical content in this session or a previous one.
                    val contentHash = java.security.MessageDigest.getInstance("SHA-256")
                        .digest(photoData)
                        .take(6)
                        .joinToString("") { "%02x".format(it) }

                    if (contentHash in uploadedHashes) {
                        diag.debug(TAG, "Skipping ${photo.filename} — duplicate content hash in session", mapOf(
                            "localId" to photo.localId,
                            "contentHash" to contentHash
                        ))
                        db.photoDao().updateSyncStatus(photo.localId, SyncStatus.SYNCED)
                        skipped++
                        continue
                    }

                    // Check against previously synced photos with the same hash
                    val existingSynced = db.photoDao().getSyncedByHash(contentHash)
                    if (existingSynced != null) {
                        diag.debug(TAG, "Skipping ${photo.filename} — matches synced photo ${existingSynced.filename}", mapOf(
                            "localId" to photo.localId,
                            "matchedId" to existingSynced.localId,
                            "contentHash" to contentHash
                        ))
                        db.photoDao().updateSyncStatus(photo.localId, SyncStatus.SYNCED)
                        skipped++
                        continue
                    }

                    // Store hash in entity for future dedup
                    if (photo.photoHash == null) {
                        db.photoDao().update(photo.copy(photoHash = contentHash))
                    }

                    diag.debug(TAG, "Read ${photoData.size} bytes from content URI", mapOf(
                        "localId" to photo.localId,
                        "filename" to photo.filename,
                        "sizeBytes" to photoData.size.toString(),
                        "mediaType" to (photo.mediaType ?: "unknown"),
                        "mimeType" to photo.mimeType
                    ))

                    // Generate thumbnail based on media type
                    val thumbnailData = when (photo.mediaType) {
                        "video" -> generateVideoThumbnail(uri)
                        "gif" -> {
                            // For GIFs, decode first frame
                            val opts = BitmapFactory.Options().apply { inSampleSize = 1 }
                            val bitmap = BitmapFactory.decodeByteArray(photoData, 0, photoData.size, opts)
                            bitmapToJpeg(ThumbnailUtils.extractThumbnail(bitmap, 256, 256)).also {
                                bitmap?.recycle()
                            }
                        }
                        else -> {
                            // Photos: decode, apply EXIF rotation, then thumbnail
                            val bitmap = BitmapFactory.decodeByteArray(photoData, 0, photoData.size)
                            val rotated = applyExifRotation(bitmap, photoData)
                            bitmapToJpeg(ThumbnailUtils.extractThumbnail(rotated, 256, 256)).also {
                                rotated?.recycle()
                            }
                        }
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
                    val correctedPhoto = if (photo.mediaType == "photo" || photo.mediaType == null) {
                        try {
                            val exif = androidx.exifinterface.media.ExifInterface(photoData.inputStream())
                            val orient = exif.getAttributeInt(
                                androidx.exifinterface.media.ExifInterface.TAG_ORIENTATION,
                                androidx.exifinterface.media.ExifInterface.ORIENTATION_NORMAL
                            )
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
                        diag.info(TAG, "Uploading photo (encrypted)", mapOf(
                            "localId" to correctedPhoto.localId,
                            "filename" to correctedPhoto.filename,
                            "sizeBytes" to photoData.size.toString()
                        ))
                        photoRepository.uploadPhoto(correctedPhoto, photoData, thumbnailData)
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

            frame?.let { bitmapToJpeg(ThumbnailUtils.extractThumbnail(it, 256, 256)) }
        } catch (e: Exception) {
            Log.w(TAG, "Video thumbnail generation failed", e)
            null
        }
    }

    /**
     * Read EXIF orientation from raw image bytes and apply the matching
     * rotation/flip to the decoded bitmap so thumbnails are correctly oriented.
     */
    private fun applyExifRotation(bitmap: Bitmap?, imageBytes: ByteArray): Bitmap? {
        bitmap ?: return null
        return try {
            val exif = ExifInterface(java.io.ByteArrayInputStream(imageBytes))
            val orientation = exif.getAttributeInt(
                ExifInterface.TAG_ORIENTATION,
                ExifInterface.ORIENTATION_NORMAL
            )
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
            val rotated = Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, matrix, true)
            if (rotated !== bitmap) bitmap.recycle()
            rotated
        } catch (_: Exception) {
            bitmap
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
                    && (it.mediaType == "photo" || it.mediaType == null)
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
