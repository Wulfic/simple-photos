package com.simplephotos

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.fragment.app.FragmentActivity
import com.google.accompanist.permissions.ExperimentalPermissionsApi
import com.google.accompanist.permissions.rememberMultiplePermissionsState
import com.simplephotos.ui.navigation.NavGraph
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_BIOMETRIC_ENABLED
import com.simplephotos.ui.theme.SimplePhotosTheme
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import javax.inject.Inject

@AndroidEntryPoint
class MainActivity : FragmentActivity() {

    @Inject
    lateinit var dataStore: DataStore<Preferences>

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        // Check if biometric is enabled in settings
        val biometricEnabled = runBlocking {
            val prefs = dataStore.data.first()
            prefs[KEY_BIOMETRIC_ENABLED] ?: false
        }

        val biometricAvailable = BiometricManager.from(this)
            .canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.BIOMETRIC_WEAK) == BiometricManager.BIOMETRIC_SUCCESS

        setContent {
            SimplePhotosTheme {
                var authenticated by remember { mutableStateOf(!biometricEnabled || !biometricAvailable) }
                var authFailed by remember { mutableStateOf(false) }

                if (!authenticated) {
                    // Show biometric prompt on launch
                    LaunchedEffect(Unit) {
                        showBiometricPrompt(
                            onSuccess = { authenticated = true },
                            onFail = { authFailed = true }
                        )
                    }
                    // Lock screen while waiting
                    Box(
                        modifier = Modifier
                            .fillMaxSize()
                            .background(MaterialTheme.colorScheme.background),
                        contentAlignment = Alignment.Center
                    ) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(
                                "Simple Photos",
                                fontSize = 24.sp,
                                fontWeight = FontWeight.Bold,
                                color = MaterialTheme.colorScheme.onBackground
                            )
                            Spacer(Modifier.height(8.dp))
                            Text(
                                if (authFailed) "Authentication failed" else "Unlock to continue",
                                fontSize = 14.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            if (authFailed) {
                                Spacer(Modifier.height(16.dp))
                                Button(onClick = {
                                    authFailed = false
                                    showBiometricPrompt(
                                        onSuccess = { authenticated = true },
                                        onFail = { authFailed = true }
                                    )
                                }) {
                                    Text("Try Again")
                                }
                            }
                        }
                    }
                } else {
                    // Gate on media permissions before entering the app
                    PermissionGate {
                        NavGraph()
                    }
                }
            }
        }
    }

    private fun showBiometricPrompt(onSuccess: () -> Unit, onFail: () -> Unit) {
        val executor = ContextCompat.getMainExecutor(this)
        val prompt = BiometricPrompt(this, executor, object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                onSuccess()
            }
            override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                if (errorCode != BiometricPrompt.ERROR_USER_CANCELED &&
                    errorCode != BiometricPrompt.ERROR_NEGATIVE_BUTTON &&
                    errorCode != BiometricPrompt.ERROR_CANCELED) {
                    onFail()
                }
            }
            override fun onAuthenticationFailed() {
                // Individual attempt failed — prompt stays open for retry
            }
        })
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle("Unlock Simple Photos")
            .setSubtitle("Verify your identity")
            .setNegativeButtonText("Cancel")
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.BIOMETRIC_WEAK)
            .build()
        prompt.authenticate(info)
    }
}

/**
 * Composable gate that requests required media permissions before showing [content].
 * On API 33+ requests READ_MEDIA_IMAGES and READ_MEDIA_VIDEO;
 * on older versions requests READ_EXTERNAL_STORAGE.
 * If the user permanently denies, offers a button to open app settings.
 */
@OptIn(ExperimentalPermissionsApi::class)
@Composable
private fun PermissionGate(content: @Composable () -> Unit) {
    val mediaPermissions = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        listOf(
            Manifest.permission.READ_MEDIA_IMAGES,
            Manifest.permission.READ_MEDIA_VIDEO
        )
    } else {
        listOf(Manifest.permission.READ_EXTERNAL_STORAGE)
    }
    val permissionsState = rememberMultiplePermissionsState(mediaPermissions)

    if (permissionsState.allPermissionsGranted) {
        content()
    } else {
        // Request permissions automatically on first composition
        LaunchedEffect(Unit) {
            permissionsState.launchMultiplePermissionRequest()
        }

        val context = LocalContext.current

        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background)
                .systemBarsPadding(),
            contentAlignment = Alignment.Center
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 32.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Icon(
                    painter = painterResource(R.drawable.ic_image),
                    contentDescription = null,
                    modifier = Modifier.size(64.dp),
                    tint = MaterialTheme.colorScheme.primary
                )

                Spacer(Modifier.height(24.dp))

                Text(
                    "Media Access Required",
                    fontSize = 22.sp,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.onBackground
                )

                Spacer(Modifier.height(12.dp))

                Text(
                    "Simple Photos needs access to your photos and videos to browse, back up, and manage your media library.",
                    fontSize = 14.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center
                )

                Spacer(Modifier.height(32.dp))

                // If user has denied once (shouldShowRationale = true) or permanently denied
                if (permissionsState.shouldShowRationale) {
                    // System will still show the dialog — offer retry
                    Button(
                        onClick = { permissionsState.launchMultiplePermissionRequest() },
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(12.dp)
                    ) {
                        Text("Grant Permissions", modifier = Modifier.padding(vertical = 4.dp))
                    }
                } else {
                    // Permanently denied — direct to app settings
                    Text(
                        "Permissions were denied. Please enable them in app settings.",
                        fontSize = 13.sp,
                        color = MaterialTheme.colorScheme.error,
                        textAlign = TextAlign.Center
                    )
                    Spacer(Modifier.height(16.dp))
                    Button(
                        onClick = {
                            val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                                data = Uri.fromParts("package", context.packageName, null)
                            }
                            context.startActivity(intent)
                        },
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(12.dp)
                    ) {
                        Text("Open Settings", modifier = Modifier.padding(vertical = 4.dp))
                    }
                }

                Spacer(Modifier.height(12.dp))

                // Allow skipping — some features work without media access (e.g. cloud-only browsing)
                TextButton(onClick = { /* skip handled by re-checking on resume */ }) {
                    // We don't actually skip — we just keep the gate up.
                    // But we could add a skip mechanism if needed.
                }
            }
        }
    }
}
