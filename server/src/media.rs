//! Shared media file utilities — extension detection, MIME mapping.
//!
//! Used by both `photos` and `setup` modules to avoid duplication.
//!
//! Browser-native formats are supported directly.  Non-native formats that
//! can be converted via FFmpeg (HEIC, MKV, TIFF, etc.) are also accepted
//! and converted during import — see [`crate::conversion`].

/// Valid media file extensions — browser-native formats only.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    // Images (all natively renderable by modern browsers)
    "jpg", "jpeg", "png", "gif", "webp", "avif", "bmp", "ico",
    // Videos (universally playable in <video>)
    "mp4", "webm",
    // Audio (universally playable in <audio>)
    "mp3", "flac", "ogg", "wav",
];

/// Check whether a filename has a recognised media extension.
/// O(n) linear scan is fine for ~15 extensions; only used during import scans,
/// not in hot request paths.
pub fn is_media_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    MEDIA_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{}", ext)))
}

/// Returns `true` when the filename is either a native media format
/// OR a convertible format (HEIC, MKV, TIFF, etc.) that the conversion
/// pipeline can handle.
pub fn is_importable_file(name: &str) -> bool {
    is_media_file(name) || crate::conversion::is_convertible(name)
}

/// Returns `true` when the filename extension is a supported media format.
pub fn is_supported_extension(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    MEDIA_EXTENSIONS.contains(&ext.as_str())
}

/// Map a filename extension to its MIME type.
pub fn mime_from_extension(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        // Unknown extension — return generic binary MIME type
        _ => "application/octet-stream",
    }
}
