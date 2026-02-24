package com.simplephotos.data.remote

import android.content.Context
import android.net.wifi.WifiManager
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.Inet4Address
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.Socket
import java.net.URL

private const val TAG = "ServerDiscovery"

/**
 * Discovered Simple Photos server on the local network.
 */
data class DiscoveredServer(
    val url: String,
    val version: String,
    val host: String,
    val port: Int
)

/**
 * Scans the local network for Simple Photos servers by probing the /health
 * endpoint on common ports across all IPs in the device's subnet.
 */
object ServerDiscovery {

    private val COMMON_PORTS = listOf(8080, 3000, 443, 8443, 80, 8000, 9000)
    private const val CONNECT_TIMEOUT_MS = 600
    private const val READ_TIMEOUT_MS = 600

    // Limit concurrent connections to avoid exhausting the connection pool on mobile
    private const val MAX_CONCURRENT_PROBES = 50

    /**
     * Discover servers on the local network.
     *
     * Strategy:
     * 1. Determine the device's local IP address and subnet (via NetworkInterface + WiFi fallback).
     * 2. First do a fast TCP connect scan to find hosts with open ports.
     * 3. Then probe only reachable host:port combos for `/health`.
     * 4. Return any that respond with `{"service":"simple-photos",...}`.
     */
    suspend fun discover(context: Context? = null): List<DiscoveredServer> = withContext(Dispatchers.IO) {
        val localAddresses = getLocalAddresses(context)
        Log.d(TAG, "Local addresses found: $localAddresses")
        if (localAddresses.isEmpty()) return@withContext emptyList()

        val semaphore = Semaphore(MAX_CONCURRENT_PROBES)
        val results = mutableListOf<DiscoveredServer>()

        coroutineScope {
            val jobs = localAddresses.flatMap { localIp ->
                val prefix = localIp.substringBeforeLast(".")
                Log.d(TAG, "Scanning subnet $prefix.0/24")
                // Scan .1 through .254
                (1..254).flatMap { i ->
                    val ip = "$prefix.$i"
                    COMMON_PORTS.map { port ->
                        async {
                            semaphore.withPermit {
                                probeServer(ip, port)
                            }
                        }
                    }
                }
            }

            results.addAll(jobs.awaitAll().filterNotNull())
        }

        Log.d(TAG, "Discovery complete. Found ${results.size} servers")
        // Deduplicate by url
        results.distinctBy { it.url }
    }

    /**
     * Check if a port is open using a quick TCP connect, then probe the health endpoint.
     */
    private fun probeServer(host: String, port: Int): DiscoveredServer? {
        return try {
            // Quick TCP connect check first — much faster than a full HTTP request
            val socket = Socket()
            try {
                socket.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
                socket.close()
            } catch (_: Exception) {
                return null // Port not open, skip HTTP probe
            }

            // Port is open — now try HTTP health check
            val protocols = if (port == 443 || port == 8443) listOf("https") else listOf("http")
            for (protocol in protocols) {
                val url = "$protocol://$host:$port"
                try {
                    val conn = URL("$url/health").openConnection() as HttpURLConnection
                    conn.connectTimeout = CONNECT_TIMEOUT_MS
                    conn.readTimeout = READ_TIMEOUT_MS
                    conn.requestMethod = "GET"
                    conn.setRequestProperty("X-Requested-With", "SimplePhotos")

                    if (conn.responseCode == 200) {
                        val body = conn.inputStream.bufferedReader().readText()
                        conn.disconnect()
                        val json = JSONObject(body)
                        if (json.optString("service") == "simple-photos") {
                            Log.d(TAG, "Found server at $url (v${json.optString("version", "?")})")
                            return DiscoveredServer(
                                url = url,
                                version = json.optString("version", "unknown"),
                                host = host,
                                port = port
                            )
                        }
                    } else {
                        conn.disconnect()
                    }
                } catch (_: Exception) {
                    // This particular protocol/host/port didn't work
                }
            }
            null
        } catch (_: Exception) {
            null
        }
    }

    /**
     * Get the device's local IPv4 addresses (non-loopback).
     * 
     * Uses two strategies:
     * 1. NetworkInterface enumeration (works in most cases)
     * 2. WifiManager fallback (works when NetworkInterface is restricted on some devices)
     */
    private fun getLocalAddresses(context: Context? = null): List<String> {
        val addresses = mutableSetOf<String>()

        // Strategy 1: NetworkInterface (standard Java API)
        try {
            val interfaces = NetworkInterface.getNetworkInterfaces()
            while (interfaces.hasMoreElements()) {
                val iface = interfaces.nextElement()
                if (iface.isLoopback || !iface.isUp) continue
                // Only consider WiFi/Ethernet-like interfaces
                val name = iface.name?.lowercase() ?: continue
                if (name.startsWith("wlan") || name.startsWith("eth") || name.startsWith("en")) {
                    val addrs = iface.inetAddresses
                    while (addrs.hasMoreElements()) {
                        val addr = addrs.nextElement()
                        if (addr is Inet4Address && !addr.isLoopbackAddress) {
                            addr.hostAddress?.let { addresses.add(it) }
                        }
                    }
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "NetworkInterface enumeration failed: ${e.message}")
        }

        // Strategy 2: WifiManager fallback (Android-specific)
        if (addresses.isEmpty() && context != null) {
            try {
                @Suppress("DEPRECATION")
                val wifiManager = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
                val connInfo = wifiManager?.connectionInfo
                val ipInt = connInfo?.ipAddress ?: 0
                if (ipInt != 0) {
                    val ip = String.format(
                        "%d.%d.%d.%d",
                        ipInt and 0xff,
                        ipInt shr 8 and 0xff,
                        ipInt shr 16 and 0xff,
                        ipInt shr 24 and 0xff
                    )
                    if (ip != "0.0.0.0") {
                        Log.d(TAG, "WiFi Manager fallback IP: $ip")
                        addresses.add(ip)
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "WifiManager fallback failed: ${e.message}")
            }
        }

        return addresses.toList()
    }
}
