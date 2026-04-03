package com.simplephotos.ui.screens.viewer

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.R
import com.simplephotos.data.local.entities.PhotoEntity

@Composable
fun ViewerEditPanel(
    visible: Boolean,
    editTab: String,
    onEditTabChange: (String) -> Unit,
    brightnessValue: Float,
    onBrightnessChange: (Float) -> Unit,
    rotateValue: Int,
    onRotateLeft: () -> Unit,
    onRotateRight: () -> Unit,
    trimStart: Float,
    onTrimStartChange: (Float) -> Unit,
    trimEnd: Float,
    onTrimEndChange: (Float) -> Unit,
    mediaDuration: Float,
    currentPhoto: PhotoEntity?,
    onSave: () -> Unit,
    onSaveCopy: () -> Unit,
    onReset: () -> Unit,
    onCancel: () -> Unit,
    modifier: Modifier = Modifier
) {
    AnimatedVisibility(
        visible = visible,
        enter = slideInVertically { it },
        exit = slideOutVertically { it },
        modifier = modifier
    ) {
        val currentMediaType = currentPhoto?.mediaType ?: "photo"
        val isPhoto = currentMediaType == "photo"
        val isVideo = currentMediaType == "video"
        val isAudio = currentMediaType == "audio"
        val showCrop = isPhoto || isVideo
        val showBrightness = isPhoto || isVideo
        val showRotate = isPhoto || isVideo
        val showTrim = isVideo || isAudio

        Surface(
            color = Color(0xE6111827),
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .navigationBarsPadding()
                    .padding(horizontal = 16.dp, vertical = 12.dp)
            ) {
                // Tab selector — only show tabs available for this media type
                Row(
                    modifier = Modifier.fillMaxWidth().padding(bottom = 8.dp),
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    if (showCrop) {
                        Surface(
                            shape = RoundedCornerShape(50),
                            color = if (editTab == "crop") Color.White else Color.Transparent,
                            modifier = Modifier
                                .padding(horizontal = 4.dp)
                                .clip(RoundedCornerShape(50))
                                .clickable { onEditTabChange("crop") }
                        ) {
                            Text(
                                "Crop",
                                color = if (editTab == "crop") Color.Black else Color.White,
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium
                            )
                        }
                    }
                    if (showBrightness) {
                        Surface(
                            shape = RoundedCornerShape(50),
                            color = if (editTab == "brightness") Color.White else Color.Transparent,
                            modifier = Modifier
                                .padding(horizontal = 4.dp)
                                .clip(RoundedCornerShape(50))
                                .clickable { onEditTabChange("brightness") }
                        ) {
                            Text(
                                "Brightness",
                                color = if (editTab == "brightness") Color.Black else Color.White,
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium
                            )
                        }
                    }
                    if (showRotate) {
                        Surface(
                            shape = RoundedCornerShape(50),
                            color = if (editTab == "rotate") Color.White else Color.Transparent,
                            modifier = Modifier
                                .padding(horizontal = 4.dp)
                                .clip(RoundedCornerShape(50))
                                .clickable { onEditTabChange("rotate") }
                        ) {
                            Text(
                                "Rotate",
                                color = if (editTab == "rotate") Color.Black else Color.White,
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium
                            )
                        }
                    }
                    if (showTrim) {
                        Surface(
                            shape = RoundedCornerShape(50),
                            color = if (editTab == "trim") Color.White else Color.Transparent,
                            modifier = Modifier
                                .padding(horizontal = 4.dp)
                                .clip(RoundedCornerShape(50))
                                .clickable { onEditTabChange("trim") }
                        ) {
                            Text(
                                "Trim",
                                color = if (editTab == "trim") Color.Black else Color.White,
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium
                            )
                        }
                    }
                }

                Spacer(Modifier.height(8.dp))

                when (editTab) {
                    "brightness" -> {
                        Text("Brightness: ${brightnessValue.toInt()}", color = Color.White, fontSize = 12.sp)
                        Slider(
                            value = brightnessValue,
                            onValueChange = onBrightnessChange,
                            valueRange = -100f..100f,
                            modifier = Modifier.fillMaxWidth(),
                            colors = SliderDefaults.colors(
                                thumbColor = Color(0xFF60A5FA),
                                activeTrackColor = Color(0xFF3B82F6)
                            )
                        )
                    }
                    "crop" -> {
                        // Crop overlay is drawn on the main photo above; just show instructions here
                        Text("Drag the corner handles on the photo to adjust crop", color = Color.Gray, fontSize = 12.sp)
                    }
                    "rotate" -> {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.Center,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            // Rotate left button
                            IconButton(onClick = onRotateLeft) {
                                Icon(
                                    painter = painterResource(R.drawable.ic_back_arrow),
                                    contentDescription = "Rotate left",
                                    tint = Color.White,
                                    modifier = Modifier.size(20.dp)
                                )
                            }
                            Spacer(Modifier.width(16.dp))
                            Text(
                                "${rotateValue}°",
                                color = Color.White,
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium
                            )
                            Spacer(Modifier.width(16.dp))
                            // Rotate right button
                            IconButton(onClick = onRotateRight) {
                                Icon(
                                    painter = painterResource(R.drawable.ic_back_arrow),
                                    contentDescription = "Rotate right",
                                    tint = Color.White,
                                    modifier = Modifier
                                        .size(20.dp)
                                        .graphicsLayer(scaleX = -1f)
                                )
                            }
                        }
                    }
                    "trim" -> {
                        if (mediaDuration > 0f) {
                            // Time labels
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween
                            ) {
                                Text(formatTrimTime(trimStart), color = Color.Gray, fontSize = 12.sp)
                                Text(
                                    "${formatTrimTime(trimEnd - trimStart)} selected",
                                    color = Color.White,
                                    fontSize = 12.sp,
                                    fontWeight = FontWeight.Medium
                                )
                                Text(formatTrimTime(trimEnd), color = Color.Gray, fontSize = 12.sp)
                            }
                            Spacer(Modifier.height(4.dp))

                            // ── Single-bar dual-thumb trim slider (matches web) ──
                            val thumbRadiusDp = 8.dp
                            val trackHeightDp = 6.dp

                            BoxWithConstraints(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(40.dp)
                            ) {
                                val trackW = constraints.maxWidth.toFloat()
                                val density = androidx.compose.ui.platform.LocalDensity.current

                                // Background track
                                Box(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .height(trackHeightDp)
                                        .align(Alignment.Center)
                                        .clip(RoundedCornerShape(50))
                                        .background(Color.White.copy(alpha = 0.1f))
                                )

                                // Selected range highlight (blue bar between start and end)
                                val startFrac = (trimStart / mediaDuration).coerceIn(0f, 1f)
                                val endFrac = (trimEnd / mediaDuration).coerceIn(0f, 1f)
                                Box(
                                    modifier = Modifier
                                        .height(trackHeightDp)
                                        .align(Alignment.CenterStart)
                                        .offset(x = with(density) { (startFrac * trackW).toDp() })
                                        .width(with(density) { ((endFrac - startFrac) * trackW).toDp() })
                                        .clip(RoundedCornerShape(50))
                                        .background(Color(0xFF3B82F6).copy(alpha = 0.6f))
                                )

                                // Start thumb
                                val startThumbX = with(density) { (startFrac * trackW).toDp() }
                                Box(
                                    modifier = Modifier
                                        .offset(x = startThumbX - thumbRadiusDp, y = 0.dp)
                                        .size(thumbRadiusDp * 2)
                                        .align(Alignment.CenterStart)
                                        .clip(CircleShape)
                                        .background(Color.White)
                                        .then(
                                            Modifier.drawBehind {
                                                drawCircle(
                                                    color = Color(0xFF3B82F6),
                                                    radius = size.minDimension / 2f,
                                                    style = Stroke(width = 2.dp.toPx())
                                                )
                                            }
                                        )
                                        .pointerInput(mediaDuration) {
                                            detectDragGestures { change, dragAmount ->
                                                change.consume()
                                                val dx = dragAmount.x / trackW
                                                onTrimStartChange(
                                                    (trimStart + dx * mediaDuration)
                                                        .coerceIn(0f, trimEnd - 0.5f)
                                                )
                                            }
                                        }
                                )

                                // End thumb
                                val endThumbX = with(density) { (endFrac * trackW).toDp() }
                                Box(
                                    modifier = Modifier
                                        .offset(x = endThumbX - thumbRadiusDp, y = 0.dp)
                                        .size(thumbRadiusDp * 2)
                                        .align(Alignment.CenterStart)
                                        .clip(CircleShape)
                                        .background(Color.White)
                                        .then(
                                            Modifier.drawBehind {
                                                drawCircle(
                                                    color = Color(0xFF3B82F6),
                                                    radius = size.minDimension / 2f,
                                                    style = Stroke(width = 2.dp.toPx())
                                                )
                                            }
                                        )
                                        .pointerInput(mediaDuration) {
                                            detectDragGestures { change, dragAmount ->
                                                change.consume()
                                                val dx = dragAmount.x / trackW
                                                onTrimEndChange(
                                                    (trimEnd + dx * mediaDuration)
                                                        .coerceIn(trimStart + 0.5f, mediaDuration)
                                                )
                                            }
                                        }
                                )
                            }

                            Spacer(Modifier.height(4.dp))
                            Text(
                                "Full duration: ${formatTrimTime(mediaDuration)}",
                                color = Color.Gray.copy(alpha = 0.6f),
                                fontSize = 11.sp,
                                modifier = Modifier.fillMaxWidth(),
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center
                            )
                        } else {
                            // Duration not yet known
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.Center,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                CircularProgressIndicator(
                                    color = Color.Gray,
                                    modifier = Modifier.size(16.dp),
                                    strokeWidth = 2.dp
                                )
                                Spacer(Modifier.width(8.dp))
                                Text("Loading duration…", color = Color.Gray, fontSize = 12.sp)
                            }
                            Spacer(Modifier.height(4.dp))
                            Text(
                                "Play the media briefly if the duration doesn't appear.",
                                color = Color.Gray.copy(alpha = 0.5f),
                                fontSize = 11.sp,
                                modifier = Modifier.fillMaxWidth(),
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center
                            )
                        }
                    }
                }

                Spacer(Modifier.height(8.dp))

                // Action buttons — matches web: Save, Save Copy, Reset, Cancel
                Row(
                    modifier = Modifier.fillMaxWidth().padding(bottom = 8.dp),
                    horizontalArrangement = Arrangement.Center
                ) {
                    Button(
                        onClick = onSave,
                        colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF2563EB)),
                        shape = RoundedCornerShape(8.dp),
                        modifier = Modifier.padding(horizontal = 4.dp)
                    ) {
                        Text("Save", color = Color.White, fontSize = 14.sp)
                    }
                    Button(
                        onClick = onSaveCopy,
                        colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF16A34A)),
                        shape = RoundedCornerShape(8.dp),
                        modifier = Modifier.padding(horizontal = 4.dp)
                    ) {
                        Text("Save Copy", color = Color.White, fontSize = 14.sp)
                    }
                    if (currentPhoto?.cropMetadata != null) {
                        Button(
                            onClick = onReset,
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF4B5563)),
                            shape = RoundedCornerShape(8.dp),
                            modifier = Modifier.padding(horizontal = 4.dp)
                        ) {
                            Text("Reset", color = Color.White, fontSize = 14.sp)
                        }
                    }
                    Button(
                        onClick = onCancel,
                        colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF374151)),
                        shape = RoundedCornerShape(8.dp),
                        modifier = Modifier.padding(horizontal = 4.dp)
                    ) {
                        Text("Cancel", color = Color.White, fontSize = 14.sp)
                    }
                }
            }
        }
    }
}

/** Format seconds as MM:SS or HH:MM:SS */
private fun formatTrimTime(secs: Float): String {
    val s = secs.coerceAtLeast(0f).toInt()
    val h = s / 3600
    val m = (s % 3600) / 60
    val sec = s % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, sec) else "%d:%02d".format(m, sec)
}
