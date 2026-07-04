//! Definitive Google Photos Takeout sidecar detection & resolution.
//!
//! Google Takeout ships a JSON metadata sidecar next to each media file, but the
//! sidecar's *name* follows a pile of undocumented, version-dependent rules that
//! are the reason "Google Photos imports never work": the exported JPEG usually
//! has its capture date and GPS stripped, so the ONLY place the true
//! `photoTakenTime`/`geoData` survive is that sidecar — and if we fail to pair
//! it, the photo lands with the wrong date (the unzip date) and no location.
//!
//! This module is the single source of truth for that pairing. A caller builds a
//! [`TakeoutDirContext`] once per directory (from the `.json` names it already
//! saw while walking), then asks it to resolve the sidecar for each media file.
//!
//! Naming rules handled (for a media file `NAME.EXT`):
//!
//! | Rule | Example media | Sidecar |
//! |------|---------------|---------|
//! | Supplemental (2023+) | `IMG_1.jpg` | `IMG_1.jpg.supplemental-metadata.json` |
//! | Legacy               | `IMG_1.jpg` | `IMG_1.jpg.json` |
//! | Length truncation    | `a_very_long_name.jpg` | `a_very_long_name.jpg.supplemental-m….json` |
//! | Duplicate counter    | `IMG_1(1).jpg` | `IMG_1.jpg(1).json` |
//! | `-edited` inheritance| `IMG_1-edited.jpg` | *(reuses `IMG_1.jpg`'s sidecar)* |
//!
//! Detection is schema-validated ([`is_photo_sidecar`]) so a matched JSON that is
//! actually an album-level `metadata.json` (or `print-subscriptions.json`, …) is
//! never mistaken for a per-photo sidecar.

use std::collections::HashMap;
use std::path::Path;

use crate::media::original_name_for_edited;

use super::models::GooglePhotosMetadata;

/// Per-directory Takeout context: the `.json` sidecars present in one directory
/// plus the album that directory represents. Built once per directory during the
/// import walk so per-file resolution is a cheap in-memory lookup.
pub struct TakeoutDirContext {
    /// lowercased json filename → original-case json filename (on-disk casing).
    json_by_lower: HashMap<String, String>,
    /// The album this folder represents, or `None` for date/container folders.
    album: Option<String>,
}

impl TakeoutDirContext {
    /// Build a context from the `.json` filenames found in `dir`. Non-`.json`
    /// names are ignored, so a caller can pass every entry name it saw.
    pub fn new<I, S>(json_names: I, dir: &Path) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut json_by_lower = HashMap::new();
        for n in json_names {
            let n = n.into();
            if n.to_lowercase().ends_with(".json") {
                json_by_lower.insert(n.to_lowercase(), n);
            }
        }
        Self {
            json_by_lower,
            album: derive_album_from_dir(dir),
        }
    }

    /// True when this directory holds at least one Google sidecar — i.e. it is a
    /// Takeout export directory. Used to gate album recording so a plain user
    /// folder (no sidecars) never gets turned into an album.
    pub fn is_takeout(&self) -> bool {
        !self.json_by_lower.is_empty()
    }

    /// The Takeout album name for files in this directory, or `None` for
    /// date/container/non-Takeout folders. Only yields an album when this really
    /// looks like a Takeout directory ([`is_takeout`](Self::is_takeout)).
    pub fn album_name(&self) -> Option<&str> {
        if self.is_takeout() {
            self.album.as_deref()
        } else {
            None
        }
    }

    /// Resolve the Google sidecar filename for `media_name`, returning the
    /// on-disk (original-case) filename present in this directory. Join it with
    /// the directory to read the sidecar. Returns `None` when nothing pairs.
    pub fn resolve_sidecar(&self, media_name: &str) -> Option<String> {
        if self.json_by_lower.is_empty() {
            return None;
        }
        // Try the file's own name first. A "-edited" copy carries no sidecar of
        // its own in Takeout, so fall back to the unedited original's sidecar
        // (we keep the edited pixels and drop the original, so the surviving
        // edited row must inherit the original's metadata).
        self.resolve_for(media_name).or_else(|| {
            original_name_for_edited(media_name).and_then(|orig| self.resolve_for(&orig))
        })
    }

    /// Resolve the sidecar for one exact media name (no `-edited` fallback).
    fn resolve_for(&self, base: &str) -> Option<String> {
        for cand in exact_candidates(base) {
            if let Some(orig) = self.json_by_lower.get(&cand.to_lowercase()) {
                return Some(orig.clone());
            }
        }
        self.fuzzy_supplemental(base)
    }

    /// Length-truncation fallback: Google caps the sidecar filename length,
    /// truncating the trailing `.supplemental-metadata` while keeping the whole
    /// media filename. Accept the longest `.json` whose stem keeps the full media
    /// name intact AND is a prefix of the intended `"<base>.supplemental-metadata"`
    /// — so a truncated boilerplate tail still pairs, but we never mis-pair a
    /// *different* media file's sidecar.
    fn fuzzy_supplemental(&self, base: &str) -> Option<String> {
        let base_lower = base.to_lowercase();
        let intended = format!("{base_lower}.supplemental-metadata");
        let mut best: Option<(&String, usize)> = None;
        for (lower, orig) in &self.json_by_lower {
            let Some(stem) = lower.strip_suffix(".json") else {
                continue;
            };
            if stem.len() > base_lower.len()
                && stem.starts_with(&base_lower)
                && intended.starts_with(stem)
            {
                let better = match best {
                    Some((_, len)) => stem.len() > len,
                    None => true,
                };
                if better {
                    best = Some((orig, stem.len()));
                }
            }
        }
        best.map(|(orig, _)| orig.clone())
    }
}

