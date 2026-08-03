/**
 * Library feature screens — People, Pets, Memories, Trips.
 *
 * Each "list" screen renders a thumbnail grid of clusters / memories /
 * trips (matching the web behaviour in web/src/pages/Albums.tsx). Each
 * tile drills into a detail screen that displays the photos belonging
 * to that cluster / memory / trip.
 */
package com.simplephotos.ui.screens.library

import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Place
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import coil.compose.AsyncImage
import com.simplephotos.ui.components.rememberThumbnailRequest
import com.simplephotos.ui.navigation.DetailNavBar
import com.simplephotos.data.album.GridPhotoIds
import com.simplephotos.data.album.gridPhotoIds
import com.simplephotos.data.repository.AiRepository
import com.simplephotos.data.repository.GeoRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.remote.dto.*
import dagger.hilt.android.lifecycle.HiltViewModel
import javax.inject.Inject
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val TAG = "LibraryFeatures"

// Face-tile framing lives in FaceCrop.kt (`TileFaceBox`, `faceCropRect`) so the
// arithmetic is JVM-testable without a device — see FaceCropTest.

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
    banner: String? = null,
    faceBox: (T) -> TileFaceBox? = { null },
    /** Circular portrait tiles for person/pet clusters, matching web's
     *  `variant="avatar"` (#48c). Trips and Memories stay rectangular. */
    circular: Boolean = false,
) {
    Scaffold(
        topBar = {
            Column {
                // Persistent main navbar across the albums/library section (#35).
                DetailNavBar()
                TopAppBar(
                    title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    },
                    // AppHeader above already consumed the status-bar inset.
                    windowInsets = WindowInsets(0),
                )
            }
        }
    ) { padding ->
        Column(modifier = Modifier.fillMaxSize().padding(padding)) {
        if (banner != null) {
            Text(
                banner,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surfaceVariant)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
            )
        }
        Box(modifier = Modifier.fillMaxSize()) {
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
                            faceBox = faceBox(item),
                            circular = circular,
                            onClick = { onItemClick(item) },
                        )
                    }
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
    faceBox: TileFaceBox? = null,
    circular: Boolean = false,
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
                    // Clips the blown-up thumbnail below — the graphicsLayer
                    // deliberately draws outside its own bounds.
                    .clip(
                        if (circular) CircleShape
                        else RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp)
                    )
                    .background(MaterialTheme.colorScheme.surfaceVariant),
                contentAlignment = Alignment.Center,
            ) {
                if (!thumbUrl.isNullOrEmpty()) {
                    // Frame the detected face when the server sent a bbox —
                    // otherwise show the whole (cover-cropped) photo.
                    val rect = faceCropRect(faceBox)
                    val cropModifier = if (rect != null) {
                        // Reproduces the web CSS exactly: FillBounds makes the
                        // image the tile's size, then this scales it up about
                        // the top-left and slides the chosen window into view.
                        // TransformOrigin(0,0) is what makes the translation a
                        // plain offset — with a centred origin the scale would
                        // move the window too, which is the class of mistake
                        // that produced #48 in the first place.
                        Modifier.graphicsLayer {
                            transformOrigin = TransformOrigin(0f, 0f)
                            scaleX = 1f / rect.zx
                            scaleY = 1f / rect.zy
                            translationX = rect.px * (1f - 1f / rect.zx) * size.width
                            translationY = rect.py * (1f - 1f / rect.zy) * size.height
                        }
                    } else Modifier
                    AsyncImage(
                        model = rememberThumbnailRequest(data = thumbUrl),
                        contentDescription = label,
                        // FillBounds, not Crop: the bbox is normalised against
                        // the whole photo, so a centre-crop would apply it in a
                        // coordinate space it does not belong to. Aspect is
                        // preserved by zx/zy differing, not by the scaler.
                        contentScale = if (rect != null) ContentScale.FillBounds else ContentScale.Crop,
                        modifier = Modifier.fillMaxSize().then(cropModifier),
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
    grid: GridPhotoIds,
    serverBaseUrl: String,
    onPhotoClick: (photoId: String, photoIds: List<String>) -> Unit,
    emptyHint: String,
    actions: @Composable RowScope.() -> Unit = {},
) {
    // Tiles are drawn from SERVER ids (the thumbnail endpoint is keyed on them),
    // but the viewer pages LOCAL ids — see [GridPhotoIds]. Conflating the two is
    // what made every tap here resolve to no page (E3a).
    val photoIds = grid.serverIds
    Scaffold(
        topBar = {
            Column {
                // Persistent main navbar across the albums/library section (#35).
                DetailNavBar()
                TopAppBar(
                    title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    },
                    actions = actions,
                    // AppHeader above already consumed the status-bar inset.
                    windowInsets = WindowInsets(0),
                )
            }
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
                                .clickable { onPhotoClick(grid.viewerIdFor(id), grid.viewerIds) },
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

/**
 * Project a server-side photo id list onto the local mirror (#52, E3a).
 *
 * Shared by all four detail ViewModels below because they are identical in this
 * respect and were identically broken: each mapped a server endpoint's photo ids
 * straight into `Screen.PhotoViewer.createRoute`, where they were matched against
 * [com.simplephotos.data.local.entities.PhotoEntity.localId] — a random UUID that
 * a server id can never equal. See [GridPhotoIds] for the full account.
 */
private suspend fun PhotoRepository.gridFor(serverIds: List<String>): GridPhotoIds =
    gridPhotoIds(serverIds, withContext(Dispatchers.IO) { getPhotosByServerPhotoIds(serverIds) })

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

    /** Reassign a face detection to the chosen person, then invoke [onDone]. */
    fun assignFace(detectionId: Long, clusterId: Long, onDone: () -> Unit) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { repo.assignFace(detectionId, clusterId) }
            } catch (e: Exception) {
                error = e.message
            }
            onDone()
        }
    }
}

