/**
 * Library feature screens — People, Pets, Memories, Trips.
 *
 * Each "list" screen renders a thumbnail grid of clusters / memories /
 * trips (matching the web behaviour in web/src/pages/Albums.tsx). Each
 * tile drills into a detail screen that displays the photos belonging
 * to that cluster / memory / trip.
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
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Place
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import coil.compose.AsyncImage
import com.simplephotos.ui.components.rememberThumbnailRequest
import com.simplephotos.data.repository.AiRepository
import com.simplephotos.data.repository.GeoRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.remote.dto.*
import dagger.hilt.android.lifecycle.HiltViewModel
import javax.inject.Inject
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// ── Generic grid scaffold ────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun <T> GridScaffold(
    title: String,
    onBack: () -> Unit,
    loading: Boolean,
    error: String?,
    items: List<T>,
    keyOf: (T) -> Any,
    label: (T) -> String,
    subtitle: (T) -> String,
    thumbUrl: (T) -> String?,
    onItemClick: (T) -> Unit,
    emptyHint: String,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
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
                items.isEmpty() -> Text(
                    emptyHint,
                    modifier = Modifier.align(Alignment.Center).padding(16.dp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                else -> LazyVerticalGrid(
                    columns = GridCells.Adaptive(minSize = 140.dp),
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(12.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(items, key = { keyOf(it) }) { item ->
                        ClusterTile(
                            label = label(item),
                            subtitle = subtitle(item),
                            thumbUrl = thumbUrl(item),
                            onClick = { onItemClick(item) },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ClusterTile(
    label: String,
    subtitle: String,
    thumbUrl: String?,
    onClick: () -> Unit,
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        shape = RoundedCornerShape(12.dp),
    ) {
        Column {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f)
                    .background(MaterialTheme.colorScheme.surfaceVariant),
                contentAlignment = Alignment.Center,
            ) {
                if (!thumbUrl.isNullOrEmpty()) {
                    AsyncImage(
                        model = rememberThumbnailRequest(data = thumbUrl),
                        contentDescription = label,
                        contentScale = ContentScale.Crop,
                        modifier = Modifier.fillMaxSize(),
                    )
                } else {
                    Icon(
                        Icons.Default.Person,
                        contentDescription = null,
                        modifier = Modifier.size(40.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Column(modifier = Modifier.padding(horizontal = 10.dp, vertical = 8.dp)) {
                Text(label, fontWeight = FontWeight.SemiBold, maxLines = 1)
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
        }
    }
}

// ── Per-cluster photo grid (drill-down) ──────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PhotoIdsGridScaffold(
    title: String,
    onBack: () -> Unit,
    loading: Boolean,
    error: String?,
    photoIds: List<String>,
    serverBaseUrl: String,
    onPhotoClick: (String) -> Unit,
    emptyHint: String,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
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
                photoIds.isEmpty() -> Text(
                    emptyHint,
                    modifier = Modifier.align(Alignment.Center).padding(16.dp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                else -> LazyVerticalGrid(
                    columns = GridCells.Adaptive(minSize = 110.dp),
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(2.dp),
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    items(photoIds, key = { it }) { id ->
                        val url = if (serverBaseUrl.isNotEmpty())
                            "$serverBaseUrl/api/photos/$id/thumb" else null
                        Box(
                            modifier = Modifier
                                .fillMaxWidth()
                                .aspectRatio(1f)
                                .clip(RoundedCornerShape(2.dp))
                                .background(MaterialTheme.colorScheme.surfaceVariant)
                                .clickable { onPhotoClick(id) },
                        ) {
                            if (url != null) {
                                AsyncImage(
                                    model = rememberThumbnailRequest(data = url),
                                    contentDescription = null,
                                    contentScale = ContentScale.Crop,
                                    modifier = Modifier.fillMaxSize(),
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── People list + detail ─────────────────────────────────────────────────────

@HiltViewModel
class PeopleViewModel @Inject constructor(
    private val repo: AiRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var clusters by mutableStateOf<List<FaceCluster>>(emptyList()); private set
    var serverBaseUrl by mutableStateOf(""); private set

    init { reload() }

    fun reload() {
        viewModelScope.launch {
            loading = true; error = null
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                clusters = repo.listFaceClusters()
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun PeopleScreen(
    onBack: () -> Unit,
    onPersonClick: (Long) -> Unit,
    vm: PeopleViewModel = hiltViewModel(),
) {
    GridScaffold(
        title = "People",
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        items = vm.clusters,
        keyOf = { it.id },
        label = { it.label ?: "Unnamed" },
        subtitle = { "${it.photoCount} photos" },
        thumbUrl = { c ->
            c.representative?.let { id ->
                if (vm.serverBaseUrl.isNotEmpty()) "${vm.serverBaseUrl}/api/photos/$id/thumb" else null
            }
        },
        onItemClick = { cluster -> onPersonClick(cluster.id) },
        emptyHint = "No face clusters yet. Enable AI in Settings to begin scanning.",
    )
}

@HiltViewModel
class PersonDetailViewModel @Inject constructor(
    private val repo: AiRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var photoIds by mutableStateOf<List<String>>(emptyList()); private set
    var serverBaseUrl by mutableStateOf(""); private set
    var label by mutableStateOf("Person"); private set

    fun load(clusterId: Long) {
        viewModelScope.launch {
            loading = true; error = null
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                val all = repo.listFaceClusters()
                label = all.firstOrNull { it.id == clusterId }?.label ?: "Person"
                photoIds = repo.listFaceClusterPhotos(clusterId.toString()).map { it.photoId }
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun PersonDetailScreen(
    clusterId: Long,
    onBack: () -> Unit,
    onPhotoClick: (String) -> Unit,
    vm: PersonDetailViewModel = hiltViewModel(),
) {
    LaunchedEffect(clusterId) { vm.load(clusterId) }
    PhotoIdsGridScaffold(
        title = vm.label,
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        photoIds = vm.photoIds,
        serverBaseUrl = vm.serverBaseUrl,
        onPhotoClick = onPhotoClick,
        emptyHint = "No photos for this person.",
    )
}

// ── Pets list + detail ───────────────────────────────────────────────────────

@HiltViewModel
class PetsViewModel @Inject constructor(
    private val repo: AiRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var clusters by mutableStateOf<List<PetCluster>>(emptyList()); private set
    var serverBaseUrl by mutableStateOf(""); private set

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                clusters = repo.listPetClusters()
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun PetsScreen(
    onBack: () -> Unit,
    onPetClick: (Long) -> Unit,
    vm: PetsViewModel = hiltViewModel(),
) {
    GridScaffold(
        title = "Pets",
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        items = vm.clusters,
        keyOf = { it.id },
        label = { it.label ?: it.species },
        subtitle = { "${it.photoCount} photos" },
        thumbUrl = { c ->
            c.representative?.let { id ->
                if (vm.serverBaseUrl.isNotEmpty()) "${vm.serverBaseUrl}/api/photos/$id/thumb" else null
            }
        },
        onItemClick = { cluster -> onPetClick(cluster.id) },
        emptyHint = "No pet clusters yet.",
    )
}

@HiltViewModel
class PetDetailViewModel @Inject constructor(
    private val repo: AiRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var photoIds by mutableStateOf<List<String>>(emptyList()); private set
    var serverBaseUrl by mutableStateOf(""); private set
    var label by mutableStateOf("Pet"); private set

    fun load(clusterId: Long) {
        viewModelScope.launch {
            loading = true; error = null
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                val all = repo.listPetClusters()
                val match = all.firstOrNull { it.id == clusterId }
                label = match?.label ?: match?.species ?: "Pet"
                photoIds = repo.listPetClusterPhotos(clusterId.toString()).map { it.photoId }
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun PetDetailScreen(
    clusterId: Long,
    onBack: () -> Unit,
    onPhotoClick: (String) -> Unit,
    vm: PetDetailViewModel = hiltViewModel(),
) {
    LaunchedEffect(clusterId) { vm.load(clusterId) }
    PhotoIdsGridScaffold(
        title = vm.label,
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        photoIds = vm.photoIds,
        serverBaseUrl = vm.serverBaseUrl,
        onPhotoClick = onPhotoClick,
        emptyHint = "No photos for this pet.",
    )
}

// ── Memories list + detail ───────────────────────────────────────────────────

@HiltViewModel
class MemoriesViewModel @Inject constructor(
    private val repo: GeoRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var memories by mutableStateOf<List<GeoMemory>>(emptyList()); private set
    var serverBaseUrl by mutableStateOf(""); private set

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                memories = repo.listMemories()
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun MemoriesScreen(
    onBack: () -> Unit,
    onMemoryClick: (String) -> Unit,
    vm: MemoriesViewModel = hiltViewModel(),
) {
    GridScaffold(
        title = "Memories",
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        items = vm.memories,
        keyOf = { it.id },
        label = { it.name },
        subtitle = { "${it.photoCount} photos · ${it.dateLabel}" },
        thumbUrl = { m ->
            m.firstPhotoId?.let { id ->
                if (vm.serverBaseUrl.isNotEmpty()) "${vm.serverBaseUrl}/api/photos/$id/thumb" else null
            }
        },
        onItemClick = { mem -> onMemoryClick(mem.id) },
        emptyHint = "No memories curated yet.",
    )
}

@HiltViewModel
class MemoryDetailViewModel @Inject constructor(
    private val repo: GeoRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var photoIds by mutableStateOf<List<String>>(emptyList()); private set
    var serverBaseUrl by mutableStateOf(""); private set
    var title by mutableStateOf("Memory"); private set

    fun load(memoryId: String) {
        viewModelScope.launch {
            loading = true; error = null
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                val all = repo.listMemories()
                title = all.firstOrNull { it.id == memoryId }?.name ?: "Memory"
                photoIds = repo.listMemoryPhotos(memoryId).map { it.id }
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun MemoryDetailScreen(
    memoryId: String,
    onBack: () -> Unit,
    onPhotoClick: (String) -> Unit,
    vm: MemoryDetailViewModel = hiltViewModel(),
) {
    LaunchedEffect(memoryId) { vm.load(memoryId) }
    PhotoIdsGridScaffold(
        title = vm.title,
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        photoIds = vm.photoIds,
        serverBaseUrl = vm.serverBaseUrl,
        onPhotoClick = onPhotoClick,
        emptyHint = "No photos for this memory.",
    )
}

// ── Trips list + detail ──────────────────────────────────────────────────────

@HiltViewModel
class TripsViewModel @Inject constructor(
    private val repo: GeoRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var trips by mutableStateOf<List<GeoTrip>>(emptyList()); private set
    var serverBaseUrl by mutableStateOf(""); private set

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                trips = repo.listTrips()
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun TripsScreen(
    onBack: () -> Unit,
    onTripClick: (String) -> Unit,
    vm: TripsViewModel = hiltViewModel(),
) {
    GridScaffold(
        title = "Trips",
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        items = vm.trips,
        keyOf = { it.id },
        label = { it.name },
        subtitle = {
            val place = listOfNotNull(it.city.takeIf { c -> c.isNotEmpty() }, it.country.takeIf { c -> c.isNotEmpty() })
                .joinToString(", ")
            "${it.photoCount} photos · $place"
        },
        thumbUrl = { t ->
            t.firstPhotoId?.let { id ->
                if (vm.serverBaseUrl.isNotEmpty()) "${vm.serverBaseUrl}/api/photos/$id/thumb" else null
            }
        },
        onItemClick = { trip -> onTripClick(trip.id) },
        emptyHint = "No trips detected yet.",
    )
}

@HiltViewModel
class TripDetailViewModel @Inject constructor(
    private val repo: GeoRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var photoIds by mutableStateOf<List<String>>(emptyList()); private set
    var serverBaseUrl by mutableStateOf(""); private set
    var title by mutableStateOf("Trip"); private set

    fun load(tripId: String) {
        viewModelScope.launch {
            loading = true; error = null
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                val all = repo.listTrips()
                title = all.firstOrNull { it.id == tripId }?.name ?: "Trip"
                photoIds = repo.listTripPhotos(tripId).map { it.id }
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun TripDetailScreen(
    tripId: String,
    onBack: () -> Unit,
    onPhotoClick: (String) -> Unit,
    vm: TripDetailViewModel = hiltViewModel(),
) {
    LaunchedEffect(tripId) { vm.load(tripId) }
    PhotoIdsGridScaffold(
        title = vm.title,
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        photoIds = vm.photoIds,
        serverBaseUrl = vm.serverBaseUrl,
        onPhotoClick = onPhotoClick,
        emptyHint = "No photos for this trip.",
    )
}