/// Candidate sidecar names for a media file, most-specific first (exact matches).
fn exact_candidates(base: &str) -> Vec<String> {
    let mut v = vec![
        // Newer default (2023+): full media name + ".supplemental-metadata.json".
        format!("{base}.supplemental-metadata.json"),
        // Older default: full media name + ".json".
        format!("{base}.json"),
    ];
    // Duplicate-counter displacement: to disambiguate duplicate filenames Google
    // appends "(n)" — but on the SIDECAR it moves that counter to *after* the
    // extension: "IMG_1234(1).JPG" → "IMG_1234.JPG(1).json".
    if let Some((stem, ext, n)) = split_media_counter(base) {
        v.push(format!("{stem}.{ext}({n}).json"));
        v.push(format!("{stem}.{ext}.supplemental-metadata({n}).json"));
    }
    v
}

/// If `base` is a duplicate-numbered media name `STEM(N).EXT`, split it into
/// `(STEM, EXT, N)`. Returns `None` for anything without a trailing `(digits)`
/// immediately before a real extension.
fn split_media_counter(base: &str) -> Option<(String, String, String)> {
    let (stem_c, ext) = base.rsplit_once('.')?;
    // A real extension is short and alphanumeric — don't treat "file(1)" (no
    // extension) or "a.b.c" oddities as counters.
    if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    if !stem_c.ends_with(')') {
        return None;
    }
    let open = stem_c.rfind('(')?;
    let n = &stem_c[open + 1..stem_c.len() - 1];
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((stem_c[..open].to_string(), ext.to_string(), n.to_string()))
}

/// Is the parsed JSON a per-*photo* Google sidecar (as opposed to an album-level
/// `metadata.json`, `print-subscriptions.json`, or some unrelated `.json`)?
///
/// A per-photo sidecar always carries a `photoTakenTime` and/or the
/// `googlePhotosOrigin` marker; album metadata has a `title` but neither. This
/// guard means a coincidentally-named `.json` can never poison a photo's date.
pub fn is_photo_sidecar(meta: &GooglePhotosMetadata) -> bool {
    meta.photo_taken_time.is_some() || meta.google_photos_origin.is_some()
}

