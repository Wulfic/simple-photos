package com.simplephotos.ui.screens.viewer

import android.content.Intent
import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.data.local.entities.PhotoEntity

@Composable
fun ViewerInfoPanel(
    visible: Boolean,
    photo: PhotoEntity?,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier
) {
    val context = LocalContext.current

    AnimatedVisibility(
        visible = visible,
        enter = slideInVertically { it },
        exit = slideOutVertically { it },
        modifier = modifier
    ) {
        Surface(
            color = Color(0xF2111827),
            shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .navigationBarsPadding()
                    .padding(horizontal = 20.dp, vertical = 16.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Photo Details", color = Color.White, fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
                    IconButton(onClick = onDismiss, modifier = Modifier.size(24.dp)) {
                        Icon(Icons.Default.Close, contentDescription = "Close", tint = Color.Gray, modifier = Modifier.size(16.dp))
                    }
                }
                Spacer(Modifier.height(12.dp))
                photo?.let { photo ->
                    InfoDetailRow("Filename", photo.filename)
                    InfoDetailRow("Type", photo.mimeType)
                    if (photo.width > 0 && photo.height > 0) {
                        InfoDetailRow("Dimensions", "${photo.width} × ${photo.height}")
                    }
                    photo.sizeBytes?.let { size ->
                        InfoDetailRow("Size", formatInfoBytes(size))
                    }
                    if (photo.takenAt > 0L) {
                        InfoDetailRow("Taken", java.text.SimpleDateFormat("MMM d, yyyy  h:mm a", java.util.Locale.getDefault()).format(java.util.Date(photo.takenAt)))
                    }
                    if (photo.createdAt > 0L) {
                        InfoDetailRow("Uploaded", java.text.SimpleDateFormat("MMM d, yyyy  h:mm a", java.util.Locale.getDefault()).format(java.util.Date(photo.createdAt)))
                    }
                    photo.durationSecs?.let { dur ->
                        InfoDetailRow("Duration", "%.1fs".format(dur))
                    }
                    photo.cameraModel?.let { cam ->
                        InfoDetailRow("Device", cam)
                    }
                    if (photo.latitude != null && photo.longitude != null) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(vertical = 4.dp),
                            horizontalArrangement = Arrangement.SpaceBetween
                        ) {
                            Text("Location", color = Color.Gray, fontSize = 13.sp)
                            Text(
                                "%.5f, %.5f ↗".format(photo.latitude, photo.longitude),
                                color = Color(0xFF60A5FA),
                                fontSize = 13.sp,
                                modifier = Modifier.clickable {
                                    val uri = Uri.parse(
                                        "https://www.google.com/maps?q=${photo.latitude},${photo.longitude}"
                                    )
                                    context.startActivity(
                                        Intent(Intent.ACTION_VIEW, uri)
                                    )
                                }
                            )
                        }
                    }
                }
            }
        }
    }
}
