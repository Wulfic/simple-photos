/**
 * Library feature screens — People, Pets, Things, Timeline, Memories,
 * Trips, Places. Each is a simple list view backed by a Hilt ViewModel
 * that talks to AiRepository / GeoRepository.
 *
 * Drill-down to per-photo viewer is intentionally deferred — tapping an
 * item shows a count card. Future work can wire the Photo viewer once
 * server-id ↔ local-id resolution is implemented for non-synced photos.
 */
package com.simplephotos.ui.screens.library

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.repository.AiRepository
import com.simplephotos.data.repository.GeoRepository
import com.simplephotos.data.remote.dto.*
import dagger.hilt.android.lifecycle.HiltViewModel
import javax.inject.Inject
import kotlinx.coroutines.launch

// ── Generic "list of titled rows" scaffold ───────────────────────────────────

private data class Row(val title: String, val subtitle: String)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ListScaffold(
    title: String,
    onBack: () -> Unit,
    loading: Boolean,
    error: String?,
    rows: List<Row>,
    emptyHint: String,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        Box(modifier = Modifier.fillMaxSize().padding(padding)) {
            when {
                loading -> CircularProgressIndicator(
                    modifier = Modifier.align(Alignment.Center)
                )
                error != null -> Text(
                    "Error: $error",
                    modifier = Modifier.align(Alignment.Center).padding(16.dp),
                    color = MaterialTheme.colorScheme.error,
                )
                rows.isEmpty() -> Text(
                    emptyHint,
                    modifier = Modifier.align(Alignment.Center).padding(16.dp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                else -> LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(rows) { row ->
                        Card(modifier = Modifier.fillMaxWidth()) {
                            Column(modifier = Modifier.padding(16.dp)) {
                                Text(row.title, fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                                Text(
                                    row.subtitle,
                                    fontSize = 13.sp,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── People ───────────────────────────────────────────────────────────────────

// ── People ───────────────────────────────────────────────────────────────────

@HiltViewModel
class PeopleViewModel @Inject constructor(private val repo: AiRepository) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var clusters by mutableStateOf<List<FaceCluster>>(emptyList()); private set

    init { reload() }

    fun reload() {
        viewModelScope.launch {
            loading = true; error = null
            try { clusters = repo.listFaceClusters() }
            catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun PeopleScreen(onBack: () -> Unit, vm: PeopleViewModel = hiltViewModel()) {
    ListScaffold(
        title = "People",
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        rows = vm.clusters.map {
            Row(it.label ?: "Unnamed person", "${it.photoCount} photos")
        },
        emptyHint = "No face clusters yet. Enable AI in Settings to begin scanning.",
    )
}

// ── Pets ─────────────────────────────────────────────────────────────────────

@HiltViewModel
class PetsViewModel @Inject constructor(private val repo: AiRepository) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var clusters by mutableStateOf<List<PetCluster>>(emptyList()); private set

    init {
        viewModelScope.launch {
            try { clusters = repo.listPetClusters() }
            catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun PetsScreen(onBack: () -> Unit, vm: PetsViewModel = hiltViewModel()) {
    ListScaffold(
        title = "Pets",
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        rows = vm.clusters.map {
            Row(it.label ?: it.species, "${it.photoCount} photos")
        },
        emptyHint = "No pet clusters yet.",
    )
}

// ── Memories ─────────────────────────────────────────────────────────────────

@HiltViewModel
class MemoriesViewModel @Inject constructor(private val repo: GeoRepository) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var memories by mutableStateOf<List<GeoMemory>>(emptyList()); private set

    init {
        viewModelScope.launch {
            try { memories = repo.listMemories() }
            catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun MemoriesScreen(onBack: () -> Unit, vm: MemoriesViewModel = hiltViewModel()) {
    ListScaffold(
        title = "Memories",
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        rows = vm.memories.map {
            Row(it.name, "${it.photoCount} photos · ${it.dateLabel}")
        },
        emptyHint = "No memories curated yet.",
    )
}

// ── Trips ────────────────────────────────────────────────────────────────────

@HiltViewModel
class TripsViewModel @Inject constructor(private val repo: GeoRepository) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var trips by mutableStateOf<List<GeoTrip>>(emptyList()); private set

    init {
        viewModelScope.launch {
            try { trips = repo.listTrips() }
            catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun TripsScreen(onBack: () -> Unit, vm: TripsViewModel = hiltViewModel()) {
    ListScaffold(
        title = "Trips",
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        rows = vm.trips.map {
            val place = listOfNotNull(it.city, it.country).joinToString(", ")
            Row(it.name, "${it.photoCount} photos · $place · ${it.dateLabel}")
        },
        emptyHint = "No trips detected yet.",
    )
}
