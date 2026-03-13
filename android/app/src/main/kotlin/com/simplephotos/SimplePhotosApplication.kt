package com.simplephotos

import android.app.Application
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.hilt.work.HiltWorkerFactory
import androidx.work.Configuration
import coil.ImageLoader
import coil.ImageLoaderFactory
import coil.decode.GifDecoder
import coil.decode.SvgDecoder
import com.simplephotos.ui.theme.ThemeState
import dagger.hilt.android.HiltAndroidApp
import okhttp3.OkHttpClient
import javax.inject.Inject

/**
 * Application subclass that initialises Hilt DI, configures WorkManager with
 * [HiltWorkerFactory], and provides a custom Coil [ImageLoader] backed by the
 * authenticated OkHttp client (required so image loads carry the Bearer token).
 */
@HiltAndroidApp
class SimplePhotosApplication : Application(), Configuration.Provider, ImageLoaderFactory {

    @Inject
    lateinit var workerFactory: HiltWorkerFactory

    @Inject
    lateinit var dataStore: DataStore<Preferences>

    @Inject
    lateinit var okHttpClient: OkHttpClient

    override val workManagerConfiguration: Configuration
        get() = Configuration.Builder()
            .setWorkerFactory(workerFactory)
            .build()

    override fun onCreate() {
        super.onCreate()
        ThemeState.init(dataStore)
    }

    override fun newImageLoader(): ImageLoader {
        // By the time Coil first requests this (on first image load in a
        // Composable), Hilt has already injected okHttpClient in onCreate().
        // Using the authenticated OkHttpClient is essential — without it,
        // Coil makes unauthenticated requests and all images return 401.
        return ImageLoader.Builder(this)
            .crossfade(true)
            .okHttpClient(okHttpClient)
            // Cap the in-memory bitmap cache at 16 MB. We clear this
            // cache whenever a video starts playing (see onVideoUriReady)
            // so the heap freed here is immediately available for the
            // video decoder's output buffers.
            .memoryCachePolicy(coil.request.CachePolicy.ENABLED)
            .memoryCache {
                coil.memory.MemoryCache.Builder(this)
                    .maxSizeBytes(16 * 1024 * 1024) // 16 MB
                    .build()
            }
            .components {
                add(GifDecoder.Factory())
                add(SvgDecoder.Factory())
            }
            .build()
    }
}
