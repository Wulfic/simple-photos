/**
 * Activity / processing-status repository — combined progress for
 * upload, conversion, encryption, AI, and geo pipelines.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.ActivityStatusResponse
import com.simplephotos.data.remote.dto.TranscodeStatusResponse
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class ActivityRepository @Inject constructor(private val api: ApiService) {

    suspend fun status(): ActivityStatusResponse = api.getActivityStatus()

    suspend fun transcode(): TranscodeStatusResponse = api.getTranscodeStatus()
}
