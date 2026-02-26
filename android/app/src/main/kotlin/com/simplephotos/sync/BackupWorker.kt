package com.simplephotos.sync

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadataRetriever
import android.media.ThumbnailUtils
import android.util.Log
import androidx.hilt.work.HiltWorker
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SyncRepository
import dagger.assisted.Assisted
import dagger.assisted.AssistedInject
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.InputStream

@HiltWorker
class BackupWorker @AssistedInject constructor(
    @Assisted context: Context,
    @Assisted params: WorkerParameters,
    private val photoRepository: PhotoRepository,
    private val syncRepository: SyncRepository,
    private val db: AppDatabase
) : CoroutineWorker(context, params) {

    companion object {
        private const val TAG = "BackupWorker"
        private const val MAX_ATTEMPTS = 5
    }

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        try {
            // Step 1: Scan MediaStore for new photos and videos
            syncRepository.scanForNewMedia()

            // Step 2: Recover stuck uploads — photos left at UPLOADING from a crash
            // are reset to PENDING so they get retried.
            db.photoDao().resetStuckUploading()

            // Step 3: Mark exhausted queue entries as failed
            db.blobQueueDao().markExhaustedAsFailed()

            // Step 4: Fetch the set of filenames already on the server so we
            // can skip re-uploading photos that are already backed up.
            val mode = photoRepository.getEncryptionMode()
            val serverFilenames: Set<String> = if (mode == "plain") {
                photoRepository.getServerFilenames()
            } else {
                emptySet() // encrypted mode uses blob IDs, not filenames
            }

            // Step 5: Process photos that need uploading:
            //   - PENDING (newly scanned or manually imported)
            //   - FAILED  (previous attempt hit an error — retry)
            val pending = db.photoDao().getByStatus(SyncStatus.PENDING) +
                          db.photoDao().getByStatus(SyncStatus.FAILED)

            for (photo in pending) {
                // ── Server-side dedup ──────────────────────────────────
                // If the server already has a photo with this exact filename,
                // mark it SYNCED locally and skip the upload entirely.
                if (mode == "plain" && photo.filename in serverFilenames) {
                    Log.i(TAG, "Skipping ${photo.filename} — already exists on server")
                    db.photoDao().updateSyncStatus(photo.localId, SyncStatus.SYNCED)
                    continue
                }

                val localPath = photo.localPath ?: continue

                try {
                    val uri = android.net.Uri.parse(localPath)
                    val inputStream = applicationContext.contentResolver.openInputStream(uri)
                    if (inputStream == null) {
                        Log.w(TAG, "Cannot open ${photo.localId} — content URI inaccessible, skipping")
                        continue
                    }
                    val photoData = inputStream.use { it.readBytes() }

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
                            // Photos: standard bitmap thumbnail
                            val bitmap = BitmapFactory.decodeByteArray(photoData, 0, photoData.size)
                            bitmapToJpeg(ThumbnailUtils.extractThumbnail(bitmap, 256, 256)).also {
                                bitmap?.recycle()
                            }
                        }
                    }

                    if (thumbnailData != null) {
                        // Cache thumbnail locally and update DB path so the gallery can show it
                        val thumbPath = photoRepository.saveThumbnailToDisk(photo.localId, thumbnailData)
                        db.photoDao().updateThumbnailPath(photo.localId, thumbPath)

                        // Upload based on encryption mode
                        if (mode == "plain") {
                            photoRepository.uploadPhotoPlain(photo, photoData)
                        } else {
                            photoRepository.uploadPhoto(photo, photoData, thumbnailData)
                        }
                        Log.i(TAG, "Uploaded ${photo.localId} (${photo.filename}) successfully")
                    } else {
                        Log.w(TAG, "Failed to generate thumbnail for ${photo.localId}, skipping")
                    }
                } catch (e: Exception) {
                    Log.e(TAG, "Failed to upload ${photo.localId}: ${e.message}", e)
                    // uploadPhotoPlain / uploadPhoto already set FAILED status internally
                }
            }

            Result.success()
        } catch (e: Exception) {
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

    private fun bitmapToJpeg(bitmap: Bitmap?): ByteArray? {
        bitmap ?: return null
        val stream = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.JPEG, 80, stream)
        bitmap.recycle()
        return stream.toByteArray()
    }
}