@Composable
fun PeopleScreen(
    onBack: () -> Unit,
    onPersonClick: (Long) -> Unit,
    assignDetectionId: Long? = null,
    onAssigned: () -> Unit = {},
    vm: PeopleViewModel = hiltViewModel(),
) {
    val assigning = assignDetectionId != null
    GridScaffold(
        title = if (assigning) "Assign to person" else "People",
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
        faceBox = { c -> tileFaceBoxOf(c.repBboxX, c.repBboxY, c.repBboxW, c.repBboxH) },
        onItemClick = { cluster ->
            if (assignDetectionId != null) {
                vm.assignFace(assignDetectionId, cluster.id, onAssigned)
            } else {
                onPersonClick(cluster.id)
            }
        },
        emptyHint = "No face clusters yet. Enable AI in Settings to begin scanning.",
        banner = if (assigning) "Choose the correct person for this face." else null,
        circular = true,
    )
}

@HiltViewModel
class PersonDetailViewModel @Inject constructor(
    private val repo: AiRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var grid by mutableStateOf(GridPhotoIds.EMPTY); private set
    var serverBaseUrl by mutableStateOf(""); private set
    var label by mutableStateOf("Person"); private set

    fun load(clusterId: Long) {
        viewModelScope.launch {
            loading = true; error = null
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                val all = repo.listFaceClusters()
                label = all.firstOrNull { it.id == clusterId }?.label ?: "Person"
                grid = photoRepo.gridFor(
                    repo.listFaceClusterPhotos(clusterId.toString()).map { it.photoId }
                )
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }

    /** Rename this person (face cluster). Optimistically updates the title; on
     *  failure logs, surfaces the error and leaves the old label. */
    fun rename(clusterId: Long, name: String, onDone: () -> Unit) {
        viewModelScope.launch {
            val outcome = performClusterRename(name) {
                withContext(Dispatchers.IO) { repo.renameFaceCluster(clusterId.toString(), it) }
            }
            when (outcome) {
                is RenameOutcome.Renamed -> { label = outcome.label; error = null }
                is RenameOutcome.Failed -> {
                    Log.e(TAG, "Rename of face cluster $clusterId failed: ${outcome.message}")
                    error = outcome.message
                }
                RenameOutcome.Skipped -> Unit
            }
            onDone()
        }
    }
}

@Composable
fun PersonDetailScreen(
    clusterId: Long,
    onBack: () -> Unit,
    onPhotoClick: (photoId: String, photoIds: List<String>) -> Unit,
    vm: PersonDetailViewModel = hiltViewModel(),
) {
    LaunchedEffect(clusterId) { vm.load(clusterId) }
    var showRename by remember { mutableStateOf(false) }
    PhotoIdsGridScaffold(
        title = vm.label,
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        grid = vm.grid,
        serverBaseUrl = vm.serverBaseUrl,
        onPhotoClick = onPhotoClick,
        emptyHint = "No photos for this person.",
        actions = {
            IconButton(onClick = { showRename = true }) {
                Icon(Icons.Default.Edit, contentDescription = "Rename person")
            }
        },
    )
    if (showRename) {
        RenameClusterDialog(
            current = vm.label,
            title = "Rename person",
            onDismiss = { showRename = false },
            onConfirm = { newName -> vm.rename(clusterId, newName) { showRename = false } },
        )
    }
}

/** Shared rename dialog for a person/pet cluster.
 *
 * [title] is a parameter because the dialog was already documented as being
 * "for a person/pet cluster" while hardcoding "Rename person" — reusing it for
 * pets (#39) without this would have shown a pet owner the word "person". */
@Composable
private fun RenameClusterDialog(
    current: String,
    title: String,
    onDismiss: () -> Unit,
    onConfirm: (String) -> Unit,
) {
    var value by remember { mutableStateOf(current) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = value,
                onValueChange = { value = it },
                singleLine = true,
                label = { Text("Name") },
            )
        },
        confirmButton = {
            TextButton(
                onClick = { onConfirm(value) },
                enabled = value.trim().isNotEmpty(),
            ) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
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
        // #48(d): the server now resolves a representative pet box, so the tile
        // frames the animal instead of centre-cropping it. Same call as People's
        // — `faceCropRect` is normalised-box arithmetic and knows nothing about
        // faces. Returns null for pets processed before migration 039, which
        // draws the plain crop this screen used to draw for everything.
        faceBox = { c -> tileFaceBoxOf(c.repBboxX, c.repBboxY, c.repBboxW, c.repBboxH) },
        onItemClick = { cluster -> onPetClick(cluster.id) },
        emptyHint = "No pet clusters yet.",
        // Circular to match web.
        circular = true,
    )
}

