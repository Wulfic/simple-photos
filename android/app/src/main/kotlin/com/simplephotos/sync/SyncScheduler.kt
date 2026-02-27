package com.simplephotos.sync

import android.content.Context
import android.net.Uri
import android.provider.MediaStore
import androidx.work.*
import java.util.concurrent.TimeUnit

object SyncScheduler {
    private const val WORK_NAME = "photo_backup"
    private const val REACTIVE_WORK_NAME = "photo_backup_reactive"

    fun schedule(context: Context, wifiOnly: Boolean = true) {
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(
                if (wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED
            )
            .setRequiresBatteryNotLow(true)
            .build()

        // Periodic fallback every 15 minutes (down from 1 hour)
        val periodicRequest = PeriodicWorkRequestBuilder<BackupWorker>(15, TimeUnit.MINUTES)
            .setConstraints(constraints)
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 10, TimeUnit.MINUTES)
            .build()

        WorkManager.getInstance(context)
            .enqueueUniquePeriodicWork(WORK_NAME, ExistingPeriodicWorkPolicy.KEEP, periodicRequest)

        // Reactive: trigger when new photos/videos appear in MediaStore.
        // Uses content URI triggers so the worker fires within seconds of a
        // photo being taken or a file being saved to the device.
        val reactiveConstraints = Constraints.Builder()
            .setRequiredNetworkType(
                if (wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED
            )
            .addContentUriTrigger(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true)
            .addContentUriTrigger(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, true)
            // Small delay to batch rapid bursts (e.g., burst-mode photos)
            .setTriggerContentUpdateDelay(5, TimeUnit.SECONDS)
            .setTriggerContentMaxDelay(30, TimeUnit.SECONDS)
            .build()

        val reactiveRequest = OneTimeWorkRequestBuilder<BackupWorker>()
            .setConstraints(reactiveConstraints)
            .build()

        WorkManager.getInstance(context)
            .enqueueUniqueWork(REACTIVE_WORK_NAME, ExistingWorkPolicy.REPLACE, reactiveRequest)
    }

    /**
     * Re-register the reactive content-URI observer after each execution.
     * Called by BackupWorker at the end of doWork() so that the next media
     * change is also detected.
     */
    fun rescheduleReactive(context: Context, wifiOnly: Boolean = true) {
        val reactiveConstraints = Constraints.Builder()
            .setRequiredNetworkType(
                if (wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED
            )
            .addContentUriTrigger(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true)
            .addContentUriTrigger(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, true)
            .setTriggerContentUpdateDelay(5, TimeUnit.SECONDS)
            .setTriggerContentMaxDelay(30, TimeUnit.SECONDS)
            .build()

        val reactiveRequest = OneTimeWorkRequestBuilder<BackupWorker>()
            .setConstraints(reactiveConstraints)
            .build()

        WorkManager.getInstance(context)
            .enqueueUniqueWork(REACTIVE_WORK_NAME, ExistingWorkPolicy.REPLACE, reactiveRequest)
    }

    fun triggerNow(context: Context) {
        val request = OneTimeWorkRequestBuilder<BackupWorker>()
            .setConstraints(
                Constraints.Builder()
                    .setRequiredNetworkType(NetworkType.CONNECTED)
                    .build()
            )
            .build()

        WorkManager.getInstance(context).enqueue(request)
    }

    fun cancel(context: Context) {
        WorkManager.getInstance(context).cancelUniqueWork(WORK_NAME)
        WorkManager.getInstance(context).cancelUniqueWork(REACTIVE_WORK_NAME)
    }
}
