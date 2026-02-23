package com.simplephotos.ui.screens.twofactor

import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import com.simplephotos.data.repository.AuthRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * ViewModel managing the two-factor authentication setup flow.
 *
 * State machine: Idle → Loading → ShowQR → Confirming → Done
 */
@HiltViewModel
class TwoFactorSetupViewModel @Inject constructor(
    private val authRepo: AuthRepository
) : ViewModel() {

    enum class Step { IDLE, LOADING, SHOW_QR, CONFIRMING, DONE, ERROR }

    var step by mutableStateOf(Step.IDLE)
    var otpauthUri by mutableStateOf("")
    var backupCodes by mutableStateOf<List<String>>(emptyList())
    var confirmCode by mutableStateOf("")
    var error by mutableStateOf<String?>(null)

    fun beginSetup() {
        step = Step.LOADING
        error = null
        viewModelScope.launch {
            try {
                val resp = authRepo.setup2fa()
                otpauthUri = resp.otpauthUri
                backupCodes = resp.backupCodes
                step = Step.SHOW_QR
            } catch (e: Exception) {
                error = e.message ?: "Failed to initiate 2FA setup"
                step = Step.ERROR
            }
        }
    }

    fun confirm() {
        if (confirmCode.length != 6) {
            error = "Enter a 6-digit code from your authenticator app"
            return
        }
        step = Step.CONFIRMING
        error = null
        viewModelScope.launch {
            try {
                authRepo.confirm2fa(confirmCode)
                step = Step.DONE
            } catch (e: Exception) {
                error = e.message ?: "Verification failed — check the code and try again"
                step = Step.SHOW_QR
            }
        }
    }
}

/**
 * Generates a Bitmap QR code from the given text using ZXing.
 * Returns null on failure so the caller can degrade gracefully.
 */
private fun generateQrBitmap(text: String, size: Int = 512): Bitmap? {
    return try {
        val writer = QRCodeWriter()
        val matrix = writer.encode(text, BarcodeFormat.QR_CODE, size, size)
        val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.RGB_565)
        for (x in 0 until size) {
            for (y in 0 until size) {
                bmp.setPixel(x, y, if (matrix[x, y]) Color.BLACK else Color.WHITE)
            }
        }
        bmp
    } catch (_: Exception) {
        null
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TwoFactorSetupScreen(
    onBack: () -> Unit,
    viewModel: TwoFactorSetupViewModel = hiltViewModel()
) {
    val clipboardManager = LocalClipboardManager.current

    // Kick off setup on first composition
    LaunchedEffect(Unit) {
        if (viewModel.step == TwoFactorSetupViewModel.Step.IDLE) {
            viewModel.beginSetup()
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Two-Factor Authentication") },
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
                .padding(16.dp)
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            when (viewModel.step) {
                TwoFactorSetupViewModel.Step.IDLE,
                TwoFactorSetupViewModel.Step.LOADING -> {
                    Spacer(Modifier.height(48.dp))
                    CircularProgressIndicator()
                    Spacer(Modifier.height(16.dp))
                    Text("Setting up two-factor authentication…")
                }

                TwoFactorSetupViewModel.Step.SHOW_QR,
                TwoFactorSetupViewModel.Step.CONFIRMING -> {
                    // QR Code
                    Text(
                        "Scan this QR code with your authenticator app:",
                        style = MaterialTheme.typography.bodyLarge,
                        textAlign = TextAlign.Center
                    )
                    Spacer(Modifier.height(16.dp))

                    val qrBitmap = remember(viewModel.otpauthUri) {
                        generateQrBitmap(viewModel.otpauthUri)
                    }
                    if (qrBitmap != null) {
                        Image(
                            bitmap = qrBitmap.asImageBitmap(),
                            contentDescription = "2FA QR Code",
                            modifier = Modifier.size(240.dp)
                        )
                    } else {
                        // Fallback: show URI text
                        Text(
                            viewModel.otpauthUri,
                            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                            modifier = Modifier.padding(8.dp)
                        )
                    }

                    Spacer(Modifier.height(24.dp))

                    // Backup codes
                    Card(modifier = Modifier.fillMaxWidth()) {
                        Column(modifier = Modifier.padding(16.dp)) {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Text("Backup Codes", style = MaterialTheme.typography.titleSmall)
                                IconButton(onClick = {
                                    clipboardManager.setText(
                                        AnnotatedString(viewModel.backupCodes.joinToString("\n"))
                                    )
                                }) {
                                    Icon(Icons.Default.ContentCopy, "Copy codes")
                                }
                            }
                            Text(
                                "Save these codes in a safe place. Each can be used once if you lose access to your authenticator app.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            Spacer(Modifier.height(8.dp))
                            viewModel.backupCodes.forEach { code ->
                                Text(
                                    code,
                                    style = MaterialTheme.typography.bodyMedium.copy(
                                        fontFamily = FontFamily.Monospace
                                    ),
                                    modifier = Modifier.padding(vertical = 2.dp)
                                )
                            }
                        }
                    }

                    Spacer(Modifier.height(24.dp))

                    // Confirmation code entry
                    OutlinedTextField(
                        value = viewModel.confirmCode,
                        onValueChange = { input ->
                            // Only allow digits, max 6
                            viewModel.confirmCode = input.filter { it.isDigit() }.take(6)
                        },
                        label = { Text("6-digit code") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth()
                    )

                    viewModel.error?.let { err ->
                        Spacer(Modifier.height(8.dp))
                        Text(err, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                    }

                    Spacer(Modifier.height(16.dp))

                    Button(
                        onClick = { viewModel.confirm() },
                        enabled = viewModel.step != TwoFactorSetupViewModel.Step.CONFIRMING,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        if (viewModel.step == TwoFactorSetupViewModel.Step.CONFIRMING) {
                            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                        } else {
                            Text("Verify & Enable")
                        }
                    }
                }

                TwoFactorSetupViewModel.Step.DONE -> {
                    Spacer(Modifier.height(48.dp))
                    Text(
                        "Two-factor authentication enabled!",
                        style = MaterialTheme.typography.headlineSmall,
                        textAlign = TextAlign.Center
                    )
                    Spacer(Modifier.height(24.dp))
                    Button(onClick = onBack, modifier = Modifier.fillMaxWidth()) {
                        Text("Done")
                    }
                }

                TwoFactorSetupViewModel.Step.ERROR -> {
                    Spacer(Modifier.height(48.dp))
                    Text(
                        viewModel.error ?: "Unknown error",
                        color = MaterialTheme.colorScheme.error,
                        textAlign = TextAlign.Center
                    )
                    Spacer(Modifier.height(16.dp))
                    OutlinedButton(onClick = { viewModel.beginSetup() }) {
                        Text("Retry")
                    }
                }
            }
        }
    }
}
