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

    init {
        loadPhoto()
    }

    private fun loadPhoto() {
        viewModelScope.launch {
            try {
                photo = photoRepository.getPhoto(photoId)

                // If we have a server blob, decrypt it to get the media bytes
                photo?.serverBlobId?.let { blobId ->
                    withContext(Dispatchers.IO) {
                        val decrypted = photoRepository.downloadAndDecryptBlob(blobId)
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
                photoRepository.deletePhoto(p)
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
                    Text(
                        viewModel.error ?: "Error",
                        color = Color.White,
                        modifier = Modifier.padding(16.dp)
                    )
                }

                viewModel.photo?.mediaType == "video" -> {
                    // Video playback — use local path if available, or decrypted data
                    val localPath = viewModel.photo?.localPath
                    if (localPath != null) {
                        VideoPlayer(uri = Uri.parse(localPath))
                    } else if (viewModel.decryptedData != null) {
                        // Write decrypted video to a temp file for ExoPlayer
                        val tempUri = remember(viewModel.decryptedData) {
                            val tempFile = java.io.File.createTempFile("video_", ".mp4", context.cacheDir)
                            tempFile.writeBytes(viewModel.decryptedData!!)
                            tempFile.deleteOnExit()
                            Uri.fromFile(tempFile)
                        }
                        VideoPlayer(uri = tempUri)
                    } else {
                        Text("Video not available", color = Color.White)
                    }
                }

                else -> {
                    // Photo or GIF — show from local path or decrypted data
                    val localPath = viewModel.photo?.localPath
                    if (localPath != null) {
                        val bitmap = remember(localPath) {
                            try {
                                val inputStream = context.contentResolver.openInputStream(Uri.parse(localPath))
                                android.graphics.BitmapFactory.decodeStream(inputStream)
                            } catch (e: Exception) {
                                null
                            }
                        }
                        bitmap?.let {
                            Image(
                                bitmap = it.asImageBitmap(),
                                contentDescription = viewModel.photo?.filename,
                                modifier = Modifier.fillMaxSize(),
                                contentScale = ContentScale.Fit
                            )
                        }
                    } else if (viewModel.decryptedData != null) {
                        val bitmap = remember(viewModel.decryptedData) {
                            android.graphics.BitmapFactory.decodeByteArray(
                                viewModel.decryptedData, 0, viewModel.decryptedData!!.size
                            )
                        }
                        bitmap?.let {
                            Image(
                                bitmap = it.asImageBitmap(),
                                contentDescription = viewModel.photo?.filename,
                                modifier = Modifier.fillMaxSize(),
                                contentScale = ContentScale.Fit
                            )
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
