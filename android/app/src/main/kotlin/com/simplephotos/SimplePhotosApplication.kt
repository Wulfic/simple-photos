/**
 * Application-level initialization.
 *
 * Configures Hilt dependency injection, WorkManager with [HiltWorkerFactory]
 * for background backup scheduling, and a custom Coil [ImageLoader] for
 * efficient thumbnail loading with server-authenticated requests.
 */
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
        //
        // Memory cache sizing: 25% of max heap (Coil's recommended default).
        // This is typically 48–128 MB depending on device.  At 256×256 ARGB
        // thumbnails (~256 KB each), that holds 200–500 thumbnails in memory.
        // Full-resolution viewer images are selectively evicted when a video
        // starts (see onVideoUriReady) — thumbnails stay warm so the gallery
        // grid repopulates instantly when the user navigates back.
        val cacheSize = (Runtime.getRuntime().maxMemory() / 4).coerceIn(
            16L * 1024 * 1024,   // floor: 16 MB
            128L * 1024 * 1024   // ceiling: 128 MB
        )
        return ImageLoader.Builder(this)
            .crossfade(true)
            .okHttpClient(okHttpClient)
            .memoryCachePolicy(coil.request.CachePolicy.ENABLED)
            .memoryCache {
                coil.memory.MemoryCache.Builder(this)
                    .maxSizeBytes(cacheSize.toInt())
                    .build()
            }
            .components {
                add(GifDecoder.Factory())
                add(SvgDecoder.Factory())
            }
            .build()
    }
}
