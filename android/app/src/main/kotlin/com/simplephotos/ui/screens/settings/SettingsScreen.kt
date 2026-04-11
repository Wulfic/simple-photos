/**
 * Composable screen that displays and manages user-configurable application settings.
 */
package com.simplephotos.ui.screens.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import com.simplephotos.data.remote.dto.*
import com.simplephotos.R
import androidx.compose.ui.res.painterResource

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    onLogout: () -> Unit,
    onSetup2fa: () -> Unit,
    onBackupFolders: () -> Unit,
    viewModel: SettingsViewModel = hiltViewModel()
) {
    val scrollState = rememberScrollState()

    // Check admin status on first compose
    LaunchedEffect(Unit) {
        viewModel.checkAdmin()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(painter = painterResource(R.drawable.ic_back_arrow), contentDescription = "Back", modifier = Modifier.size(24.dp))
                    }
                }
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
                .verticalScroll(scrollState)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            // ── Account ──────────────────────────────────────────────────
            SettingsCard(title = "Account", icon = Icons.Default.Person) {
                SettingsRow("Server", viewModel.serverUrl)
                SettingsRow("Username", viewModel.username)
                SettingsRow("Mode", "Encrypted")

                // ── Change Password (inline) ─────────────────────────────
                HorizontalDivider(modifier = Modifier.padding(vertical = 12.dp))
                AccountChangePasswordSection(viewModel)

                // ── Two-Factor Authentication (inline) ───────────────────
                HorizontalDivider(modifier = Modifier.padding(vertical = 12.dp))
                Text("Two-Factor Authentication", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
                Spacer(Modifier.height(8.dp))

                if (viewModel.totpLoading) {
                    CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                } else if (viewModel.totpEnabled) {
                    // 2FA is enabled — show status and disable option
                    var showDisableDialog by remember { mutableStateOf(false) }
                    var disableCode by remember { mutableStateOf("") }
                    var disableError by remember { mutableStateOf<String?>(null) }
                    var disabling by remember { mutableStateOf(false) }

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(Icons.Default.CheckCircle, contentDescription = null, modifier = Modifier.size(18.dp), tint = Color(0xFF22C55E))
                            Spacer(Modifier.width(6.dp))
                            Text("Enabled", color = Color(0xFF22C55E))
                        }
                        OutlinedButton(
                            onClick = { showDisableDialog = true },
                            colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error)
                        ) { Text("Disable") }
                    }

                    if (showDisableDialog) {
                        AlertDialog(
                            onDismissRequest = { if (!disabling) showDisableDialog = false },
                            title = { Text("Disable 2FA") },
                            text = {
                                Column {
                                    Text("Enter your current TOTP code to disable two-factor authentication.")
                                    Spacer(Modifier.height(12.dp))
                                    OutlinedTextField(
                                        value = disableCode,
                                        onValueChange = { disableCode = it.filter { c -> c.isDigit() }.take(6); disableError = null },
                                        label = { Text("6-digit code") },
                                        singleLine = true,
                                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                                        isError = disableError != null,
                                        supportingText = disableError?.let { e -> { Text(e) } },
                                        modifier = Modifier.fillMaxWidth(),
                                        enabled = !disabling
                                    )
                                }
                            },
                            confirmButton = {
                                Button(
                                    onClick = {
                                        disabling = true
                                        viewModel.disable2fa(disableCode,
                                            onSuccess = { disabling = false; showDisableDialog = false; disableCode = "" },
                                            onError = { disabling = false; disableError = it }
                                        )
                                    },
                                    enabled = disableCode.length == 6 && !disabling,
                                    colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error)
                                ) {
                                    if (disabling) { CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp); Spacer(Modifier.width(8.dp)) }
                                    Text("Disable 2FA")
                                }
                            },
                            dismissButton = { TextButton(onClick = { showDisableDialog = false; disableCode = "" }, enabled = !disabling) { Text("Cancel") } }
                        )
                    }
                } else {
                    // 2FA is disabled — show setup button
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text("Not enabled", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        OutlinedButton(onClick = onSetup2fa) { Text("Enable 2FA") }
                    }
                }
            }

            // ── Biometric Lock ───────────────────────────────────────────
            SettingsCard(title = "Security", iconPainter = painterResource(R.drawable.ic_lock)) {
                Text(
                    "Require fingerprint or face unlock to open the app.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(8.dp))

                // Password verification dialog state
                var showBiometricPasswordDialog by remember { mutableStateOf(false) }
                var biometricPassword by remember { mutableStateOf("") }
                var biometricPasswordError by remember { mutableStateOf<String?>(null) }
                var biometricPasswordVisible by remember { mutableStateOf(false) }
                var biometricVerifying by remember { mutableStateOf(false) }

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Biometric Lock")
                    Switch(
                        checked = viewModel.biometricEnabled,
                        onCheckedChange = { enabling ->
                            if (enabling) {
                                // Prompt for password before enabling
                                biometricPassword = ""
                                biometricPasswordError = null
                                showBiometricPasswordDialog = true
                            } else {
                                // Disabling doesn't need verification
                                viewModel.disableBiometric()
                            }
                        }
                    )
                }

                if (showBiometricPasswordDialog) {
                    AlertDialog(
                        onDismissRequest = {
                            if (!biometricVerifying) {
                                showBiometricPasswordDialog = false
                            }
                        },
                        icon = { Icon(painter = painterResource(R.drawable.ic_lock), contentDescription = null, modifier = Modifier.size(24.dp), tint = MaterialTheme.colorScheme.primary) },
                        title = { Text("Enable Biometric Lock") },
                        text = {
                            Column {
                                Text(
                                    "Enter your password to enable biometric lock.",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                                Spacer(Modifier.height(12.dp))
                                OutlinedTextField(
                                    value = biometricPassword,
                                    onValueChange = {
                                        biometricPassword = it
                                        biometricPasswordError = null
                                    },
                                    label = { Text("Password") },
                                    singleLine = true,
                                    visualTransformation = if (biometricPasswordVisible) VisualTransformation.None else PasswordVisualTransformation(),
                                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                                    trailingIcon = {
                                        IconButton(onClick = { biometricPasswordVisible = !biometricPasswordVisible }) {
                                            Icon(
                                                imageVector = if (biometricPasswordVisible) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                                                contentDescription = if (biometricPasswordVisible) "Hide password" else "Show password"
                                            )
                                        }
                                    },
                                    isError = biometricPasswordError != null,
                                    supportingText = biometricPasswordError?.let { err -> { Text(err) } },
                                    modifier = Modifier.fillMaxWidth(),
                                    enabled = !biometricVerifying
                                )
                            }
                        },
                        confirmButton = {
                            Button(
                                onClick = {
                                    biometricVerifying = true
                                    viewModel.enableBiometricWithPassword(
                                        password = biometricPassword,
                                        onSuccess = {
                                            biometricVerifying = false
                                            showBiometricPasswordDialog = false
                                            biometricPassword = ""
                                        },
                                        onError = { errorMsg ->
                                            biometricVerifying = false
                                            biometricPasswordError = errorMsg
                                        }
                                    )
                                },
                                enabled = biometricPassword.isNotEmpty() && !biometricVerifying
                            ) {
                                if (biometricVerifying) {
                                    CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                                    Spacer(Modifier.width(8.dp))
                                }
                                Text("Enable")
                            }
                        },
                        dismissButton = {
                            TextButton(
                                onClick = {
                                    showBiometricPasswordDialog = false
                                    biometricPassword = ""
                                },
                                enabled = !biometricVerifying
                            ) {
                                Text("Cancel")
                            }
                        }
                    )
                }
            }

            // ── Display ──────────────────────────────────────────────────
            SettingsCard(title = "Display", icon = Icons.Default.GridView) {
                Text(
                    "Choose how large thumbnails appear in the photo grid.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column {
                        Text("Thumbnail Size")
                        Text(
                            if (viewModel.thumbnailSize == "large") "Large — fewer, bigger thumbnails"
                            else "Normal — more thumbnails per row",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(
                            "Normal",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (viewModel.thumbnailSize == "normal") MaterialTheme.colorScheme.primary
                                    else MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Switch(
                            checked = viewModel.thumbnailSize == "large",
                            onCheckedChange = { viewModel.toggleThumbnailSize() }
                        )
                        Text(
                            "Large",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (viewModel.thumbnailSize == "large") MaterialTheme.colorScheme.primary
                                    else MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }

            // ── Storage Stats ────────────────────────────────────────────
            SettingsCard(title = "Storage", iconPainter = painterResource(R.drawable.ic_floppy_disk)) {
                val stats = viewModel.storageStats
                if (viewModel.storageLoading) {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
                } else if (stats != null) {
                    StorageBar(stats)
                    Spacer(Modifier.height(8.dp))
                    SettingsRow("Photos", "${stats.photoCount} files (${formatBytes(stats.photoBytes)})")
                    SettingsRow("Videos", "${stats.videoCount} files (${formatBytes(stats.videoBytes)})")
                    SettingsRow("Total Used", formatBytes(stats.userTotalBytes))
                    SettingsRow("Disk Free", formatBytes(stats.fsFreeBytes))
                    SettingsRow("Disk Total", formatBytes(stats.fsTotalBytes))
                } else {
                    Text("Unable to load stats", style = MaterialTheme.typography.bodySmall)
                }
                Spacer(Modifier.height(4.dp))
                TextButton(onClick = { viewModel.loadStorageStats() }) {
                    Text("Refresh")
                }
            }

            // ── Backup Folders ───────────────────────────────────────────
            SettingsCard(title = "Backup Folders", iconPainter = painterResource(R.drawable.ic_image)) {
                Text(
                    "Choose which folders on your device to back up. Camera is backed up by default.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(8.dp))
                OutlinedButton(
                    onClick = onBackupFolders,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Icon(painter = painterResource(R.drawable.ic_folder), contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text("Select Backup Folders")
                }
            }

            // ── Free Up Space ────────────────────────────────────────────
            val context = LocalContext.current
            var showFreeUpDialog by remember { mutableStateOf(false) }

            SettingsCard(title = "Free Up Space", icon = Icons.Default.CleaningServices) {
                if (viewModel.freeableCount > 0) {
                    Text(
                        "${viewModel.freeableCount} photos already backed up to the server can be removed from this device, freeing up ${formatBytes(viewModel.freeableBytes)}.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(
                        onClick = { showFreeUpDialog = true },
                        enabled = !viewModel.freeUpLoading,
                        modifier = Modifier.fillMaxWidth(),
                        colors = ButtonDefaults.outlinedButtonColors(
                            contentColor = MaterialTheme.colorScheme.error
                        )
                    ) {
                        if (viewModel.freeUpLoading) {
                            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.width(8.dp))
                        }
                        Icon(Icons.Default.DeleteForever, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(8.dp))
                        Text("Free Up ${formatBytes(viewModel.freeableBytes)}")
                    }
                } else {
                    Text(
                        "No backed-up photos to remove from this device.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            if (showFreeUpDialog) {
                AlertDialog(
                    onDismissRequest = { showFreeUpDialog = false },
                    icon = { Icon(Icons.Default.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
                    title = { Text("Free Up Space?") },
                    text = {
                        Text(
                            "This will permanently delete ${viewModel.freeableCount} photos and videos from your device that have already been backed up to the server.\n\nYou'll free up approximately ${formatBytes(viewModel.freeableBytes)}.\n\nThis cannot be undone. The files will still be available on your server."
                        )
                    },
                    confirmButton = {
                        Button(
                            onClick = {
                                showFreeUpDialog = false
                                viewModel.freeUpSpace(context) { _ -> }
                            },
                            colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error)
                        ) {
                            Text("Delete from Device")
                        }
                    },
                    dismissButton = {
                        TextButton(onClick = { showFreeUpDialog = false }) {
                            Text("Cancel")
                        }
                    }
                )
            }


            // ── Diagnostic Logging ───────────────────────────────────────
            SettingsCard(title = "Troubleshooting", icon = Icons.Default.BugReport) {
                Text(
                    "When enabled, the app sends detailed backup logs to the server to help diagnose upload issues.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Diagnostic Logging")
                    Switch(
                        checked = viewModel.diagnosticLogging,
                        onCheckedChange = { viewModel.toggleDiagnosticLogging() }
                    )
                }
            }


            // ── Active Server ────────────────────────────────────────────
            SettingsCard(title = "Active Server", icon = Icons.Default.Dns) {
                SettingsRow("URL", viewModel.serverUrl)
                SettingsRow("Status", "Connected")
            }

            // ── Scan for New Files (admin) ───────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "Scan for New Files", icon = Icons.Default.Radar) {
                    Text(
                        "Scan the server storage directory for files that were placed there manually and register them in the database.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(
                        onClick = { viewModel.scanForNewFiles() },
                        enabled = !viewModel.scanning,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        if (viewModel.scanning) {
                            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.width(8.dp))
                        }
                        Icon(Icons.Default.Radar, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(8.dp))
                        Text("Scan Now")
                    }
                    viewModel.scanResult?.let { result ->
                        Spacer(Modifier.height(8.dp))
                        Text(result, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
                    }
                }
            }







            // ── SSL/TLS (admin) ──────────────────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "SSL / TLS", iconPainter = painterResource(R.drawable.ic_lock)) {
                    if (viewModel.sslLoading) {
                        CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                    } else {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(
                                if (viewModel.sslEnabled) Icons.Default.CheckCircle else Icons.Default.Warning,
                                contentDescription = null,
                                modifier = Modifier.size(18.dp),
                                tint = if (viewModel.sslEnabled) Color(0xFF22C55E) else Color(0xFFF59E0B)
                            )
                            Spacer(Modifier.width(8.dp))
                            Text(
                                if (viewModel.sslEnabled) "SSL/TLS is enabled"
                                else "SSL/TLS is not enabled — connections are unencrypted",
                                style = MaterialTheme.typography.bodySmall,
                                color = if (viewModel.sslEnabled) Color(0xFF22C55E) else Color(0xFFF59E0B)
                            )
                        }
                    }
                }
            }

            // ── Manage Users (admin) ─────────────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "Manage Users", icon = Icons.Default.Group) {
                    Text(
                        "User management is available in the web interface.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Open ${viewModel.serverUrl} in a browser to manage users.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.primary
                    )
                }
            }


            // ── About ────────────────────────────────────────────────────
            SettingsCard(title = "About", icon = Icons.Default.Info) {
                Text("Simple Photos", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                Text("Version 0.6.9", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(4.dp))
                Text(
                    "A self-hosted photo storage solution with optional end-to-end encryption.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            // ── Credits & Links ──────────────────────────────────────────
            SettingsCard(title = "Credits & Links", iconPainter = painterResource(R.drawable.ic_star)) {
                val uriHandler = androidx.compose.ui.platform.LocalUriHandler.current
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(painter = painterResource(R.drawable.ic_star), contentDescription = null, modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(8.dp))
                    Column {
                        Text("Icons", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
                        Text(
                            "Custom icons by Angus_87 on Flaticon",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.clickable { uriHandler.openUri("https://www.flaticon.com/authors/angus-87") }
                        )
                    }
                }
                Spacer(Modifier.height(12.dp))
                HorizontalDivider()
                Spacer(Modifier.height(12.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(painter = painterResource(R.drawable.ic_shared), contentDescription = null, modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(8.dp))
                    Column {
                        Text("Source Code", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
                        Text(
                            "github.com/wulfic/simple-photos",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.clickable { uriHandler.openUri("https://github.com/wulfic/simple-photos") }
                        )
                    }
                }
            }

            // ── Messages ─────────────────────────────────────────────────
            viewModel.error?.let { err ->
                Text(err, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
            viewModel.message?.let { msg ->
                Text(msg, color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.bodySmall)
            }

            // ── Logout ───────────────────────────────────────────────────
            Button(
                onClick = { viewModel.logout(onLogout) },
                enabled = !viewModel.loading,
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error)
            ) {
                if (viewModel.loading) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onError)
                } else {
                    Text("Log Out")
                }
            }

            Spacer(Modifier.height(32.dp))
        }
    }
}
