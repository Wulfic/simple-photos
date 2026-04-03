package com.simplephotos.ui.screens.securegallery

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

@Composable
fun SecureGalleryScreen(
    onBack: () -> Unit,
    onPhotoClick: (String) -> Unit,
    viewModel: SecureGalleryViewModel = hiltViewModel()
) {
    val context = LocalContext.current
    val activity = context as? FragmentActivity

    // ── Biometric + secure credential storage ─────────
    val biometricManager = remember { BiometricManager.from(context) }
    val canUseBiometric = remember {
        biometricManager.canAuthenticate(
            BiometricManager.Authenticators.BIOMETRIC_STRONG or
                BiometricManager.Authenticators.BIOMETRIC_WEAK
        ) == BiometricManager.BIOMETRIC_SUCCESS
    }

    val encryptedPrefs = remember {
        try {
            val masterKey = MasterKey.Builder(context)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            EncryptedSharedPreferences.create(
                context,
                "secure_gallery_prefs",
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
            )
        } catch (_: Exception) { null }
    }

    val hasStoredPassword = remember {
        encryptedPrefs?.getString("gallery_password", null) != null
    }

    var lastAttemptedPassword by remember { mutableStateOf<String?>(null) }

    // Store password on successful authentication
    LaunchedEffect(viewModel.isAuthenticated) {
        if (viewModel.isAuthenticated && lastAttemptedPassword != null) {
            encryptedPrefs?.edit()?.putString("gallery_password", lastAttemptedPassword)?.apply()
        }
    }

    // Launch biometric prompt
    val promptBiometric: () -> Unit = {
        if (activity != null && canUseBiometric) {
            val storedPw = encryptedPrefs?.getString("gallery_password", null)
            if (storedPw != null) {
                val executor = ContextCompat.getMainExecutor(context)
                val callback = object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        lastAttemptedPassword = storedPw
                        viewModel.unlock(storedPw)
                    }
                }
                val prompt = BiometricPrompt(activity, executor, callback)
                val info = BiometricPrompt.PromptInfo.Builder()
                    .setTitle("Unlock Secure Albums")
                    .setSubtitle("Use biometrics to access your secure albums")
                    .setNegativeButtonText("Use Password")
                    .build()
                prompt.authenticate(info)
            }
        }
    }

    // ── Password Gate ─────────────────────────────────
    if (!viewModel.isAuthenticated) {
        PasswordGate(
            onBack = onBack,
            onUnlock = { password ->
                lastAttemptedPassword = password
                viewModel.unlock(password)
            },
            isLoading = viewModel.authLoading,
            error = viewModel.authError,
            canUseBiometric = canUseBiometric && hasStoredPassword,
            onBiometricRequest = promptBiometric
        )
        return
    }

    // ── Gallery Detail ────────────────────────────────
    val sel = viewModel.selectedGallery
    if (sel != null) {
        GalleryDetailView(
            gallery = sel,
            items = viewModel.items,
            itemsLoading = viewModel.itemsLoading,
            allPhotos = viewModel.allPhotos,
            error = viewModel.error,
            onBack = { viewModel.deselectGallery() },
            onPhotoClick = onPhotoClick,
            onAddPhotos = { viewModel.addPhotosToGallery(it) },
            viewModel = viewModel
        )
        return
    }

    // ── Gallery List ──────────────────────────────────
    GalleryListView(
        galleries = viewModel.galleries,
        galleriesLoading = viewModel.galleriesLoading,
        error = viewModel.error,
        onBack = onBack,
        onGalleryClick = { viewModel.selectGallery(it) },
        onCreateGallery = { viewModel.createGallery(it) },
        onDeleteGallery = { viewModel.deleteGallery(it) }
    )
}

