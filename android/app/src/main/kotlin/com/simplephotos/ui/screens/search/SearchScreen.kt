/**
 * Composable search screen providing tag-based and text-based photo search
 * with a filterable results grid.
 */
package com.simplephotos.ui.screens.search

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.R
import com.simplephotos.data.remote.dto.SearchResult
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.theme.ThemeState
import java.io.File

// ── Screen ──────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun SearchScreen(
    onPhotoClick: (String) -> Unit,
    onGalleryClick: () -> Unit,
    onAlbumsClick: () -> Unit,
    onTrashClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onSecureGalleryClick: () -> Unit = {},
    onSharedAlbumsClick: () -> Unit = {},
    onDiagnosticsClick: () -> Unit = {},
    onLogout: () -> Unit,
    isAdmin: Boolean = false,
    viewModel: SearchViewModel = hiltViewModel()
) {
    val focusRequester = remember { FocusRequester() }
    val isSystemDark = androidx.compose.foundation.isSystemInDarkTheme()

    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
    }

    Scaffold(
        topBar = {
            AppHeader(
                activeTab = ActiveTab.SEARCH,
                username = viewModel.username,
                navigation = HeaderNavigation(
                    onGalleryClick = onGalleryClick,
                    onAlbumsClick = onAlbumsClick,
                    onSearchClick = { /* already on search */ },
                    onTrashClick = onTrashClick,
                    onSettingsClick = onSettingsClick,
                    onSecureGalleryClick = onSecureGalleryClick,
                    onSharedAlbumsClick = onSharedAlbumsClick,
                    onDiagnosticsClick = onDiagnosticsClick,
                    onLogout = { viewModel.logout(onLogout) },
                    onToggleTheme = { ThemeState.toggle(viewModel.dataStore, ThemeState.isDark(isSystemDark)) },
                    isAdmin = isAdmin
                )
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
        ) {
            // Search input
            OutlinedTextField(
                value = viewModel.query,
                onValueChange = { viewModel.updateQuery(it) },
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .focusRequester(focusRequester),
                placeholder = { Text("Search tags, filenames, dates, types…") },
                leadingIcon = {
                    Icon(
                        painter = painterResource(R.drawable.ic_magnify_glass),
                        contentDescription = "Search",
                        modifier = Modifier.size(20.dp)
                    )
                },
                trailingIcon = {
                    if (viewModel.query.isNotEmpty()) {
                        IconButton(onClick = { viewModel.updateQuery("") }) {
                            Text("✕", fontSize = 16.sp)
                        }
                    }
                },
                singleLine = true,
                shape = RoundedCornerShape(16.dp)
            )

            // Loading
            if (viewModel.isLoading) {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(top = 32.dp),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator()
                }
            }

            // No results
            if (viewModel.searched && !viewModel.isLoading && viewModel.results.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(top = 32.dp),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            "No results found",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            "Try a different tag or filename",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f)
                        )
                    }
                }
            }

            // Results count
            if (viewModel.results.isNotEmpty()) {
                Text(
                    "${viewModel.results.size} result${if (viewModel.results.size != 1) "s" else ""}",
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            // Results grid — justified layout matching the gallery
            if (viewModel.results.isNotEmpty()) {
                val targetRowHeight = 120.dp
                com.simplephotos.ui.components.JustifiedGrid(
                    items = viewModel.results,
                    getAspectRatio = { r ->
                        val w = r.width ?: 0
                        val h = r.height ?: 0
                        if (w > 0 && h > 0) w.toFloat() / h.toFloat()
                        else 1f
                    },
                    getKey = { it.id },
                    targetRowHeight = targetRowHeight,
                    gap = 2.dp,
                ) { result, widthDp, heightDp ->
                    val localPhoto = viewModel.localPhotoMap[result.id]
                    val clickId = localPhoto?.localId ?: result.id
                    SearchResultTile(
                        result = result,
                        serverBaseUrl = viewModel.serverBaseUrl,
                        localThumbnailPath = localPhoto?.thumbnailPath,
                        onClick = { onPhotoClick(clickId) },
                        widthDp = widthDp,
                        heightDp = heightDp
                    )
                }
            }

            // Empty state
            if (viewModel.query.isEmpty() && !viewModel.isLoading) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(
                            painter = painterResource(R.drawable.ic_magnify_glass),
                            contentDescription = null,
                            modifier = Modifier.size(48.dp),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.3f)
                        )
                        Spacer(Modifier.height(12.dp))
                        Text(
                            "Search your library",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            "Search by tags, filenames,\ndates, or media types",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                            textAlign = TextAlign.Center
                        )
                    }
                }
            }
        }
    }
}

// ── Result tile ─────────────────────────────────────────────────────────────

@Composable
private fun SearchResultTile(
    result: SearchResult,
    serverBaseUrl: String,
    localThumbnailPath: String?,
    onClick: () -> Unit,
    widthDp: Dp = 100.dp,
    heightDp: Dp = 100.dp
) {
    val context = LocalContext.current

    // Prefer local cached thumbnail, fall back to server URL
    val imageModel: Any? = when {
        localThumbnailPath != null -> File(localThumbnailPath)
        serverBaseUrl.isNotEmpty() -> "$serverBaseUrl/api/photos/${result.id}/thumb"
        else -> null
    }

    Box(
        modifier = Modifier
            .size(widthDp, heightDp)
            .clip(RoundedCornerShape(4.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable(onClick = onClick)
    ) {
        if (imageModel != null) {
            AsyncImage(
                model = ImageRequest.Builder(context)
                    .data(imageModel)
                    .crossfade(true)
                    .size(512)
                    .build(),
                contentDescription = result.filename,
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
        } else {
            Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surfaceVariant) {
                Box(contentAlignment = Alignment.Center) {
                    Text(result.filename.take(8), style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Center, modifier = Modifier.padding(4.dp))
                }
            }
        }

        // Video badge
        if (result.mediaType == "video") {
            Surface(
                modifier = Modifier
                    .align(Alignment.BottomEnd)
                    .padding(4.dp),
                color = Color.Black.copy(alpha = 0.6f),
                shape = RoundedCornerShape(4.dp)
            ) {
                Text(
                    "▶",
                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                    color = Color.White,
                    fontSize = 10.sp
                )
            }
        }

        // GIF badge
        if (result.mediaType == "gif") {
            Surface(
                modifier = Modifier
                    .align(Alignment.BottomEnd)
                    .padding(4.dp),
                color = Color.Black.copy(alpha = 0.6f),
                shape = RoundedCornerShape(4.dp)
            ) {
                Text(
                    "GIF",
                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                    color = Color.White,
                    fontSize = 10.sp
                )
            }
        }

        // Tag chips at top
        if (result.tags.isNotEmpty()) {
            Row(
                modifier = Modifier
                    .align(Alignment.TopStart)
                    .padding(4.dp),
                horizontalArrangement = Arrangement.spacedBy(2.dp)
            ) {
                result.tags.take(2).forEach { tag ->
                    Surface(
                        color = Color.Black.copy(alpha = 0.6f),
                        shape = CircleShape
                    ) {
                        Text(
                            text = tag,
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                            color = Color.White,
                            fontSize = 9.sp,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }
                if (result.tags.size > 2) {
                    Surface(
                        color = Color.Black.copy(alpha = 0.6f),
                        shape = CircleShape
                    ) {
                        Text(
                            text = "+${result.tags.size - 2}",
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                            color = Color.White,
                            fontSize = 9.sp
                        )
                    }
                }
            }
        }
    }
}
