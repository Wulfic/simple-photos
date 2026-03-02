package com.simplephotos

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
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
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.google.accompanist.permissions.ExperimentalPermissionsApi
import com.google.accompanist.permissions.rememberMultiplePermissionsState
import com.simplephotos.data.repository.BackupFolderRepository
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
 * Composable gate that requests **full** media access before showing [content].
 *
 * On Android 14+ (API 34, targetSdk 34), the system always offers the user
 * a "Select photos and videos" option alongside "Allow all".  If the user
 * chooses the former, READ_MEDIA_IMAGES is *not* granted — the app only sees
 * the manually-selected items, which breaks folder discovery for backup.
 *
 * IMPORTANT: We use ContextCompat.checkSelfPermission() directly for the
 * full/partial access decision — NOT Accompanist's allPermissionsGranted.
 * Accompanist 0.34.0 has a known issue on Android 14+ where it can report
 * allPermissionsGranted=true when only partial ("Select photos") access was
 * granted, because the runtime result callback receives PERMISSION_GRANTED
 * for compat reasons.  Direct system checks are the ground truth.
 *
 * Accompanist is still used solely to launch the permission request dialog.
 */
@OptIn(ExperimentalPermissionsApi::class)
@Composable
private fun PermissionGate(content: @Composable () -> Unit) {
    val context = LocalContext.current

    // ── Counter incremented on ON_RESUME to force permission re-check ──
    // When the user returns from system Settings, this triggers recomposition
    // and re-evaluation of the actual permission state.
    var permCheckTrigger by remember { mutableIntStateOf(0) }

    // Direct system permission checks (NOT Accompanist)
    val actualFullAccess = remember(permCheckTrigger) {
        BackupFolderRepository.hasFullMediaAccess(context)
    }

    val actualPartialAccess = remember(permCheckTrigger, actualFullAccess) {
        BackupFolderRepository.hasPartialMediaAccess(context)
    }

    val hasAnyAccess = actualFullAccess || actualPartialAccess

    // Re-check permissions every time the activity resumes (e.g. returning from Settings)
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                permCheckTrigger++
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    // Accompanist is used ONLY for launching the system permission dialog.
    // We do NOT trust its allPermissionsGranted for the gate decision.
    val mediaPermissions = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        listOf(
            Manifest.permission.READ_MEDIA_IMAGES,
            Manifest.permission.READ_MEDIA_VIDEO
        )
    } else {
        listOf(Manifest.permission.READ_EXTERNAL_STORAGE)
    }
    val permissionsState = rememberMultiplePermissionsState(mediaPermissions) {
        // Callback fires after the system permission dialog is dismissed.
        // Force an immediate re-check so we don't depend on Accompanist's state.
        permCheckTrigger++
    }

    // Log permission state for diagnostics
    LaunchedEffect(permCheckTrigger) {
        val readImages = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_IMAGES) == PackageManager.PERMISSION_GRANTED
        } else null
        val readVideo = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_VIDEO) == PackageManager.PERMISSION_GRANTED
        } else null
        val visualUserSelected = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED) == PackageManager.PERMISSION_GRANTED
        } else null
        android.util.Log.i("PermissionGate",
            "Permission check #$permCheckTrigger: " +
            "actualFullAccess=$actualFullAccess " +
            "actualPartialAccess=$actualPartialAccess " +
            "accompanistAllGranted=${permissionsState.allPermissionsGranted} " +
            "READ_MEDIA_IMAGES=$readImages " +
            "READ_MEDIA_VIDEO=$readVideo " +
            "VISUAL_USER_SELECTED=$visualUserSelected " +
            "API=${Build.VERSION.SDK_INT} " +
            "shouldShowRationale=${permissionsState.shouldShowRationale}"
        )
    }

    when {
        actualFullAccess -> {
            content()
        }
        actualPartialAccess -> {
            // User chose "Select photos and videos" — partial access.
            // We can't re-request via the runtime dialog; must direct to Settings.
            PartialAccessScreen(context)
        }
        else -> {
            // No access at all — request for the first time.
            // Use a flag to only auto-launch the dialog once.
            var hasLaunched by remember { mutableStateOf(false) }
            LaunchedEffect(Unit) {
                if (!hasLaunched) {
                    hasLaunched = true
                    permissionsState.launchMultiplePermissionRequest()
                }
            }

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
                        "Simple Photos needs full access to your photos and videos to discover all folders, back up, and manage your media library.\n\nWhen prompted, please choose \"Allow all\".",
                        fontSize = 14.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center
                    )

                    Spacer(Modifier.height(32.dp))

                    // After the dialog has been dismissed without granting,
                    // check if we can re-request or need to go to Settings.
                    if (hasLaunched && !hasAnyAccess) {
                        if (permissionsState.shouldShowRationale) {
                            Button(
                                onClick = { permissionsState.launchMultiplePermissionRequest() },
                                modifier = Modifier.fillMaxWidth(),
                                shape = RoundedCornerShape(12.dp)
                            ) {
                                Text("Grant Full Access", modifier = Modifier.padding(vertical = 4.dp))
                            }
                        } else {
                            Text(
                                "Permissions were denied. Please grant full media access in app settings.",
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
                    }
                }
            }
        }
    }
}

/**
 * Shown when the user has partial photo access ("Select photos and videos").
 * Directs them to system Settings to upgrade to "Allow all".
 */
@Composable
private fun PartialAccessScreen(context: android.content.Context) {
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
                "Full Access Required",
                fontSize = 22.sp,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.onBackground
            )

            Spacer(Modifier.height(12.dp))

            Text(
                "You\u2019ve granted partial photo access (\u201CSelect photos\u201D). " +
                    "Simple Photos is a backup app and needs access to all photos " +
                    "and videos to discover your folders.\n\n" +
                    "Please open Settings \u2192 Permissions \u2192 Photos and Videos \u2192 " +
                    "and select \u201CAllow all\u201D.",
                fontSize = 14.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center
            )

            Spacer(Modifier.height(32.dp))

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

            Spacer(Modifier.height(8.dp))

            Text(
                "After changing the permission, return here and the app will continue automatically.",
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center
            )
        }
    }
}
