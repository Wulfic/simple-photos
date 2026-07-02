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
    "mp4", "webm", // Audio (universally playable in <audio>)
    "mp3", "flac", "ogg", "wav",
];

/// Check whether a filename has a recognised media extension.
/// O(n) linear scan is fine for ~15 extensions; only used during import scans,
/// not in hot request paths.
pub fn is_media_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    MEDIA_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
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

// ── Google Photos Takeout "-edited" de-duplication ───────────────────────────
//
// Google Photos Takeout ships BOTH the unedited original (`IMG_1234.jpg`) and a
// baked-in edited copy (`IMG_1234-edited.jpg`) for every edited photo. We keep
// the edited pixels and drop the original. This is the single source of truth
// for that rule on the server — every import path (bulk ingest, autoscan
// registration, and the legacy import scan) routes through it so dedup behaves
// identically everywhere instead of being reimplemented per path. The web
// client mirrors it in `web/src/utils/media.ts::dedupeGooglePhotosEdits` (the
// two languages can't share code, but the rule and tests are kept in lockstep).

/// The Google Photos "-edited" suffix, matched case-insensitively immediately
/// before the file extension.
const EDITED_SUFFIX: &str = "-edited";

/// If `name` is a Google Photos Takeout "-edited" variant, return the original
/// filename it derives from (`IMG_1234.jpg` for `IMG_1234-edited.jpg`). Returns
/// `None` for anything that isn't an edited variant. The suffix match is
/// case-insensitive and must sit immediately before the extension; `get`
/// returns `None` on a non-char-boundary so this stays panic-free for multibyte
/// filenames.
pub fn original_name_for_edited(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    let cut = stem.len().checked_sub(EDITED_SUFFIX.len())?;
    if stem.get(cut..)?.eq_ignore_ascii_case(EDITED_SUFFIX) {
        Some(format!("{}.{}", &stem[..cut], ext))
    } else {
        None
    }
}

/// Given an iterator of filenames, return the lowercased original names implied
/// by every "-edited" file present (`img_1234.jpg` for `IMG_1234-edited.jpg`).
/// Callers drop any file whose lowercased name is in this set, keeping the
/// edited copy — so an implied original that isn't actually present is simply a
/// no-op for the filter. Empty when there are no "-edited" files.
pub fn edited_shadowed_originals<'a, I>(names: I) -> std::collections::HashSet<String>
where
    I: IntoIterator<Item = &'a str>,
{
    names
        .into_iter()
        .filter_map(original_name_for_edited)
        .map(|o| o.to_lowercase())
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_name_for_edited_recovers_base() {
        assert_eq!(
            original_name_for_edited("IMG_1234-edited.jpg").as_deref(),
            Some("IMG_1234.jpg")
        );
        // Case-insensitive suffix; base keeps its original casing.
        assert_eq!(
            original_name_for_edited("photo-EDITED.JPG").as_deref(),
            Some("photo.JPG")
        );
        // Plain originals aren't edited variants.
        assert_eq!(original_name_for_edited("IMG_1234.jpg"), None);
        // "-edited" must sit right before the extension, not mid-name.
        assert_eq!(original_name_for_edited("my-edited-photo.jpg"), None);
        // No extension → no match (and no panic).
        assert_eq!(original_name_for_edited("noext-edited"), None);
    }

    #[test]
    fn edited_shadowed_originals_flags_implied_originals() {
        let names = ["IMG_1.jpg", "IMG_1-edited.jpg", "IMG_2.jpg", "solo-edited.png"];
        let shadowed = edited_shadowed_originals(names.iter().copied());
        // Every "-edited" file contributes its implied original (lowercased).
        assert!(shadowed.contains("img_1.jpg"));
        assert!(shadowed.contains("solo.png"));
        // A plain file with no "-edited" sibling is never in the set.
        assert!(!shadowed.contains("img_2.jpg"));
        assert_eq!(shadowed.len(), 2);
        // The point of the filter: only names actually PRESENT get dropped, so
        // `solo.png` (absent) is a no-op while `IMG_1.jpg` (present) is dropped.
        let kept: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| !shadowed.contains(&n.to_lowercase()))
            .collect();
        assert_eq!(kept, vec!["IMG_1-edited.jpg", "IMG_2.jpg", "solo-edited.png"]);
    }

    #[test]
    fn edited_shadowed_originals_is_case_insensitive() {
        // On-disk casing differs between the original and its edited sibling.
        let names = ["Photo.JPG", "photo-edited.jpg"];
        let shadowed = edited_shadowed_originals(names.iter().copied());
        assert!(shadowed.contains("photo.jpg"));
    }
}