@HiltViewModel
class PetDetailViewModel @Inject constructor(
    private val repo: AiRepository,
    private val photoRepo: PhotoRepository,
) : ViewModel() {
    var loading by mutableStateOf(true); private set
    var error by mutableStateOf<String?>(null); private set
    var grid by mutableStateOf(GridPhotoIds.EMPTY); private set
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
                grid = photoRepo.gridFor(
                    repo.listPetClusterPhotos(clusterId.toString()).map { it.photoId }
                )
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }

    /** Rename this pet cluster (#39). The whole backend path already existed
     *  (`ApiService.renamePetCluster` → `AiRepository.renamePetCluster`); only
     *  this and the dialog wiring were missing. Mirrors
     *  [PersonDetailViewModel.rename] via the shared [performClusterRename]. */
    fun rename(clusterId: Long, name: String, onDone: () -> Unit) {
        viewModelScope.launch {
            val outcome = performClusterRename(name) {
                withContext(Dispatchers.IO) { repo.renamePetCluster(clusterId.toString(), it) }
            }
            when (outcome) {
                is RenameOutcome.Renamed -> { label = outcome.label; error = null }
                is RenameOutcome.Failed -> {
                    Log.e(TAG, "Rename of pet cluster $clusterId failed: ${outcome.message}")
                    error = outcome.message
                }
                RenameOutcome.Skipped -> Unit
            }
            onDone()
        }
    }
}

@Composable
fun PetDetailScreen(
    clusterId: Long,
    onBack: () -> Unit,
    onPhotoClick: (photoId: String, photoIds: List<String>) -> Unit,
    vm: PetDetailViewModel = hiltViewModel(),
) {
    LaunchedEffect(clusterId) { vm.load(clusterId) }
    var showRename by remember { mutableStateOf(false) }
    PhotoIdsGridScaffold(
        title = vm.label,
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        grid = vm.grid,
        serverBaseUrl = vm.serverBaseUrl,
        onPhotoClick = onPhotoClick,
        emptyHint = "No photos for this pet.",
        actions = {
            IconButton(onClick = { showRename = true }) {
                Icon(Icons.Default.Edit, contentDescription = "Rename pet")
            }
        },
    )
    if (showRename) {
        RenameClusterDialog(
            current = vm.label,
            title = "Rename pet",
            onDismiss = { showRename = false },
            onConfirm = { newName -> vm.rename(clusterId, newName) { showRename = false } },
        )
    }
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
    var grid by mutableStateOf(GridPhotoIds.EMPTY); private set
    var serverBaseUrl by mutableStateOf(""); private set
    var title by mutableStateOf("Memory"); private set

    fun load(memoryId: String) {
        viewModelScope.launch {
            loading = true; error = null
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                val all = repo.listMemories()
                title = all.firstOrNull { it.id == memoryId }?.name ?: "Memory"
                grid = photoRepo.gridFor(repo.listMemoryPhotos(memoryId).map { it.id })
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun MemoryDetailScreen(
    memoryId: String,
    onBack: () -> Unit,
    onPhotoClick: (photoId: String, photoIds: List<String>) -> Unit,
    vm: MemoryDetailViewModel = hiltViewModel(),
) {
    LaunchedEffect(memoryId) { vm.load(memoryId) }
    PhotoIdsGridScaffold(
        title = vm.title,
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        grid = vm.grid,
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
    var grid by mutableStateOf(GridPhotoIds.EMPTY); private set
    var serverBaseUrl by mutableStateOf(""); private set
    var title by mutableStateOf("Trip"); private set

    fun load(tripId: String) {
        viewModelScope.launch {
            loading = true; error = null
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepo.getServerBaseUrl() }
                val all = repo.listTrips()
                title = all.firstOrNull { it.id == tripId }?.name ?: "Trip"
                grid = photoRepo.gridFor(repo.listTripPhotos(tripId).map { it.id })
            } catch (e: Exception) { error = e.message }
            loading = false
        }
    }
}

@Composable
fun TripDetailScreen(
    tripId: String,
    onBack: () -> Unit,
    onPhotoClick: (photoId: String, photoIds: List<String>) -> Unit,
    vm: TripDetailViewModel = hiltViewModel(),
) {
    LaunchedEffect(tripId) { vm.load(tripId) }
    PhotoIdsGridScaffold(
        title = vm.title,
        onBack = onBack,
        loading = vm.loading,
        error = vm.error,
        grid = vm.grid,
        serverBaseUrl = vm.serverBaseUrl,
        onPhotoClick = onPhotoClick,
        emptyHint = "No photos for this trip.",
    )
}
