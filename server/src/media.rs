//! Shared media file utilities — extension detection, MIME mapping.
//!
//! Used by both `photos` and `setup` modules to avoid duplication.

/// Valid media file extensions for scanning & import.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "heif", "bmp", "tiff", "tif",
    "svg", "dng", "cr2", "nef", "arw", "raw", "ico", "cur", "hdr",
    // Videos
    "mp4", "mov", "mkv", "webm", "avi", "3gp", "m4v", "wmv", "asf", "hevc", "h264", "h265", "mpg", "mpeg",
    // Audio
    "mp3", "aiff", "flac", "ogg", "wav", "wma",
];

/// Check whether a filename has a recognised media extension.
pub fn is_media_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    MEDIA_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{}", ext)))
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
        "heic" => "image/heic",
        "heif" => "image/heif",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "cur" => "image/x-icon",
        "hdr" => "image/vnd.radiance",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "3gp" => "video/3gpp",
        "m4v" => "video/x-m4v",
        "wmv" => "video/x-ms-wmv",
        "asf" => "video/x-ms-asf",
        "hevc" | "h265" => "video/hevc",
        "h264" => "video/h264",
        "mpg" | "mpeg" => "video/mpeg",
        "mp3" => "audio/mpeg",
        "aiff" => "audio/aiff",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "wma" => "audio/x-ms-wma",
        _ => "application/octet-stream",
    }
}
