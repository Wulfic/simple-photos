/**
 * Hilt module providing the OkHttpClient, Retrofit instance, and ApiService
 * with automatic token refresh and server-URL resolution.
 */
package com.simplephotos.di

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.GalleryTokenHolder
import com.simplephotos.data.remote.SessionRefresher
import com.simplephotos.data.remote.dto.RefreshRequest
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_ACCESS_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_REFRESH_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import okhttp3.Authenticator
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.Interceptor
import okhttp3.ConnectionPool
import okhttp3.OkHttpClient
import okhttp3.logging.HttpLoggingInterceptor
import retrofit2.Retrofit
import retrofit2.converter.gson.GsonConverterFactory
import java.util.concurrent.TimeUnit
import javax.inject.Singleton

/**
 * Hilt module providing the OkHttp client, Retrofit instance, and [ApiService].
 *
 * Features:
 * - Dynamic base-URL interceptor (reads server URL from DataStore per request).
 * - Bearer token injection on every request.
 * - Transparent 401 → refresh-token → retry authenticator.
 */
@Module
@InstallIn(SingletonComponent::class)
object NetworkModule {

    @Provides
    @Singleton
    fun provideOkHttpClient(dataStore: DataStore<Preferences>): OkHttpClient {
        // Single per-request interceptor: rewrites the base URL to the
        // configured server AND injects auth headers. Merged into one so we
        // do a single blocking DataStore read per request instead of two.
        val requestInterceptor = Interceptor { chain ->
            val prefs = runBlocking { dataStore.data.first() }
            val serverUrl = (prefs[KEY_SERVER_URL] ?: "http://localhost:8080").trimEnd('/')
            val token = prefs[KEY_ACCESS_TOKEN]

            val originalRequest = chain.request()
            val builder = originalRequest.newBuilder()
                .addHeader("X-Requested-With", "SimplePhotos")
            if (token != null) {
                builder.addHeader("Authorization", "Bearer $token")
            }
            // Secure-album gate: attach the unlock token (when a secure album is
            // unlocked) so requests for secure-album media pass the server's
            // check. Ignored by the server for non-secure endpoints.
            GalleryTokenHolder.token?.let { builder.addHeader("X-Gallery-Token", it) }

            // Rewrite scheme + host + port to the configured server, keep the path.
            "$serverUrl/".toHttpUrlOrNull()?.let { base ->
                val newUrl = originalRequest.url.newBuilder()
                    .scheme(base.scheme)
                    .host(base.host)
                    .port(base.port)
                    .build()
                builder.url(newUrl)
            }

            chain.proceed(builder.build())
        }

        // A separate client for /auth/refresh: it must not go through the
        // authenticator below (a 401 there would recurse), and a short timeout
        // keeps a dead server from holding the refresh lock for two minutes.
        val refreshClient = OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(10, TimeUnit.SECONDS)
            .build()

        // Policy (single-flight, dead-token memory, transient cooldown) lives in
        // SessionRefresher so it is JVM-tested; this is only the I/O.
        val refresher = SessionRefresher(
            readTokens = {
                val prefs = runBlocking { dataStore.data.first() }
                SessionRefresher.Tokens(prefs[KEY_ACCESS_TOKEN], prefs[KEY_REFRESH_TOKEN])
            },
            callRefresh = { refreshToken ->
                val prefs = runBlocking { dataStore.data.first() }
                val serverUrl = (prefs[KEY_SERVER_URL] ?: "http://localhost:8080").trimEnd('/') + "/"
                val api = Retrofit.Builder()
                    .baseUrl(serverUrl)
                    .client(refreshClient)
                    .addConverterFactory(GsonConverterFactory.create())
                    .build()
                    .create(ApiService::class.java)
                runBlocking { api.refresh(RefreshRequest(refreshToken)) }
            },
            saveTokens = { access, refresh ->
                runBlocking {
                    dataStore.edit {
                        it[KEY_ACCESS_TOKEN] = access
                        refresh?.let { rt -> it[KEY_REFRESH_TOKEN] = rt }
                    }
                }
            },
        )

        // Authenticator: 401 → (maybe) refresh → retry once
        val tokenAuthenticator = Authenticator { _, response ->
            if (response.request.header("X-Retry-After-Refresh") != null) return@Authenticator null
            val requestAccess = response.request.header("Authorization")
                ?.removePrefix("Bearer ")?.trim()
            refresher.authenticate(requestAccess)?.let { access ->
                response.request.newBuilder()
                    .header("Authorization", "Bearer $access")
                    .header("X-Retry-After-Refresh", "true")
                    .build()
            }
        }

        return OkHttpClient.Builder()
            .addInterceptor(requestInterceptor)
            .authenticator(tokenAuthenticator)
            .addInterceptor(HttpLoggingInterceptor().apply {
                level = HttpLoggingInterceptor.Level.HEADERS
                // Never log bearer tokens or secure-album unlock tokens — they
                // would otherwise land in Logcat in plaintext.
                redactHeader("Authorization")
                redactHeader("X-Gallery-Token")
            })
            .connectionPool(ConnectionPool(
                maxIdleConnections = 12,
                keepAliveDuration = 5,
                timeUnit = TimeUnit.MINUTES
            ))
            .connectTimeout(30, TimeUnit.SECONDS)
            .readTimeout(120, TimeUnit.SECONDS)
            .writeTimeout(120, TimeUnit.SECONDS)
            .build()
    }

    @Provides
    @Singleton
    fun provideRetrofit(client: OkHttpClient): Retrofit {
        // Use a placeholder base URL — the dynamic interceptor replaces it per-request.
        return Retrofit.Builder()
            .baseUrl("http://localhost/")
            .client(client)
            .addConverterFactory(GsonConverterFactory.create())
            .build()
    }

    @Provides
    @Singleton
    fun provideApiService(retrofit: Retrofit): ApiService =
        retrofit.create(ApiService::class.java)
}
