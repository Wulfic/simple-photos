//! Download endpoints for serving the Android APK and other client apps.
//!
//! The APK is served from a configurable path on disk. If no APK is available,
//! a helpful error message is returned.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::error::AppError;
use crate::state::AppState;

/// Serve the Android APK file for download.
///
/// **Intentionally unauthenticated** — the APK is a public distributable;
/// any user (or prospective user) on the LAN should be able to install it.
///
/// Looks for the APK in these locations (first match wins):
/// 1. `{project_root}/android/app/build/outputs/apk/release/app-release.apk`
/// 2. `{project_root}/android/app/build/outputs/apk/debug/app-debug.apk`
/// 3. `{project_root}/downloads/simple-photos.apk`
///
/// Returns 404 with instructions if no APK is found.
pub async fn android_apk(
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    // Resolve the project root (go up from server working dir)
    let static_root = std::path::PathBuf::from(&state.config.web.static_root);
    // static_root is typically "../web/dist", so project root is "../" from server cwd
    let project_root = std::env::current_dir()
        .unwrap_or_default()
        .join("..");

    let candidates = [
        project_root.join("android/app/build/outputs/apk/release/app-release.apk"),
        project_root.join("android/app/build/outputs/apk/debug/app-debug.apk"),
        project_root.join("downloads/simple-photos.apk"),
        // Also check relative to static_root parent
        static_root.clone().join("../../downloads/simple-photos.apk"),
    ];

    for path in &candidates {
        if path.exists() {
            let data = tokio::fs::read(path).await.map_err(|e| {
                AppError::Internal(format!("Failed to read APK file: {}", e))
            })?;

            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("simple-photos.apk");

            let mut headers = HeaderMap::new();
            headers.insert(
                "Content-Type",
                HeaderValue::from_static("application/vnd.android.package-archive"),
            );
            headers.insert(
                "Content-Disposition",
                HeaderValue::from_str(&format!(
                    "attachment; filename=\"{}\"",
                    filename
                ))
                .unwrap_or_else(|_| {
                    HeaderValue::from_static("attachment; filename=\"simple-photos.apk\"")
                }),
            );

            tracing::info!("Serving APK download: {:?}", path);
            return Ok((StatusCode::OK, headers, data).into_response());
        }
    }

    // No APK found — return a helpful error
    tracing::debug!(
        "No APK found. Searched: {:?}",
        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()
    );

    let body = serde_json::json!({
        "error": "Android APK not available",
        "message": "The Android app has not been built yet. To build it:\n\n  cd android && ./gradlew assembleRelease\n\nOr place a pre-built APK at: downloads/simple-photos.apk"
    });

    Ok((
        StatusCode::NOT_FOUND,
        axum::Json(body),
    ).into_response())
}
