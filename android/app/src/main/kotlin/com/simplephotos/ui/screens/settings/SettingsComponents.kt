package com.simplephotos.ui.screens.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import com.simplephotos.data.remote.dto.*

// ── Reusable settings card ──────────────────────────────────────────────────

@Composable
internal fun SettingsCard(
    title: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    content: @Composable ColumnScope.() -> Unit
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(icon, contentDescription = null, modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(8.dp))
                Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            }
            Spacer(Modifier.height(12.dp))
            content()
        }
    }
}

@Composable
internal fun SettingsCard(
    title: String,
    iconPainter: androidx.compose.ui.graphics.painter.Painter,
    content: @Composable ColumnScope.() -> Unit
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(painter = iconPainter, contentDescription = null, modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(8.dp))
                Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            }
            Spacer(Modifier.height(12.dp))
            content()
        }
    }
}

@Composable
internal fun SettingsRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodyMedium)
    }
}

// ── Storage Bar ─────────────────────────────────────────────────────────────

@Composable
internal fun StorageBar(stats: StorageStatsResponse) {
    val total = stats.fsTotalBytes.toFloat().coerceAtLeast(1f)
    val photoFraction = stats.photoBytes / total
    val videoFraction = stats.videoBytes / total
    val otherFraction = stats.otherBlobBytes / total
    val freeFraction = stats.fsFreeBytes / total

    Column {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(20.dp)
                .clip(RoundedCornerShape(4.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant)
        ) {
            Row(modifier = Modifier.fillMaxSize()) {
                if (photoFraction > 0.001f) {
                    Box(modifier = Modifier.fillMaxHeight().weight(photoFraction).background(Color(0xFF3B82F6)))
                }
                if (videoFraction > 0.001f) {
                    Box(modifier = Modifier.fillMaxHeight().weight(videoFraction).background(Color(0xFF8B5CF6)))
                }
                if (otherFraction > 0.001f) {
                    Box(modifier = Modifier.fillMaxHeight().weight(otherFraction).background(Color(0xFFF59E0B)))
                }
                if (freeFraction > 0.001f) {
                    Box(modifier = Modifier.fillMaxHeight().weight(freeFraction))
                }
            }
        }
        Spacer(Modifier.height(4.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            LegendDot(Color(0xFF3B82F6), "Photos")
            LegendDot(Color(0xFF8B5CF6), "Videos")
            LegendDot(Color(0xFFF59E0B), "Other")
        }
    }
}

@Composable
internal fun LegendDot(color: Color, label: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(modifier = Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(color))
        Spacer(Modifier.width(4.dp))
        Text(label, style = MaterialTheme.typography.labelSmall)
    }
}

// ── Change Password (embedded in Account card) ─────────────────────────────

@Composable
internal fun AccountChangePasswordSection(viewModel: SettingsViewModel) {
    var expanded by remember { mutableStateOf(false) }
    var currentPassword by remember { mutableStateOf("") }
    var newPassword by remember { mutableStateOf("") }
    var confirmPassword by remember { mutableStateOf("") }
    var showPasswords by remember { mutableStateOf(false) }

    Text("Password", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
    Spacer(Modifier.height(8.dp))

    if (!expanded) {
        OutlinedButton(onClick = { expanded = true }, modifier = Modifier.fillMaxWidth()) {
            Text("Change Password")
        }
    } else {
        OutlinedTextField(
            value = currentPassword,
            onValueChange = { currentPassword = it },
            label = { Text("Current Password") },
            visualTransformation = if (showPasswords) VisualTransformation.None else PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(
            value = newPassword,
            onValueChange = { newPassword = it },
            label = { Text("New Password") },
            visualTransformation = if (showPasswords) VisualTransformation.None else PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(
            value = confirmPassword,
            onValueChange = { confirmPassword = it },
            label = { Text("Confirm New Password") },
            visualTransformation = if (showPasswords) VisualTransformation.None else PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )

        // Password strength indicator
        if (newPassword.isNotEmpty()) {
            Spacer(Modifier.height(4.dp))
            val strength = passwordStrength(newPassword)
            LinearProgressIndicator(
                progress = { strength.first },
                modifier = Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)),
                color = strength.second
            )
            Text(strength.third, style = MaterialTheme.typography.labelSmall, color = strength.second)
        }

        Spacer(Modifier.height(8.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Checkbox(checked = showPasswords, onCheckedChange = { showPasswords = it })
            Text("Show passwords", style = MaterialTheme.typography.bodySmall)
        }

        Spacer(Modifier.height(8.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = {
                expanded = false
                currentPassword = ""
                newPassword = ""
                confirmPassword = ""
            }) { Text("Cancel") }
            Button(
                onClick = {
                    if (newPassword != confirmPassword) {
                        viewModel.error = "Passwords do not match"
                    } else {
                        viewModel.changePassword(currentPassword, newPassword) {
                            expanded = false
                            currentPassword = ""
                            newPassword = ""
                            confirmPassword = ""
                        }
                    }
                },
                enabled = currentPassword.isNotEmpty() && newPassword.isNotEmpty() && confirmPassword.isNotEmpty() && !viewModel.loading
            ) { Text("Update") }
        }
    }
}

internal fun passwordStrength(password: String): Triple<Float, Color, String> {
    var score = 0
    if (password.length >= 8) score++
    if (password.length >= 12) score++
    if (password.any { it.isUpperCase() }) score++
    if (password.any { it.isLowerCase() }) score++
    if (password.any { it.isDigit() }) score++
    if (password.any { !it.isLetterOrDigit() }) score++
    return when {
        score <= 2 -> Triple(0.2f, Color(0xFFEF4444), "Weak")
        score <= 3 -> Triple(0.4f, Color(0xFFF59E0B), "Fair")
        score <= 4 -> Triple(0.7f, Color(0xFF3B82F6), "Good")
        else -> Triple(1f, Color(0xFF22C55E), "Strong")
    }
}

// ── Utilities ────────────────────────────────────────────────────────────────

internal fun formatBytes(bytes: Long): String {
    if (bytes <= 0) return "0 B"
    val units = arrayOf("B", "KB", "MB", "GB", "TB")
    val digitGroups = (Math.log10(bytes.toDouble()) / Math.log10(1024.0)).toInt().coerceAtMost(units.lastIndex)
    return "%.1f %s".format(bytes / Math.pow(1024.0, digitGroups.toDouble()), units[digitGroups])
}
