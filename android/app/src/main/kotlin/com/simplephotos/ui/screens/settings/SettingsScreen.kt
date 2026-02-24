package com.simplephotos.ui.screens.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.ui.theme.ThemeState
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val authRepo: AuthRepository,
    private val api: ApiService,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    var serverUrl by mutableStateOf("")
    var username by mutableStateOf("")
    var loading by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var message by mutableStateOf<String?>(null)

    // Storage stats
    var storageStats by mutableStateOf<StorageStatsResponse?>(null)
    var storageLoading by mutableStateOf(false)

    // Encryption mode
    var encryptionMode by mutableStateOf("plain")

    // Admin: user list
    var users by mutableStateOf<List<AdminUser>>(emptyList())
    var usersLoading by mutableStateOf(false)
    var isAdmin by mutableStateOf(false)

    // Scan
    var scanning by mutableStateOf(false)
    var scanResult by mutableStateOf<String?>(null)

    init {
        viewModelScope.launch {
            val prefs = dataStore.data.first()
            serverUrl = prefs[KEY_SERVER_URL] ?: ""
            username = prefs[KEY_USERNAME] ?: ""
        }
        loadStorageStats()
        loadEncryptionSettings()
    }

    fun loadStorageStats() {
        viewModelScope.launch {
            storageLoading = true
            try {
                storageStats = withContext(Dispatchers.IO) { api.getStorageStats() }
            } catch (_: Exception) {}
            storageLoading = false
        }
    }

    private fun loadEncryptionSettings() {
        viewModelScope.launch {
            try {
                val settings = withContext(Dispatchers.IO) { api.getEncryptionSettings() }
                encryptionMode = settings.encryptionMode
            } catch (_: Exception) {}
        }
    }

    fun loadUsers() {
        viewModelScope.launch {
            usersLoading = true
            try {
                users = withContext(Dispatchers.IO) { api.listUsers() }
                isAdmin = true
            } catch (_: Exception) {
                isAdmin = false
            }
            usersLoading = false
        }
    }

    fun scanFiles() {
        viewModelScope.launch {
            scanning = true
            scanResult = null
            try {
                val result = withContext(Dispatchers.IO) { api.scanAndRegister() }
                scanResult = "Found and registered ${result.registered} files"
            } catch (e: Exception) {
                scanResult = "Scan failed: ${e.message}"
            }
            scanning = false
        }
    }

    fun changePassword(currentPassword: String, newPassword: String, onSuccess: () -> Unit) {
        viewModelScope.launch {
            loading = true
            error = null
            try {
                val response = withContext(Dispatchers.IO) {
                    api.changePassword(ChangePasswordRequest(currentPassword, newPassword))
                }
                if (response.isSuccessful) {
                    message = "Password changed successfully"
                    onSuccess()
                } else {
                    error = "Failed to change password (${response.code()})"
                }
            } catch (e: Exception) {
                error = "Failed to change password: ${e.message}"
            }
            loading = false
        }
    }

    fun createUser(username: String, password: String, role: String, onSuccess: () -> Unit) {
        viewModelScope.launch {
            loading = true
            error = null
            try {
                withContext(Dispatchers.IO) {
                    api.createUser(CreateUserRequest(username, password, role))
                }
                message = "User '$username' created"
                loadUsers()
                onSuccess()
            } catch (e: Exception) {
                error = "Failed to create user: ${e.message}"
            }
            loading = false
        }
    }

    fun deleteUser(userId: String) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { api.deleteUser(userId) }
                message = "User deleted"
                loadUsers()
            } catch (e: Exception) {
                error = "Failed: ${e.message}"
            }
        }
    }

    fun updateUserRole(userId: String, role: String) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { api.updateUserRole(userId, UpdateRoleRequest(role)) }
                loadUsers()
            } catch (e: Exception) {
                error = "Failed: ${e.message}"
            }
        }
    }

    fun resetUserPassword(userId: String, newPassword: String) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { api.resetUserPassword(userId, ResetPasswordRequest(newPassword)) }
                message = "Password reset"
            } catch (e: Exception) {
                error = "Failed: ${e.message}"
            }
        }
    }

    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            loading = true
            try {
                authRepo.logout()
                onLoggedOut()
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }
}

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

    // Load admin users on first compose
    LaunchedEffect(Unit) {
        viewModel.loadUsers()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.Default.ArrowBack, contentDescription = "Back")
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
                SettingsRow("Mode", viewModel.encryptionMode.replaceFirstChar { it.uppercase() })
            }

            // ── Storage Stats ────────────────────────────────────────────
            SettingsCard(title = "Storage", icon = Icons.Default.Storage) {
                val stats = viewModel.storageStats
                if (viewModel.storageLoading) {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
                } else if (stats != null) {
                    StorageBar(stats)
                    Spacer(Modifier.height(8.dp))
                    SettingsRow("Photos", "${stats.photoCount + stats.plainCount} files (${formatBytes(stats.photoBytes + stats.plainBytes)})")
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

            // ── Scan Files (admin only) ─────────────────────────────────
            if (viewModel.isAdmin) {
                SettingsCard(title = "Scan Files", icon = Icons.Default.FolderOpen) {
                    Text(
                        "Scan the server storage directory for new files and register them.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(
                        onClick = { viewModel.scanFiles() },
                        enabled = !viewModel.scanning,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        if (viewModel.scanning) {
                            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.width(8.dp))
                        }
                        Text("Scan & Register")
                    }
                    viewModel.scanResult?.let {
                        Spacer(Modifier.height(4.dp))
                        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
                    }
                }
            }

            // ── Appearance ───────────────────────────────────────────────
            SettingsCard(title = "Appearance", icon = Icons.Default.Palette) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Dark Mode")
                    Switch(
                        checked = ThemeState.mode == "dark",
                        onCheckedChange = { ThemeState.toggle(viewModel.dataStore) }
                    )
                }
            }

            // ── Change Password ──────────────────────────────────────────
            ChangePasswordSection(viewModel)

            // ── Security / 2FA ───────────────────────────────────────────
            SettingsCard(title = "Security", icon = Icons.Default.Security) {
                OutlinedButton(onClick = onSetup2fa, modifier = Modifier.fillMaxWidth()) {
                    Text("Two-Factor Authentication")
                }
            }

            // ── Manage Users (admin only) ────────────────────────────────
            if (viewModel.isAdmin) {
                ManageUsersSection(viewModel)
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

// ── Reusable settings card ──────────────────────────────────────────────────

@Composable
private fun SettingsCard(
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
private fun SettingsRow(label: String, value: String) {
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
private fun StorageBar(stats: StorageStatsResponse) {
    val total = stats.fsTotalBytes.toFloat().coerceAtLeast(1f)
    val photoFraction = (stats.photoBytes + stats.plainBytes) / total
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
private fun LegendDot(color: Color, label: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(modifier = Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(color))
        Spacer(Modifier.width(4.dp))
        Text(label, style = MaterialTheme.typography.labelSmall)
    }
}

// ── Change Password ─────────────────────────────────────────────────────────

@Composable
private fun ChangePasswordSection(viewModel: SettingsViewModel) {
    var expanded by remember { mutableStateOf(false) }
    var currentPassword by remember { mutableStateOf("") }
    var newPassword by remember { mutableStateOf("") }
    var confirmPassword by remember { mutableStateOf("") }
    var showPasswords by remember { mutableStateOf(false) }

    SettingsCard(title = "Change Password", icon = Icons.Default.Lock) {
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
}

private fun passwordStrength(password: String): Triple<Float, Color, String> {
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

// ── Manage Users (Admin) ────────────────────────────────────────────────────

@Composable
private fun ManageUsersSection(viewModel: SettingsViewModel) {
    var showCreateDialog by remember { mutableStateOf(false) }

    SettingsCard(title = "Manage Users", icon = Icons.Default.People) {
        if (viewModel.usersLoading) {
            CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
        } else {
            viewModel.users.forEach { user ->
                UserRow(user, viewModel)
            }
        }
        Spacer(Modifier.height(8.dp))
        OutlinedButton(onClick = { showCreateDialog = true }, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Default.PersonAdd, contentDescription = null, modifier = Modifier.size(16.dp))
            Spacer(Modifier.width(8.dp))
            Text("Add User")
        }
    }

    if (showCreateDialog) {
        CreateUserDialog(
            onDismiss = { showCreateDialog = false },
            onCreateUser = { username, password, role ->
                viewModel.createUser(username, password, role) {
                    showCreateDialog = false
                }
            },
            isLoading = viewModel.loading
        )
    }
}

@Composable
private fun UserRow(user: AdminUser, viewModel: SettingsViewModel) {
    var showResetPassword by remember { mutableStateOf(false) }
    var resetPassword by remember { mutableStateOf("") }

    Card(
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column {
                    Text(user.username, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
                    Text(
                        buildString {
                            append(user.role)
                            if (user.totpEnabled) append(" · 2FA enabled")
                        },
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Row {
                    // Toggle role
                    IconButton(onClick = {
                        val newRole = if (user.role == "admin") "user" else "admin"
                        viewModel.updateUserRole(user.id, newRole)
                    }, modifier = Modifier.size(32.dp)) {
                        Icon(
                            if (user.role == "admin") Icons.Default.AdminPanelSettings else Icons.Default.Person,
                            contentDescription = "Toggle role",
                            modifier = Modifier.size(16.dp)
                        )
                    }
                    // Delete
                    IconButton(onClick = { viewModel.deleteUser(user.id) }, modifier = Modifier.size(32.dp)) {
                        Icon(Icons.Default.Delete, contentDescription = "Delete", modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.error)
                    }
                }
            }

            if (showResetPassword) {
                Spacer(Modifier.height(8.dp))
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(
                        value = resetPassword,
                        onValueChange = { resetPassword = it },
                        label = { Text("New password") },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation()
                    )
                    Button(
                        onClick = {
                            viewModel.resetUserPassword(user.id, resetPassword)
                            showResetPassword = false
                            resetPassword = ""
                        },
                        enabled = resetPassword.isNotEmpty()
                    ) { Text("Set") }
                }
            }

            TextButton(onClick = { showResetPassword = !showResetPassword }) {
                Text(if (showResetPassword) "Cancel" else "Reset Password", fontSize = 12.sp)
            }
        }
    }
}

@Composable
private fun CreateUserDialog(
    onDismiss: () -> Unit,
    onCreateUser: (String, String, String) -> Unit,
    isLoading: Boolean
) {
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var role by remember { mutableStateOf("user") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Create User") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(value = username, onValueChange = { username = it }, label = { Text("Username") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(value = password, onValueChange = { password = it }, label = { Text("Password") }, visualTransformation = PasswordVisualTransformation(), singleLine = true, modifier = Modifier.fillMaxWidth())
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Admin", style = MaterialTheme.typography.bodyMedium)
                    Spacer(Modifier.width(8.dp))
                    Switch(checked = role == "admin", onCheckedChange = { role = if (it) "admin" else "user" })
                }
            }
        },
        confirmButton = {
            Button(
                onClick = { onCreateUser(username, password, role) },
                enabled = username.isNotEmpty() && password.isNotEmpty() && !isLoading
            ) { Text("Create") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        }
    )
}

// ── Utilities ────────────────────────────────────────────────────────────────

private fun formatBytes(bytes: Long): String {
    if (bytes <= 0) return "0 B"
    val units = arrayOf("B", "KB", "MB", "GB", "TB")
    val digitGroups = (Math.log10(bytes.toDouble()) / Math.log10(1024.0)).toInt().coerceAtMost(units.lastIndex)
    return "%.1f %s".format(bytes / Math.pow(1024.0, digitGroups.toDouble()), units[digitGroups])
}
