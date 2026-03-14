/**
 * Hilt module providing app-level singletons such as the Room database,
 * DataStore, CryptoManager, and KeyManager.
 */
package com.simplephotos.di

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.preferencesDataStore
import androidx.room.Room
import com.simplephotos.crypto.CryptoManager
import com.simplephotos.crypto.KeyManager
import com.simplephotos.data.local.AppDatabase
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(name = "settings")

/**
 * Hilt module providing application-scoped singletons: DataStore, Room database,
 * [KeyManager], and [CryptoManager].
 */
@Module
@InstallIn(SingletonComponent::class)
object AppModule {

    @Provides
    @Singleton
    fun provideDataStore(@ApplicationContext context: Context): DataStore<Preferences> =
        context.dataStore

    @Provides
    @Singleton
    fun provideDatabase(@ApplicationContext context: Context): AppDatabase =
        Room.databaseBuilder(context, AppDatabase::class.java, "simple-photos.db")
            .fallbackToDestructiveMigration()
            .build()

    @Provides
    @Singleton
    fun provideKeyManager(@ApplicationContext context: Context): KeyManager =
        KeyManager(context)

    @Provides
    @Singleton
    fun provideCryptoManager(keyManager: KeyManager): CryptoManager =
        CryptoManager(keyManager)
}
