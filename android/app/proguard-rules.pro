# Simple Photos ProGuard Rules

# Keep Hilt generated classes
-keep class dagger.hilt.** { *; }
-keep class javax.inject.** { *; }
-keep class * extends dagger.hilt.android.internal.managers.ViewComponentManager$FragmentContextWrapper { *; }

# Keep Retrofit interfaces
-keep,allowobfuscation interface * {
    @retrofit2.http.* <methods>;
}
-dontwarn retrofit2.**
-keep class retrofit2.** { *; }

# Keep Gson serialized/deserialized classes
-keep class com.simplephotos.data.remote.dto.** { *; }

# Keep Room entities
-keep class com.simplephotos.data.local.entities.** { *; }

# OkHttp
-dontwarn okhttp3.**
-dontwarn okio.**

# BouncyCastle
-keep class org.bouncycastle.** { *; }
-dontwarn org.bouncycastle.**

# Coroutines
-dontwarn kotlinx.coroutines.**

# Keep Compose
-dontwarn androidx.compose.**
