/**
 * Hilt module providing the OkHttpClient, Retrofit instance, and ApiService
 * with automatic token refresh and server-URL resolution.
 */
package com.simplephotos.di

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import com.simplephotos.data.remote.ApiService
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
        // Dynamic base URL interceptor — reads server URL from DataStore on each request
        // so it works immediately after the user configures a server, without restart.
        val dynamicBaseUrlInterceptor = Interceptor { chain ->
            val prefs = runBlocking { dataStore.data.first() }
            val serverUrl = (prefs[KEY_SERVER_URL] ?: "http://localhost:8080").trimEnd('/')
            val originalRequest = chain.request()
            val originalUrl = originalRequest.url

            // Parse the configured server URL
            val newBaseUrl = "$serverUrl/".toHttpUrlOrNull()
            if (newBaseUrl != null) {
                // Rebuild URL: replace scheme + host + port, keep the path
                val newUrl = originalUrl.newBuilder()
                    .scheme(newBaseUrl.scheme)
                    .host(newBaseUrl.host)
                    .port(newBaseUrl.port)
                    .build()
                val newRequest = originalRequest.newBuilder().url(newUrl).build()
                chain.proceed(newRequest)
            } else {
                chain.proceed(originalRequest)
            }
        }

        // Interceptor: inject Bearer token + X-Requested-With on every request
        val authInterceptor = Interceptor { chain ->
            val prefs = runBlocking { dataStore.data.first() }
            val token = prefs[KEY_ACCESS_TOKEN]
            val builder = chain.request().newBuilder()
                .addHeader("X-Requested-With", "SimplePhotos")
            if (token != null) {
                builder.addHeader("Authorization", "Bearer $token")
            }
            chain.proceed(builder.build())
        }

        // Authenticator: 401 → refresh token → retry once
        val tokenAuthenticator = Authenticator { _, response ->
            // Only retry once — check if we already tried refreshing
            if (response.request.header("X-Retry-After-Refresh") != null) return@Authenticator null

            val prefs = runBlocking { dataStore.data.first() }
            val refreshToken = prefs[KEY_REFRESH_TOKEN] ?: return@Authenticator null
            val serverUrl = (prefs[KEY_SERVER_URL] ?: "http://localhost:8080").trimEnd('/') + "/"

            try {
                // Build a temporary Retrofit just for the refresh call
                val tempClient = OkHttpClient.Builder()
                    .connectTimeout(10, TimeUnit.SECONDS)
                    .readTimeout(10, TimeUnit.SECONDS)
                    .build()
                val tempRetrofit = Retrofit.Builder()
                    .baseUrl(serverUrl)
                    .client(tempClient)
                    .addConverterFactory(GsonConverterFactory.create())
                    .build()
                val tempApi = tempRetrofit.create(ApiService::class.java)

                val refreshResponse = runBlocking {
                    tempApi.refresh(RefreshRequest(refreshToken))
                }

                // Store new access token
                runBlocking {
                    dataStore.edit { it[KEY_ACCESS_TOKEN] = refreshResponse.accessToken }
                }

                // Retry original request with new token
                response.request.newBuilder()
                    .header("Authorization", "Bearer ${refreshResponse.accessToken}")
                    .header("X-Retry-After-Refresh", "true")
                    .build()
            } catch (_: Exception) {
                null // Refresh failed — let the 401 propagate
            }
        }

        return OkHttpClient.Builder()
            .addInterceptor(dynamicBaseUrlInterceptor)
            .addInterceptor(authInterceptor)
            .authenticator(tokenAuthenticator)
            .addInterceptor(HttpLoggingInterceptor().apply {
                level = HttpLoggingInterceptor.Level.HEADERS
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
