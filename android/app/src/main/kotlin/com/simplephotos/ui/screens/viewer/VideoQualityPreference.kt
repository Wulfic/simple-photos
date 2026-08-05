/**
 * Resolves the two inputs behind the player's default quality (#49): the user's
 * "Cellular data saver" setting and whether the live connection is metered.
 *
 * Split from [com.simplephotos.data.media.isConstrained] on purpose — that
 * function is the *rule* and is unit-tested, this file is the *reading*, which
 * needs a Context and a live radio and therefore cannot be. Keeping the rule out
 * of here is what lets "metered alone never downgrades" be a test rather than a
 * device check.
 *
 * Web has no equivalent: the Network Information API exposes three unreliable
 * signals that have to be ORed and still return nothing on Safari and Firefox,
 * so it guesses. `NET_CAPABILITY_NOT_METERED` is a definitive answer.
 */
package com.simplephotos.ui.screens.viewer

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import com.simplephotos.data.media.isConstrained
import dagger.hilt.EntryPoint
import dagger.hilt.InstallIn
import dagger.hilt.android.EntryPointAccessors
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.flow.map

@EntryPoint
@InstallIn(SingletonComponent::class)
internal interface VideoQualityPrefEntryPoint {
    fun dataStore(): DataStore<Preferences>
}

/**
 * "Cellular data saver".
 *
 * Defaults to **true**: the issue asks for reduced quality on cellular as the
 * expected behaviour, and the failure modes are asymmetric — defaulting off
 * spends a stranger's mobile data on a 4K stream they never asked for, while
 * defaulting on costs a sharper picture until they find the switch.
 */
val KEY_CELLULAR_DATA_SAVER = booleanPreferencesKey("cellular_data_saver")

/** Whether the active network is metered, tracked live. */
@Composable
private fun rememberMetered(): Boolean {
    val context = LocalContext.current
    // Assume unmetered until told otherwise. Combined with the data-saver gate
    // the worst case is one full-quality stream before the callback lands, and
    // the alternative — assuming metered — downgrades every video on wifi for
    // the moment before the first callback, on every single open.
    var metered by remember(context) { mutableStateOf(false) }

    DisposableEffect(context) {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        if (cm == null) {
            onDispose { }
        } else {
            fun read(caps: NetworkCapabilities?) {
                // NOT_METERED absent ⇒ metered. Reading it this way (rather
                // than looking for a CELLULAR transport) also catches a metered
                // wifi hotspot, which is the case a transport check gets wrong
                // and which costs the user exactly the same money.
                metered = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) != true
            }
            read(cm.getNetworkCapabilities(cm.activeNetwork))

            val callback = object : ConnectivityManager.NetworkCallback() {
                override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                    read(caps)
                }

                override fun onLost(network: Network) {
                    read(cm.getNetworkCapabilities(cm.activeNetwork))
                }
            }
            try {
                cm.registerDefaultNetworkCallback(callback)
            } catch (e: SecurityException) {
                // Missing ACCESS_NETWORK_STATE would otherwise take down the
                // whole viewer for a quality hint.
                android.util.Log.w("VideoQuality", "cannot watch network state: ${e.message}")
            }
            onDispose {
                try {
                    cm.unregisterNetworkCallback(callback)
                } catch (e: IllegalArgumentException) {
                    // Already unregistered — not worth crashing the viewer over.
                    android.util.Log.w("VideoQuality", "network callback already gone: ${e.message}")
                }
            }
        }
    }
    return metered
}

/**
 * Whether the player should default to a reduced quality right now.
 *
 * Recomputes when either input changes, so toggling the setting or walking off
 * wifi is reflected without reopening the viewer.
 */
@Composable
fun rememberQualityConstrained(): Boolean {
    val context = LocalContext.current
    val dataStore = remember(context) {
        EntryPointAccessors.fromApplication(
            context.applicationContext,
            VideoQualityPrefEntryPoint::class.java
        ).dataStore()
    }
    val dataSaver by remember(dataStore) {
        dataStore.data.map { it[KEY_CELLULAR_DATA_SAVER] ?: true }
    }.collectAsState(initial = true)

    return isConstrained(dataSaverEnabled = dataSaver, metered = rememberMetered())
}
