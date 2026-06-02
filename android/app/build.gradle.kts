plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.hilt)
    alias(libs.plugins.ksp)
}

android {
    namespace = "com.simplephotos"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.simplephotos"
        minSdk = 26
        targetSdk = 34
        versionCode = 105
        versionName = "1.1.43"
        testInstrumentationRunner = "com.simplephotos.HiltTestRunner"
    }

    // ── Release signing (CI-driven) ──────────────────────────────────────
    // Populated only when ALL four env vars are set (the GitHub Actions
    // release workflow decodes a base64 keystore secret to a file before
    // invoking gradle). Local developers don't need to set anything — the
    // debug keystore is used automatically (see buildTypes.release below).
    val ksFile      = System.getenv("ANDROID_KEYSTORE_FILE")
    val ksPassword  = System.getenv("ANDROID_KEYSTORE_PASSWORD")
    val keyAlias    = System.getenv("ANDROID_KEY_ALIAS")
    val keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
    val hasReleaseSigning = !ksFile.isNullOrBlank() && !ksPassword.isNullOrBlank() &&
        !keyAlias.isNullOrBlank() && !keyPassword.isNullOrBlank() &&
        file(ksFile).exists()
    if (hasReleaseSigning) {
        signingConfigs {
            create("release") {
                storeFile = file(ksFile!!)
                storePassword = ksPassword
                this.keyAlias = keyAlias
                this.keyPassword = keyPassword
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Use release keystore in CI; fall back to debug keystore so
            // local `assembleRelease` builds still produce an installable APK.
            signingConfig = if (hasReleaseSigning) {
                signingConfigs.getByName("release")
            } else {
                signingConfigs.getByName("debug")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
    }
}

dependencies {
    // Compose
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.tooling.preview)
    implementation(libs.compose.icons)
    implementation(libs.activity.compose)
    debugImplementation(libs.compose.tooling)

    // Hilt
    implementation(libs.hilt.android)
    ksp(libs.hilt.compiler)
    implementation(libs.hilt.navigation.compose)
    implementation(libs.hilt.work)
    ksp(libs.hilt.work.compiler)

    // Room
    implementation(libs.room.runtime)
    implementation(libs.room.ktx)
    ksp(libs.room.compiler)

    // WorkManager
    implementation(libs.work.runtime)

    // Network
    implementation(libs.retrofit)
    implementation(libs.retrofit.gson)
    implementation(libs.okhttp)
    implementation(libs.okhttp.logging)

    // Image loading
    implementation(libs.coil.compose)
    implementation(libs.coil.video)
    implementation(libs.coil.gif)
    implementation(libs.coil.svg)

    // Media3 / ExoPlayer
    implementation(libs.media3.exoplayer)
    implementation(libs.media3.ui)
    implementation(libs.media3.session)
    implementation(libs.media3.datasource)
    implementation(libs.media3.datasource.okhttp)

    // Navigation
    implementation(libs.navigation.compose)

    // DataStore
    implementation(libs.datastore.preferences)

    // Security
    implementation(libs.security.crypto)

    // EXIF
    implementation(libs.exifinterface)

    // Permissions
    implementation(libs.accompanist.permissions)

    // Biometric
    implementation(libs.biometric)

    // Crypto
    implementation(libs.bouncycastle)

    // QR Code
    implementation(libs.zxing.core)

    // Lifecycle
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.lifecycle.viewmodel.compose)

    // Core
    implementation(libs.core.ktx)

    // Local unit tests
    testImplementation("junit:junit:4.13.2")

    // Testing
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    debugImplementation(libs.compose.ui.test.manifest)
    androidTestImplementation(libs.test.runner)
    androidTestImplementation(libs.test.rules)
    androidTestImplementation(libs.test.ext.junit)
    androidTestImplementation(libs.hilt.android.testing)
    kspAndroidTest(libs.hilt.compiler)
}
