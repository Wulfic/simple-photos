package com.simplephotos

import android.app.Application
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.hilt.work.HiltWorkerFactory
import androidx.work.Configuration
import coil.ImageLoader
import coil.ImageLoaderFactory
import com.simplephotos.ui.theme.ThemeState
import dagger.hilt.android.HiltAndroidApp
import okhttp3.OkHttpClient
import javax.inject.Inject

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
            .build()
    }
}
