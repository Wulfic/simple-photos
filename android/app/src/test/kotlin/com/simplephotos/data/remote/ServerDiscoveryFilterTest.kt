package com.simplephotos.data.remote

import org.junit.Assert.*
import org.junit.Test

/**
 * Unit tests for server discovery primary-mode filtering.
 */
class ServerDiscoveryFilterTest {

    // Mirrors DiscoveredServer from ServerDiscovery.kt
    private data class DiscoveredServer(
        val ip: String,
        val port: Int,
        val name: String,
        val version: String,
        val mode: String = "primary"
    )

    @Test
    fun filterPrimaryOnly_excludesBackupServers() {
        val servers = listOf(
            DiscoveredServer("192.168.1.10", 8080, "Main", "0.6.9", "primary"),
            DiscoveredServer("192.168.1.20", 8080, "Backup1", "0.6.9", "backup"),
            DiscoveredServer("192.168.1.30", 8080, "Backup2", "0.6.9", "backup")
        )

        val primaryOnly = servers.filter { it.mode == "primary" }
        assertEquals(1, primaryOnly.size)
        assertEquals("192.168.1.10", primaryOnly[0].ip)
    }

    @Test
    fun filterPrimaryOnly_includessDefaultMode() {
        // Servers that don't report a mode default to "primary"
        val servers = listOf(
            DiscoveredServer("192.168.1.10", 8080, "Main", "0.6.9"),
            DiscoveredServer("192.168.1.20", 8080, "WithMode", "0.6.9", "primary")
        )

        val primaryOnly = servers.filter { it.mode == "primary" }
        assertEquals(2, primaryOnly.size)
    }

    @Test
    fun filterPrimaryOnly_emptyList() {
        val servers = emptyList<DiscoveredServer>()
        val primaryOnly = servers.filter { it.mode == "primary" }
        assertTrue(primaryOnly.isEmpty())
    }

    @Test
    fun filterPrimaryOnly_allBackup() {
        val servers = listOf(
            DiscoveredServer("192.168.1.20", 8080, "Backup1", "0.6.9", "backup"),
            DiscoveredServer("192.168.1.30", 8080, "Backup2", "0.6.9", "backup")
        )

        val primaryOnly = servers.filter { it.mode == "primary" }
        assertTrue(primaryOnly.isEmpty())
    }
}