/// Derive the Takeout album name from a directory: its own folder name, unless
/// that folder is one of Google's non-album date/container folders.
///
/// Mirrors the web allowlist (`web/src/utils/takeoutAlbums.ts`): skips the
/// `Photos from YYYY` date folders and the `Takeout` / `Google Photos` container
/// folders (including common localisations).
pub fn derive_album_from_dir(dir: &Path) -> Option<String> {
    let folder = dir.file_name()?.to_string_lossy().to_string();
    if is_non_album_folder(&folder) {
        None
    } else {
        Some(folder)
    }
}

/// Matches Google's non-album container/date folders (case-insensitive).
pub fn is_non_album_folder(folder: &str) -> bool {
    let lower = folder.trim().to_lowercase();
    if lower.is_empty() {
        return true;
    }
    // "Photos from 2023", "Photos from 1998", …
    if let Some(rest) = lower.strip_prefix("photos from ") {
        if rest.len() >= 4 && rest.as_bytes()[..4].iter().all(|b| b.is_ascii_digit()) {
            return true;
        }
    }
    matches!(
        lower.as_str(),
        "takeout" | "google photos" | "google fotos" | "google foto's"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn ctx(names: &[&str]) -> TakeoutDirContext {
        TakeoutDirContext::new(
            names.iter().map(|s| s.to_string()),
            Path::new("/t/Google Photos/Summer 2023"),
        )
    }

    #[test]
    fn supplemental_metadata_pairs() {
        let c = ctx(&["IMG_1.jpg.supplemental-metadata.json", "unrelated.json"]);
        assert_eq!(
            c.resolve_sidecar("IMG_1.jpg").as_deref(),
            Some("IMG_1.jpg.supplemental-metadata.json")
        );
    }

    #[test]
    fn legacy_json_pairs() {
        let c = ctx(&["IMG_2.jpg.json"]);
        assert_eq!(
            c.resolve_sidecar("IMG_2.jpg").as_deref(),
            Some("IMG_2.jpg.json")
        );
    }

    #[test]
    fn casing_differences_pair() {
        // On-disk sidecar uses different extension casing than the media name.
        let c = ctx(&["Photo.JPG.supplemental-metadata.json"]);
        assert_eq!(
            c.resolve_sidecar("Photo.jpg").as_deref(),
            Some("Photo.JPG.supplemental-metadata.json")
        );
    }

    #[test]
    fn duplicate_counter_is_displaced_after_extension() {
        // THE classic Takeout gotcha: "IMG_1234(1).JPG" → "IMG_1234.JPG(1).json".
        let c = ctx(&["IMG_1234.JPG(1).json"]);
        assert_eq!(
            c.resolve_sidecar("IMG_1234(1).JPG").as_deref(),
            Some("IMG_1234.JPG(1).json")
        );
    }

    #[test]
    fn duplicate_counter_with_supplemental() {
        let c = ctx(&["IMG_9.jpg.supplemental-metadata(2).json"]);
        assert_eq!(
            c.resolve_sidecar("IMG_9(2).jpg").as_deref(),
            Some("IMG_9.jpg.supplemental-metadata(2).json")
        );
    }

    #[test]
    fn edited_copy_inherits_original_sidecar() {
        // The "-edited" file has no sidecar of its own; it inherits the base's.
        let c = ctx(&["IMG_5.jpg.supplemental-metadata.json"]);
        assert_eq!(
            c.resolve_sidecar("IMG_5-edited.jpg").as_deref(),
            Some("IMG_5.jpg.supplemental-metadata.json")
        );
    }

    #[test]
    fn edited_copy_prefers_own_sidecar_when_present() {
        let c = ctx(&[
            "IMG_5.jpg.supplemental-metadata.json",
            "IMG_5-edited.jpg.supplemental-metadata.json",
        ]);
        assert_eq!(
            c.resolve_sidecar("IMG_5-edited.jpg").as_deref(),
            Some("IMG_5-edited.jpg.supplemental-metadata.json")
        );
    }

    #[test]
    fn truncated_supplemental_tail_still_pairs() {
        // Google caps the sidecar filename length: the ".supplemental-metadata"
        // tail is cut but the whole media name survives.
        let media = "a_really_quite_long_original_filename.jpg";
        let truncated = "a_really_quite_long_original_filename.jpg.supplemental-me.json";
        let c = ctx(&[truncated]);
        assert_eq!(c.resolve_sidecar(media).as_deref(), Some(truncated));
    }

    #[test]
    fn truncation_never_pairs_a_different_media_file() {
        // Two long names sharing a prefix must not steal each other's sidecar.
        let c = ctx(&["holiday_beach_sunset_2021.jpg.supplemental-metad.json"]);
        assert_eq!(
            c.resolve_sidecar("holiday_beach_sunrise_2021.jpg"),
            None,
            "different media name must not fuzzy-match"
        );
    }

    #[test]
    fn no_sidecar_returns_none() {
        let c = ctx(&["something-else.jpg.json"]);
        assert_eq!(c.resolve_sidecar("IMG_404.jpg"), None);
    }

    #[test]
    fn empty_dir_is_not_takeout() {
        let c = ctx(&[]);
        assert!(!c.is_takeout());
        assert_eq!(c.album_name(), None);
        assert_eq!(c.resolve_sidecar("x.jpg"), None);
    }

    #[test]
    fn album_name_from_folder_only_when_takeout() {
        // A real album folder WITH sidecars → album is its folder name.
        let with = TakeoutDirContext::new(
            ["a.jpg.json".to_string()],
            Path::new("/t/Google Photos/Trip to Rome"),
        );
        assert_eq!(with.album_name(), Some("Trip to Rome"));

        // Same folder name but NO sidecars (a plain user folder) → never an album.
        let without = TakeoutDirContext::new(Vec::<String>::new(), Path::new("/t/Vacation Photos"));
        assert_eq!(without.album_name(), None);
    }

    #[test]
    fn date_and_container_folders_are_not_albums() {
        let date = TakeoutDirContext::new(
            ["a.jpg.json".to_string()],
            Path::new("/t/Google Photos/Photos from 2021"),
        );
        assert_eq!(date.album_name(), None);

        let container = TakeoutDirContext::new(
            ["a.jpg.json".to_string()],
            Path::new("/t/Takeout/Google Photos"),
        );
        assert_eq!(container.album_name(), None);
    }

    #[test]
    fn non_album_predicate_is_case_insensitive_and_localised() {
        assert!(is_non_album_folder("TAKEOUT"));
        assert!(is_non_album_folder("Google Photos"));
        assert!(is_non_album_folder("Google Fotos"));
        assert!(is_non_album_folder("photos from 2020"));
        assert!(is_non_album_folder(""));
        assert!(!is_non_album_folder("Vacation"));
        // "Photos from Grandma" is a real album — only YYYY date folders skip.
        assert!(!is_non_album_folder("Photos from Grandma"));
    }

    #[test]
    fn split_media_counter_parses_only_real_counters() {
        assert_eq!(
            split_media_counter("IMG_1234(1).JPG"),
            Some(("IMG_1234".into(), "JPG".into(), "1".into()))
        );
        assert_eq!(split_media_counter("IMG_1234.JPG"), None);
        assert_eq!(split_media_counter("no-counter(x).jpg"), None);
        assert_eq!(split_media_counter("file(1)"), None); // no extension
    }

    #[test]
    fn is_photo_sidecar_rejects_album_metadata() {
        // Album-level metadata.json: has a title but no photoTakenTime / origin.
        let album: GooglePhotosMetadata =
            serde_json::from_str(r#"{"title":"Summer 2023","access":"protected"}"#).unwrap();
        assert!(!is_photo_sidecar(&album));

        // A real per-photo sidecar.
        let photo: GooglePhotosMetadata = serde_json::from_str(
            r#"{"title":"IMG_1.jpg","photoTakenTime":{"timestamp":"1494963474"}}"#,
        )
        .unwrap();
        assert!(is_photo_sidecar(&photo));

        // Origin marker alone is enough (some sidecars omit photoTakenTime).
        let origin: GooglePhotosMetadata =
            serde_json::from_str(r#"{"title":"x","googlePhotosOrigin":{"driveSync":{}}}"#).unwrap();
        assert!(is_photo_sidecar(&origin));
    }
}
