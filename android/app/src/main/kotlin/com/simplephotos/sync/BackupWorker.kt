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

            // Step 2: Mark exhausted queue entries as failed
            db.blobQueueDao().markExhaustedAsFailed()

            // Step 3: Process pending photos by priority (thumbnail=0, media=1, album=2)
            val pending = db.photoDao().getByStatus(SyncStatus.PENDING)

            for (photo in pending) {
                val localPath = photo.localPath ?: continue

                try {
                    val uri = android.net.Uri.parse(localPath)
                    val inputStream: InputStream = applicationContext.contentResolver.openInputStream(uri)
                        ?: continue
                    val photoData = inputStream.readBytes()
                    inputStream.close()

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
                        // Cache thumbnail locally before uploading
                        photoRepository.saveThumbnailToDisk(photo.localId, thumbnailData)
                        photoRepository.uploadPhoto(photo, photoData, thumbnailData)
                    } else {
                        Log.w(TAG, "Failed to generate thumbnail for ${photo.localId}, skipping")
                    }
                } catch (e: Exception) {
                    Log.e(TAG, "Failed to upload ${photo.localId}: ${e.message}", e)
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
