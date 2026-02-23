package com.simplephotos.ui.screens.auth

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.repository.AuthRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class LoginViewModel @Inject constructor(
    private val authRepo: AuthRepository
) : ViewModel() {
    var username by mutableStateOf("")
    var password by mutableStateOf("")
    var totpCode by mutableStateOf("")
    var totpSessionToken by mutableStateOf<String?>(null)
    var loading by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)

    fun login(onSuccess: () -> Unit) {
        viewModelScope.launch {
            loading = true
            error = null
            try {
                if (totpSessionToken != null) {
                    // Pass username + password so key derivation happens after TOTP
                    authRepo.loginTotp(
                        totpSessionToken!!, totpCode.ifBlank { null }, null,
                        password, username
                    )
                    onSuccess()
                } else {
                    val res = authRepo.login(username, password)
                    if (res.requiresTotp == true && res.totpSessionToken != null) {
                        totpSessionToken = res.totpSessionToken
                    } else {
                        onSuccess()
                    }
                }
            } catch (e: Exception) {
                error = e.message ?: "Login failed"
            } finally {
                loading = false
            }
        }
    }
}

@Composable
fun LoginScreen(
    onLoginSuccess: () -> Unit,
    onNavigateToRegister: () -> Unit,
    viewModel: LoginViewModel = hiltViewModel()
) {
    var showPassword by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text("Simple Photos", style = MaterialTheme.typography.headlineMedium)
        Spacer(Modifier.height(4.dp))
        Text(
            "End-to-end encrypted photo library",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Spacer(Modifier.height(32.dp))

        if (viewModel.totpSessionToken == null) {
            OutlinedTextField(
                value = viewModel.username,
                onValueChange = { viewModel.username = it },
                label = { Text("Username") },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true
            )
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = viewModel.password,
                onValueChange = { viewModel.password = it },
                label = { Text("Password") },
                visualTransformation = if (showPassword) VisualTransformation.None else PasswordVisualTransformation(),
                trailingIcon = {
                    IconButton(onClick = { showPassword = !showPassword }) {
                        Icon(
                            if (showPassword) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                            contentDescription = if (showPassword) "Hide password" else "Show password"
                        )
                    }
                },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true
            )
        } else {
            Text("Enter your two-factor code", style = MaterialTheme.typography.bodyMedium)
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = viewModel.totpCode,
                onValueChange = { viewModel.totpCode = it },
                label = { Text("6-digit code or backup code") },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true
            )
        }

        viewModel.error?.let { err ->
            Spacer(Modifier.height(8.dp))
            Text(err, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }

        Spacer(Modifier.height(16.dp))
        Button(
            onClick = { viewModel.login(onLoginSuccess) },
            enabled = !viewModel.loading,
            modifier = Modifier.fillMaxWidth()
        ) {
            if (viewModel.loading) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(8.dp))
                Text("Signing in...")
            } else {
                Text(if (viewModel.totpSessionToken != null) "Verify" else "Sign In")
            }
        }

        if (viewModel.totpSessionToken == null) {
            Spacer(Modifier.height(8.dp))
            TextButton(onClick = onNavigateToRegister) {
                Text("Don't have an account? Register")
            }
        }
    }
}
