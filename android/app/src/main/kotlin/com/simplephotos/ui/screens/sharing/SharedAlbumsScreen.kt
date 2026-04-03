/**
 * Composable screen for viewing and managing shared photo albums.
 */
package com.simplephotos.ui.screens.sharing

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import com.simplephotos.data.remote.dto.SharedAlbumInfo
import com.simplephotos.data.remote.dto.SharedAlbumMember
import com.simplephotos.data.remote.dto.SharedAlbumPhoto
import com.simplephotos.data.remote.dto.ShareableUser
import com.simplephotos.ui.theme.ThemeState

// ═════════════════════════════════════════════════════════════════════════════
// Screen
// ═════════════════════════════════════════════════════════════════════════════

/**
 * Shared Albums screen — lists albums the user owns or belongs to,
 * with inline album detail (members + photos) on tap.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SharedAlbumsScreen(
    onBack: () -> Unit,
    viewModel: SharedAlbumsViewModel = hiltViewModel(),
) {
    val isSystemDark = androidx.compose.foundation.isSystemInDarkTheme()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Shared Albums") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back")
                    }
                },
                actions = {
                    IconButton(onClick = { viewModel.showCreateDialog = true }) {
                        Icon(Icons.Default.Add, "Create album")
                    }
                }
            )
        }
    ) { padding ->
        Column(modifier = Modifier.padding(padding).fillMaxSize()) {
            // Error banner
            viewModel.error?.let { msg ->
                Text(
                    msg,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(16.dp),
                    fontSize = 13.sp
                )
            }

            if (viewModel.loading) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
            } else if (viewModel.albums.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text("No shared albums yet", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            } else {
                LazyColumn(modifier = Modifier.fillMaxSize()) {
                    items(viewModel.albums, key = { it.id }) { album ->
                        SharedAlbumCard(
                            album = album,
                            isSelected = viewModel.selectedAlbum?.id == album.id,
                            onClick = {
                                if (viewModel.selectedAlbum?.id == album.id)
                                    viewModel.closeDetail()
                                else
                                    viewModel.selectAlbum(album)
                            },
                            onDelete = if (album.isOwner) ({ viewModel.albumToDelete = album }) else null
                        )

                        // Inline detail panel when this album is selected
                        if (viewModel.selectedAlbum?.id == album.id) {
                            AlbumDetailPanel(viewModel)
                        }
                    }
                }
            }
        }
    }

    // ── Dialogs ──────────────────────────────────────────────────────────

    // Create album dialog
    if (viewModel.showCreateDialog) {
        AlertDialog(
            onDismissRequest = { viewModel.showCreateDialog = false },
            title = { Text("New Shared Album") },
            text = {
                OutlinedTextField(
                    value = viewModel.newAlbumName,
                    onValueChange = { viewModel.newAlbumName = it },
                    label = { Text("Album name") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
            },
            confirmButton = {
                TextButton(onClick = { viewModel.createAlbum() }) { Text("Create") }
            },
            dismissButton = {
                TextButton(onClick = { viewModel.showCreateDialog = false }) { Text("Cancel") }
            }
        )
    }

    // Delete confirmation dialog
    viewModel.albumToDelete?.let { album ->
        AlertDialog(
            onDismissRequest = { viewModel.albumToDelete = null },
            title = { Text("Delete Album") },
            text = { Text("Delete \"${album.name}\"? This cannot be undone.") },
            confirmButton = {
                TextButton(
                    onClick = { viewModel.confirmDeleteAlbum() },
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)
                ) { Text("Delete") }
            },
            dismissButton = {
                TextButton(onClick = { viewModel.albumToDelete = null }) { Text("Cancel") }
            }
        )
    }

    // Add member picker dialog
    if (viewModel.showAddMemberDialog) {
        val existingUserIds = viewModel.members.map { it.userId }.toSet()
        val candidates = viewModel.availableUsers.filter { it.id !in existingUserIds }

        AlertDialog(
            onDismissRequest = { viewModel.showAddMemberDialog = false },
            title = { Text("Add Member") },
            text = {
                if (candidates.isEmpty()) {
                    Text("No users available to add.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    LazyColumn(modifier = Modifier.heightIn(max = 300.dp)) {
                        items(candidates) { user ->
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable { viewModel.addMember(user.id) }
                                    .padding(vertical = 10.dp, horizontal = 4.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                // Avatar
                                Box(
                                    modifier = Modifier
                                        .size(32.dp)
                                        .clip(CircleShape)
                                        .background(
                                            Brush.linearGradient(
                                                listOf(Color(0xFF3B82F6), Color(0xFF9333EA))
                                            )
                                        ),
                                    contentAlignment = Alignment.Center
                                ) {
                                    Text(
                                        user.username.take(1).uppercase(),
                                        color = Color.White,
                                        fontSize = 13.sp,
                                        fontWeight = FontWeight.Bold
                                    )
                                }
                                Spacer(Modifier.width(12.dp))
                                Text(user.username, fontSize = 15.sp)
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { viewModel.showAddMemberDialog = false }) { Text("Close") }
            },
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Album card
// ─────────────────────────────────────────────────────────────────────────────

@Composable
private fun SharedAlbumCard(
    album: SharedAlbumInfo,
    isSelected: Boolean,
    onClick: () -> Unit,
    onDelete: (() -> Unit)?,
) {
    val bg = if (isSelected)
        MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.4f)
    else
        Color.Transparent

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(bg)
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        // Icon
        Box(
            modifier = Modifier
                .size(44.dp)
                .clip(RoundedCornerShape(10.dp))
                .background(MaterialTheme.colorScheme.primaryContainer),
            contentAlignment = Alignment.Center
        ) {
            Icon(Icons.Default.Person, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
        }
        Spacer(Modifier.width(12.dp))

        // Text
        Column(modifier = Modifier.weight(1f)) {
            Text(
                album.name,
                fontWeight = FontWeight.SemiBold,
                fontSize = 15.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            Text(
                buildString {
                    append("by ${album.ownerUsername}")
                    append(" · ${album.photoCount} photos")
                    append(" · ${album.memberCount} members")
                },
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }

        // Delete button (owner only)
        if (onDelete != null) {
            IconButton(onClick = onDelete) {
                Icon(
                    Icons.Default.Delete,
                    contentDescription = "Delete",
                    tint = MaterialTheme.colorScheme.error,
                    modifier = Modifier.size(20.dp)
                )
            }
        }
    }
    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.3f))
}

// ─────────────────────────────────────────────────────────────────────────────
// Inline detail panel (members + photos)
// ─────────────────────────────────────────────────────────────────────────────

@Composable
private fun AlbumDetailPanel(viewModel: SharedAlbumsViewModel) {
    val album = viewModel.selectedAlbum ?: return

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.3f))
            .padding(horizontal = 20.dp, vertical = 12.dp)
    ) {
        if (viewModel.detailLoading) {
            Box(Modifier.fillMaxWidth().height(80.dp), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
            }
            return
        }

        // ── Members section ──────────────────────────────────────────
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Members", fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
            Spacer(Modifier.weight(1f))
            if (album.isOwner) {
                TextButton(
                    onClick = { viewModel.openAddMemberDialog() },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp)
                ) {
                    Icon(Icons.Default.Add, null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text("Add", fontSize = 13.sp)
                }
            }
        }
        Spacer(Modifier.height(4.dp))

        if (viewModel.members.isEmpty()) {
            Text("No members", fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
        } else {
            viewModel.members.forEach { member ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        modifier = Modifier
                            .size(28.dp)
                            .clip(CircleShape)
                            .background(
                                Brush.linearGradient(
                                    listOf(Color(0xFF3B82F6), Color(0xFF9333EA))
                                )
                            ),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            member.username.take(1).uppercase(),
                            color = Color.White,
                            fontSize = 11.sp,
                            fontWeight = FontWeight.Bold
                        )
                    }
                    Spacer(Modifier.width(8.dp))
                    Text(member.username, fontSize = 14.sp, modifier = Modifier.weight(1f))
                    if (album.isOwner) {
                        IconButton(
                            onClick = { viewModel.removeMember(member.userId) },
                            modifier = Modifier.size(28.dp)
                        ) {
                            Icon(
                                Icons.Default.Close,
                                contentDescription = "Remove",
                                tint = MaterialTheme.colorScheme.error,
                                modifier = Modifier.size(16.dp)
                            )
                        }
                    }
                }
            }
        }

        Spacer(Modifier.height(12.dp))

        // ── Photos section ───────────────────────────────────────────
        Text("Photos", fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
        Spacer(Modifier.height(4.dp))

        if (viewModel.photos.isEmpty()) {
            Text("No photos shared yet", fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
        } else {
            viewModel.photos.forEach { photo ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 3.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        photo.photoRef,
                        fontSize = 13.sp,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f)
                    )
                    Text(
                        photo.refType,
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 8.dp)
                    )
                    IconButton(
                        onClick = { viewModel.removePhoto(photo.id) },
                        modifier = Modifier.size(24.dp)
                    ) {
                        Icon(
                            Icons.Default.Close,
                            contentDescription = "Remove",
                            tint = MaterialTheme.colorScheme.error,
                            modifier = Modifier.size(14.dp)
                        )
                    }
                }
            }
        }
    }
    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.3f))
}
