/**
 * Library hub screen — entry point to People, Pets, Things, Map, Timeline,
 * Memories, Trips and Export. Mirrors the web UI's "Library" navigation.
 */
package com.simplephotos.ui.screens.library

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private data class LibraryEntry(
    val title: String,
    val subtitle: String,
    val color: Color,
    val onClick: () -> Unit,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LibraryScreen(
    onBack: () -> Unit,
    onPeople: () -> Unit,
    onPets: () -> Unit,
    onThings: () -> Unit,
    onMap: () -> Unit,
    onTimeline: () -> Unit,
    onMemories: () -> Unit,
    onTrips: () -> Unit,
    onLocations: () -> Unit,
    onExport: () -> Unit,
) {
    val entries = listOf(
        LibraryEntry("People", "Face clusters", Color(0xFF6366F1), onPeople),
        LibraryEntry("Pets", "Pet clusters", Color(0xFFF59E0B), onPets),
        LibraryEntry("Things", "Object detection", Color(0xFF10B981), onThings),
        LibraryEntry("Map", "Geo-tagged photos", Color(0xFF3B82F6), onMap),
        LibraryEntry("Timeline", "By year and month", Color(0xFF8B5CF6), onTimeline),
        LibraryEntry("Memories", "Auto-curated highlights", Color(0xFFEC4899), onMemories),
        LibraryEntry("Trips", "Auto-detected trips", Color(0xFF14B8A6), onTrips),
        LibraryEntry("Places", "Countries & cities", Color(0xFF0EA5E9), onLocations),
        LibraryEntry("Export", "Download archive", Color(0xFF64748B), onExport),
    )

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Library") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        LazyVerticalGrid(
            columns = GridCells.Fixed(2),
            modifier = Modifier.fillMaxSize().padding(padding).padding(12.dp),
            contentPadding = PaddingValues(4.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            items(entries) { e -> LibraryCard(e) }
        }
    }
}

@Composable
private fun LibraryCard(entry: LibraryEntry) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .height(120.dp)
            .clip(RoundedCornerShape(16.dp))
            .clickable { entry.onClick() },
        colors = CardDefaults.cardColors(containerColor = entry.color.copy(alpha = 0.12f)),
    ) {
        Column(
            modifier = Modifier.fillMaxSize().padding(16.dp),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Box(
                modifier = Modifier.size(36.dp).clip(RoundedCornerShape(10.dp))
                    .background(entry.color)
            )
            Column {
                Text(entry.title, fontWeight = FontWeight.Bold, fontSize = 16.sp)
                Text(entry.subtitle, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
