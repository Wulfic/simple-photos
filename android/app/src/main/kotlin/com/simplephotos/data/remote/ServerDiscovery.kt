/**
 * Local network scanning to auto-discover Simple Photos server instances
 * by probing the dedicated discovery port (3301) on each IP in the subnet.
 *
 * Each server runs a lightweight HTTP listener on port 3301 that responds
 * with the server's name, version, and actual HTTP port. This reduces
 * network probes from ~3,300 (254 IPs × 13 ports) to just 254.
 */
package com.simplephotos.data.remote

import android.content.Context
import android.net.ConnectivityManager
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
    val port: Int,
    val mode: String = "primary",
    val setupComplete: Boolean = false
)

/**
 * Scans the local network for Simple Photos servers.
 *
 * Primary strategy: probe the dedicated discovery port (3301) on each
 * subnet IP. The discovery response includes the server's actual HTTP
 * port, so we only need 1 probe per IP instead of scanning many ports.
 *
 * Fallback: if no servers are found via the discovery port, falls back
 * to probing common ports with `/health` (for older server versions).
 */
object ServerDiscovery {

    /** Dedicated discovery port — must match the server's `discovery_port` config. */
    private const val DISCOVERY_PORT = 3301

    /** Legacy ports to try if discovery port finds nothing (backward compat). */
    private val FALLBACK_PORTS = listOf(8080, 8081, 8082, 8083, 3000, 3001, 3002, 3003, 443, 8443, 80, 8000, 9000)

    private const val CONNECT_TIMEOUT_MS = 600
    private const val READ_TIMEOUT_MS = 600
    private const val MAX_CONCURRENT_PROBES = 100

    /**
     * Discover servers on the local network.
     *
     * Strategy:
     * 1. Determine the device's local IP address and subnet.
     * 2. Probe port 3301 on all 254 subnet IPs (fast — 1 probe per IP).
     * 3. Each response includes the server's real HTTP port → build URL.
     * 4. If nothing found, fall back to legacy multi-port scan.
     */
    suspend fun discover(context: Context? = null): List<DiscoveredServer> = withContext(Dispatchers.IO) {
        val localAddresses = getLocalAddresses(context)
        Log.d(TAG, "Local addresses found: $localAddresses")
        if (localAddresses.isEmpty()) return@withContext emptyList()

        val semaphore = Semaphore(MAX_CONCURRENT_PROBES)

        // ── Phase 1: Discovery port scan (254 probes per subnet) ─────────
        val discoveryResults = mutableListOf<DiscoveredServer>()
        coroutineScope {
            val jobs = localAddresses.flatMap { localIp ->
                val prefix = localIp.substringBeforeLast(".")
                Log.d(TAG, "Scanning subnet $prefix.0/24 on discovery port $DISCOVERY_PORT")
                (1..254).map { i ->
                    val ip = "$prefix.$i"
                    async {
                        semaphore.withPermit {
                            probeDiscoveryPort(ip)
                        }
                    }
                }
            }
            discoveryResults.addAll(jobs.awaitAll().filterNotNull())
        }

        if (discoveryResults.isNotEmpty()) {
            Log.d(TAG, "Discovery port found ${discoveryResults.size} servers")
            // Enrich each server with setup status
            val enriched = coroutineScope {
                discoveryResults.distinctBy { it.url }.map { server ->
                    async { checkSetupStatus(server) }
                }.awaitAll()
            }
            return@withContext enriched
        }

        // ── Phase 2: Fallback — legacy multi-port scan ───────────────────
        Log.d(TAG, "No servers found via discovery port, falling back to multi-port scan")
        val fallbackResults = mutableListOf<DiscoveredServer>()
        coroutineScope {
            val jobs = localAddresses.flatMap { localIp ->
                val prefix = localIp.substringBeforeLast(".")
                (1..254).flatMap { i ->
                    val ip = "$prefix.$i"
                    FALLBACK_PORTS.map { port ->
                        async {
                            semaphore.withPermit {
                                probeServerHealth(ip, port)
                            }
                        }
                    }
                }
            }
            fallbackResults.addAll(jobs.awaitAll().filterNotNull())
        }

        Log.d(TAG, "Discovery complete. Found ${fallbackResults.size} servers (fallback)")
        // Enrich each server with setup status
        val enriched = coroutineScope {
            fallbackResults.distinctBy { it.url }.map { server ->
                async { checkSetupStatus(server) }
            }.awaitAll()
        }
        enriched
    }

