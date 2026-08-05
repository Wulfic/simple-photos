/**
 * "Add to Album" picker — choose an existing album for a selection, or create a
 * new one and add to it.
 *
 * Lifted out of `GalleryScreen.kt` (where it was `private`) when the album
 * detail screen needed the identical control for Z1's "+ add these to another
 * album". Copying it was the alternative, and this repo has recorded ten
 * separate instances of one thing derived twice and drifting — a picker whose
 * two copies disagree about whether "Create & Add" exists is the same failure in
 * a smaller costume.
 */
package com.simplephotos.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items as lazyItems
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import com.simplephotos.R
import com.simplephotos.data.local.entities.AlbumEntity

/**
 * @param albums albums offered as targets. The caller decides what to exclude —
 *   the album detail screen drops the album already open, the gallery drops
 *   nothing — because "which albums make sense here" is a property of the
 *   screen, not of the picker.
 * @param subtitle optional line under the title. Z1's album-detail caller uses
 *   it to say the photos stay in the current album too, which is the whole point
 *   of the control being an add rather than a move.
 */
@Composable
fun AlbumPickerDialog(
    albums: List<AlbumEntity>,
    onDismiss: () -> Unit,
    onAlbumSelected: (String) -> Unit,
    onCreateAlbum: (String) -> Unit,
    subtitle: String? = null,
) {
    var showCreateField by remember { mutableStateOf(false) }
    var newAlbumName by remember { mutableStateOf("") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add to Album") },
        text = {
            Column(modifier = Modifier.widthIn(min = 260.dp)) {
                subtitle?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(bottom = 8.dp)
                    )
                }

                if (albums.isEmpty() && !showCreateField) {
                    Text(
                        "No albums yet. Create one to get started.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(vertical = 8.dp)
                    )
                }

                if (albums.isNotEmpty()) {
                    LazyColumn(modifier = Modifier.heightIn(max = 240.dp)) {
                        lazyItems(albums, key = { it.localId }) { album ->
                            Surface(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable { onAlbumSelected(album.localId) },
                                shape = RoundedCornerShape(8.dp)
                            ) {
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .padding(horizontal = 12.dp, vertical = 12.dp),
                                    verticalAlignment = Alignment.CenterVertically
                                ) {
                                    Icon(
                                        painter = painterResource(R.drawable.ic_folder),
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.size(20.dp)
                                    )
                                    Spacer(Modifier.width(12.dp))
                                    Text(album.name, style = MaterialTheme.typography.bodyLarge)
                                }
                            }
                        }
                    }
                    Spacer(Modifier.height(8.dp))
                }

                if (showCreateField) {
                    OutlinedTextField(
                        value = newAlbumName,
                        onValueChange = { newAlbumName = it },
                        label = { Text("Album name") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth()
                    )
                } else {
                    TextButton(
                        onClick = { showCreateField = true },
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Icon(Icons.Default.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(4.dp))
                        Text("Create New Album")
                    }
                }
            }
        },
        confirmButton = {
            if (showCreateField) {
                Button(
                    onClick = { if (newAlbumName.isNotBlank()) onCreateAlbum(newAlbumName.trim()) },
                    enabled = newAlbumName.isNotBlank()
                ) { Text("Create & Add") }
            }
        },
        dismissButton = {
            OutlinedButton(onClick = onDismiss) { Text("Cancel") }
        }
    )
}
