package com.simplephotos.ui.screens.viewer

import android.net.Uri
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import androidx.media3.common.MediaItem
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.repository.PhotoRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import javax.inject.Inject

@HiltViewModel
class PhotoViewerViewModel @Inject constructor(
    private val photoRepository: PhotoRepository,
    savedStateHandle: SavedStateHandle
) : ViewModel() {
    val photoId: String = savedStateHandle["photoId"] ?: ""
    var photo by mutableStateOf<PhotoEntity?>(null)
        private set
    var loading by mutableStateOf(true)
    var decryptedData by mutableStateOf<ByteArray?>(null)
    var error by mutableStateOf<String?>(null)

    /** For plain mode: the authenticated URL to the full-size image. */
    var plainImageUrl by mutableStateOf<String?>(null)
        private set

    /** For plain mode video: the authenticated URL. */
    var plainVideoUrl by mutableStateOf<String?>(null)
        private set

    var encryptionMode by mutableStateOf("plain")
        private set

    init {
        loadPhoto()
    }

    private fun loadPhoto() {
        viewModelScope.launch {
            try {
                photo = withContext(Dispatchers.IO) { photoRepository.getPhoto(photoId) }
                encryptionMode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }

                val p = photo ?: return@launch

                if (encryptionMode == "plain" && p.serverPhotoId != null) {
                    // Plain mode — build URL for Coil or ExoPlayer
                    val baseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                    val url = "$baseUrl/api/photos/${p.serverPhotoId}/file"
                    if (p.mediaType == "video") {
                        plainVideoUrl = url
                    } else {
                        plainImageUrl = url
                    }
                } else if (p.serverBlobId != null) {
                    // Encrypted mode — download and decrypt
                    withContext(Dispatchers.IO) {
                        val decrypted = photoRepository.downloadAndDecryptBlob(p.serverBlobId)
                        val payload = JSONObject(String(decrypted, Charsets.UTF_8))
                        val dataBase64 = payload.getString("data")
                        decryptedData = android.util.Base64.decode(dataBase64, android.util.Base64.NO_WRAP)
                    }
                }
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }

    fun deletePhoto(onDeleted: () -> Unit) {
        val p = photo ?: return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { photoRepository.deletePhoto(p) }
                onDeleted()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PhotoViewerScreen(
    onBack: () -> Unit,
    viewModel: PhotoViewerViewModel = hiltViewModel()
) {
    val context = LocalContext.current

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(viewModel.photo?.filename ?: "Viewer") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    IconButton(onClick = { viewModel.deletePhoto(onBack) }) {
                        Icon(Icons.Default.Delete, contentDescription = "Delete")
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = Color.Black.copy(alpha = 0.7f),
                    titleContentColor = Color.White,
                    navigationIconContentColor = Color.White,
                    actionIconContentColor = Color.White
                )
            )
        },
        containerColor = Color.Black
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .background(Color.Black),
            contentAlignment = Alignment.Center
        ) {
            when {
                viewModel.loading -> {
                    CircularProgressIndicator(color = Color.White)
                }

                viewModel.error != null -> {
                    Text(viewModel.error ?: "Error", color = Color.White, modifier = Modifier.padding(16.dp))
                }

                viewModel.photo?.mediaType == "video" -> {
                    // Video playback
                    val videoUri = when {
                        viewModel.plainVideoUrl != null -> Uri.parse(viewModel.plainVideoUrl)
                        viewModel.photo?.localPath != null -> Uri.parse(viewModel.photo?.localPath)
                        viewModel.decryptedData != null -> {
                            val tempFile = remember(viewModel.decryptedData) {
                                java.io.File.createTempFile("video_", ".mp4", context.cacheDir).apply {
                                    writeBytes(viewModel.decryptedData!!)
                                    deleteOnExit()
                                }
                            }
                            Uri.fromFile(tempFile)
                        }
                        else -> null
                    }
                    if (videoUri != null) {
                        VideoPlayer(uri = videoUri)
                    } else {
                        Text("Video not available", color = Color.White)
                    }
                }

                else -> {
                    // Photo/GIF display
                    when {
                        // Plain mode: load via authenticated URL
                        viewModel.plainImageUrl != null -> {
                            AsyncImage(
                                model = ImageRequest.Builder(context)
                                    .data(viewModel.plainImageUrl)
                                    .crossfade(true)
                                    .build(),
                                contentDescription = viewModel.photo?.filename,
                                modifier = Modifier.fillMaxSize(),
                                contentScale = ContentScale.Fit
                            )
                        }
                        // Local file
                        viewModel.photo?.localPath != null -> {
                            val bitmap = remember(viewModel.photo?.localPath) {
                                try {
                                    val inputStream = context.contentResolver.openInputStream(Uri.parse(viewModel.photo?.localPath))
                                    android.graphics.BitmapFactory.decodeStream(inputStream)
                                } catch (_: Exception) { null }
                            }
                            bitmap?.let {
                                Image(bitmap = it.asImageBitmap(), contentDescription = viewModel.photo?.filename, modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
                            }
                        }
                        // Decrypted blob data
                        viewModel.decryptedData != null -> {
                            val bitmap = remember(viewModel.decryptedData) {
                                android.graphics.BitmapFactory.decodeByteArray(viewModel.decryptedData, 0, viewModel.decryptedData!!.size)
                            }
                            bitmap?.let {
                                Image(bitmap = it.asImageBitmap(), contentDescription = viewModel.photo?.filename, modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun VideoPlayer(uri: Uri) {
    val context = LocalContext.current
    val player = remember {
        ExoPlayer.Builder(context).build().apply {
            setMediaItem(MediaItem.fromUri(uri))
            prepare()
        }
    }

    DisposableEffect(Unit) {
        onDispose { player.release() }
    }

    AndroidView(
        factory = {
            PlayerView(it).apply {
                this.player = player
                useController = true
            }
        },
        modifier = Modifier.fillMaxSize()
    )
}