    /**
     * Check /api/setup/status on a discovered server to determine if it's
     * configured. Returns a copy with `setupComplete` (and a possibly
     * scheme-corrected `url`) populated.
     *
     * The discovery beacon advertises whether the API speaks TLS, but older
     * servers — or one reconfigured after the beacon guessed — may report it
     * wrong. So if the advertised scheme fails we retry the opposite scheme
     * before giving up. A reachable server is never dropped without a trace:
     * every failed probe is logged. (A silently-swallowed http-vs-https
     * mismatch here is exactly what made discovery report "no servers".)
     */
    private fun checkSetupStatus(server: DiscoveredServer): DiscoveredServer {
        val candidates = schemeCandidates(server.url)
        for (url in candidates) {
            try {
                val json = fetchJson("$url/api/setup/status") ?: continue
                return server.copy(
                    url = url,
                    setupComplete = json.optBoolean("setup_complete", false),
                    mode = json.optString("mode", server.mode)
                )
            } catch (e: Exception) {
                Log.w(TAG, "setup/status probe failed for $url: ${e.message}")
            }
        }
        Log.w(TAG, "setup/status unreachable for ${server.host} via ${candidates.joinToString()} — left unenriched")
        return server
    }

    /**
     * Candidate base URLs to try, advertised scheme first then the opposite,
     * so a beacon that mis-reports (or omits) its TLS flag still resolves.
     */
    private fun schemeCandidates(url: String): List<String> = when {
        url.startsWith("https://") -> listOf(url, "http://" + url.removePrefix("https://"))
        url.startsWith("http://") -> listOf(url, "https://" + url.removePrefix("http://"))
        else -> listOf(url)
    }

    /**
     * GET [fullUrl] and parse the JSON body, or return null on a non-200
     * response. Transport failures (connection refused, a TLS handshake against
     * a plaintext port, timeouts) propagate so the caller can try another scheme.
     */
    private fun fetchJson(fullUrl: String): JSONObject? {
        val conn = URL(fullUrl).openConnection() as HttpURLConnection
        conn.connectTimeout = CONNECT_TIMEOUT_MS
        conn.readTimeout = READ_TIMEOUT_MS
        conn.requestMethod = "GET"
        return try {
            if (conn.responseCode == 200) {
                JSONObject(conn.inputStream.bufferedReader().readText())
            } else {
                null
            }
        } finally {
            conn.disconnect()
        }
    }

    /**
     * Probe the dedicated discovery port on a host.
     * Returns a DiscoveredServer with the actual HTTP port from the response.
     */
    private fun probeDiscoveryPort(host: String): DiscoveredServer? {
        return try {
            // Quick TCP connect check (no data sent/received over the raw socket;
            // the socket is closed immediately and the actual exchange uses
            // HttpURLConnection, which honours the network security config).
            // nosemgrep: kotlin.lang.security.unencrypted-socket.unencrypted-socket
            val socket = Socket()
            try {
                socket.connect(InetSocketAddress(host, DISCOVERY_PORT), CONNECT_TIMEOUT_MS)
                socket.close()
            } catch (_: Exception) {
                return null
            }

            val url = "http://$host:$DISCOVERY_PORT/"
            val conn = URL(url).openConnection() as HttpURLConnection
            conn.connectTimeout = CONNECT_TIMEOUT_MS
            conn.readTimeout = READ_TIMEOUT_MS
            conn.requestMethod = "GET"

            if (conn.responseCode == 200) {
                val body = conn.inputStream.bufferedReader().readText()
                conn.disconnect()
                val json = JSONObject(body)
                if (json.optString("service") == "simple-photos") {
                    val actualPort = json.optInt("port", DISCOVERY_PORT)
                    val version = json.optString("version", "unknown")
                    val mode = json.optString("mode", "primary")
                    // The beacon tells us whether the API speaks TLS so we build
                    // the right scheme. Older servers omit `tls` → default to
                    // http (the historic behaviour); checkSetupStatus() retries
                    // the opposite scheme if this guess turns out wrong.
                    val scheme = if (json.optBoolean("tls", false)) "https" else "http"
                    val serverUrl = "$scheme://$host:$actualPort"
                    Log.d(TAG, "Found server at $serverUrl via discovery port (v$version, mode=$mode)")
                    return DiscoveredServer(
                        url = serverUrl,
                        version = version,
                        host = host,
                        port = actualPort,
                        mode = mode
                    )
                }
            } else {
                conn.disconnect()
            }
            null
        } catch (_: Exception) {
            null
        }
    }

    /**
     * Legacy: probe a server via /health endpoint (for servers without discovery port).
     */
    private fun probeServerHealth(host: String, port: Int): DiscoveredServer? {
        return try {
            // Connect-only reachability probe; no payload traverses this socket.
            // nosemgrep: kotlin.lang.security.unencrypted-socket.unencrypted-socket
            val socket = Socket()
            try {
                socket.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
                socket.close()
            } catch (_: Exception) {
                return null
            }

            val protocols = if (port == 443 || port == 8443) listOf("https") else listOf("http")
            for (protocol in protocols) {
                val serverUrl = "$protocol://$host:$port"
                try {
                    val conn = URL("$serverUrl/health").openConnection() as HttpURLConnection
                    conn.connectTimeout = CONNECT_TIMEOUT_MS
                    conn.readTimeout = READ_TIMEOUT_MS
                    conn.requestMethod = "GET"
                    conn.setRequestProperty("X-Requested-With", "SimplePhotos")

                    if (conn.responseCode == 200) {
                        val body = conn.inputStream.bufferedReader().readText()
                        conn.disconnect()
                        val json = JSONObject(body)
                        if (json.optString("service") == "simple-photos") {
                            val mode = json.optString("mode", "primary")
                            Log.d(TAG, "Found server at $serverUrl (v${json.optString("version", "?")}, mode=$mode)")
                            return DiscoveredServer(
                                url = serverUrl,
                                version = json.optString("version", "unknown"),
                                host = host,
                                port = port,
                                mode = mode
                            )
                        }
                    } else {
                        conn.disconnect()
                    }
                } catch (_: Exception) {
                    // This protocol/host/port didn't work
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
     * 2. WifiManager fallback (works when NetworkInterface is restricted)
     */
    private fun getLocalAddresses(context: Context? = null): List<String> {
        val addresses = mutableSetOf<String>()

        // Strategy 1: NetworkInterface (standard Java API)
        try {
            val interfaces = NetworkInterface.getNetworkInterfaces()
            while (interfaces.hasMoreElements()) {
                val iface = interfaces.nextElement()
                if (iface.isLoopback || !iface.isUp) continue
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

        // Strategy 2: ConnectivityManager fallback (Android-specific). Replaces
        // the deprecated WifiManager.connectionInfo/ipAddress path — reads the
        // active network's IPv4 link address instead.
        if (addresses.isEmpty() && context != null) {
            try {
                val cm = context.applicationContext
                    .getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                val activeNetwork = cm?.activeNetwork
                val linkProps = activeNetwork?.let { cm.getLinkProperties(it) }
                linkProps?.linkAddresses?.forEach { linkAddr ->
                    val addr = linkAddr.address
                    if (addr is Inet4Address && !addr.isLoopbackAddress) {
                        addr.hostAddress?.let {
                            Log.d(TAG, "ConnectivityManager fallback IP: $it")
                            addresses.add(it)
                        }
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "ConnectivityManager fallback failed: ${e.message}")
            }
        }

        return addresses.toList()
    }
}
