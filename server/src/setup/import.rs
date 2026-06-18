//! Server-side import: scan directories and stream files for client-side upload.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::auth::middleware::AuthUser;
use crate::conversion;
use crate::error::AppError;
use crate::media::{is_importable_file, mime_from_extension};
use crate::state::AppState;

use super::admin::require_admin;

/// Query parameters for scanning a server-side directory for importable media files.
#[derive(Debug, Deserialize)]
pub struct ImportScanQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportScanResponse {
    pub directory: String,
    pub files: Vec<MediaFileEntry>,
    pub total_size: u64,
}

#[derive(Debug, Serialize)]
pub struct MediaFileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub mime_type: String,
    pub modified: Option<String>,
}

/// Admin-only: Scan a directory for importable media files.
///
/// GET /api/admin/import/scan?path=/some/path
///
/// If no path is given, scans the current storage root.
/// Returns all image/video files (recursive scan through subdirectories).
pub async fn import_scan(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<ImportScanQuery>,
) -> Result<Json<ImportScanResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let scan_path = match &query.path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => (**state.storage_root.load()).clone(),
    };

    let path_str = scan_path.display().to_string();
    if path_str.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let canonical = tokio::fs::canonicalize(&scan_path).await.map_err(|e| {
        AppError::BadRequest(format!(
            "Cannot resolve path '{}': {}",
            scan_path.display(),
            e
        ))
    })?;

    let meta = tokio::fs::metadata(&canonical).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot access '{}': {}", canonical.display(), e))
    })?;

    if !meta.is_dir() {
        return Err(AppError::BadRequest(format!(
            "'{}' is not a directory",
            canonical.display()
        )));
    }

    let mut files = Vec::new();

    // Recursively scan directories for media files
    let mut queue = vec![canonical.clone()];

    while let Some(dir) = queue.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(|e| {
            AppError::BadRequest(format!("Cannot read directory '{}': {}", dir.display(), e))
        })?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && is_importable_file(&name) {
                    let full_path = entry.path().display().to_string();
                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len()).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            dt.to_rfc3339()
                        })
                    });
                    // Use the conversion target's MIME if the file is convertible
                    let mime = if let Some(ct) = conversion::conversion_target(&name) {
                        ct.mime_type.to_string()
                    } else {
                        mime_from_extension(&name).to_string()
                    };

                    files.push(MediaFileEntry {
                        name,
                        path: full_path,
                        size,
                        mime_type: mime,
                        modified,
                    });
                }
            }
        }
    }

    // Google Photos Takeout ships both the unedited original and a baked-in
    // "-edited" copy for every edited photo. Drop the original when its edited
    // sibling is present so the scan doesn't surface duplicates to import.
    let mut files = dedupe_google_photos_edits(files);
    let total_size: u64 = files.iter().map(|f| f.size).sum();

    files.sort_by_key(|a| a.name.to_lowercase());

    tracing::info!(
        "Import scan: {} media files ({} bytes) in {:?}",
        files.len(),
        total_size,
        canonical
    );

    Ok(Json(ImportScanResponse {
        directory: canonical.display().to_string(),
        files,
        total_size,
    }))
}

/// Query parameters for streaming a single file from the server filesystem for client-side import.
#[derive(Debug, Deserialize)]
pub struct ImportFileQuery {
    pub path: String,
}

/// Admin-only: Stream a raw file from the server filesystem for client-side import.
///
/// GET /api/admin/import/file?path=/path/to/photo.jpg
///
/// The client downloads the raw file, encrypts it locally, then uploads as a blob.
pub async fn import_file(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<ImportFileQuery>,
) -> Result<axum::response::Response, AppError> {
    require_admin(&state, &auth).await?;

    let file_path = PathBuf::from(&query.path);

    if query.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let canonical = tokio::fs::canonicalize(&file_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot resolve path '{}': {}", query.path, e))
    })?;

    let meta = tokio::fs::metadata(&canonical)
        .await
        .map_err(|_e| AppError::NotFound)?;

    if !meta.is_file() {
        return Err(AppError::BadRequest("Path is not a file".into()));
    }

    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if !is_importable_file(&name) {
        return Err(AppError::BadRequest("Not a supported media file".into()));
    }

    let mime = if let Some(ct) = conversion::conversion_target(&name) {
        ct.mime_type
    } else {
        mime_from_extension(&name)
    };
    let size = meta.len();

    let file = tokio::fs::File::open(&canonical)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open file: {e}")))?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static(mime))
        .header("Content-Length", HeaderValue::from(size))
        .header(
            "Content-Disposition",
            HeaderValue::from_str(&format!("inline; filename=\"{name}\""))
                .unwrap_or_else(|_| HeaderValue::from_static("inline")),
        )
        .header("Cache-Control", HeaderValue::from_static("no-store"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

// ── Google Photos "-edited" de-duplication ───────────────────────────────────

/// If `name` is a Google Photos Takeout "-edited" variant, return the original
/// filename it derives from (`IMG_1234.jpg` for `IMG_1234-edited.jpg`). Returns
/// `None` for anything that isn't an edited variant. The suffix match is
/// case-insensitive and must sit immediately before the extension.
fn original_name_for_edited(name: &str) -> Option<String> {
    const SUFFIX: &str = "-edited";
    let (stem, ext) = name.rsplit_once('.')?;
    let cut = stem.len().checked_sub(SUFFIX.len())?;
    // `get` returns None on a non-char-boundary, keeping this panic-free for
    // multibyte filenames.
    if stem.get(cut..)?.eq_ignore_ascii_case(SUFFIX) {
        Some(format!("{}.{}", &stem[..cut], ext))
    } else {
        None
    }
}

/// Drop each unedited original whose baked-in "-edited" sibling is also present,
/// so Google Photos Takeout imports don't create duplicates. The edited copy
/// (with the user's edits applied) is kept. Mirrors the client-side
/// `web/src/utils/media.ts::dedupeGooglePhotosEdits`.
fn dedupe_google_photos_edits(files: Vec<MediaFileEntry>) -> Vec<MediaFileEntry> {
    let originals_with_edit: std::collections::HashSet<String> = files
        .iter()
        .filter_map(|f| original_name_for_edited(&f.name))
        .map(|o| o.to_lowercase())
        .collect();
    if originals_with_edit.is_empty() {
        return files;
    }
    files
        .into_iter()
        .filter(|f| !originals_with_edit.contains(&f.name.to_lowercase()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> MediaFileEntry {
        MediaFileEntry {
            name: name.to_string(),
            path: format!("/x/{name}"),
            size: 10,
            mime_type: "image/jpeg".into(),
            modified: None,
        }
    }

    fn names(files: Vec<MediaFileEntry>) -> Vec<String> {
        files.into_iter().map(|f| f.name).collect()
    }

    #[test]
    fn original_name_for_edited_recovers_base() {
        assert_eq!(
            original_name_for_edited("IMG_1234-edited.jpg").as_deref(),
            Some("IMG_1234.jpg")
        );
        // Suffix match is case-insensitive; the base keeps its original casing.
        assert_eq!(
            original_name_for_edited("photo-EDITED.JPG").as_deref(),
            Some("photo.JPG")
        );
        // Plain originals are not edited variants.
        assert_eq!(original_name_for_edited("IMG_1234.jpg"), None);
        // "-edited" must sit right before the extension, not mid-name.
        assert_eq!(original_name_for_edited("my-edited-photo.jpg"), None);
        // No extension → no match (and no panic).
        assert_eq!(original_name_for_edited("noext-edited"), None);
    }

    #[test]
    fn dedupe_drops_original_when_edited_present() {
        let files = vec![
            entry("IMG_1.jpg"),
            entry("IMG_1-edited.jpg"),
            entry("IMG_2.jpg"),
        ];
        let kept = names(dedupe_google_photos_edits(files));
        assert!(kept.contains(&"IMG_1-edited.jpg".to_string()));
        assert!(!kept.contains(&"IMG_1.jpg".to_string()));
        assert!(kept.contains(&"IMG_2.jpg".to_string()));
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn dedupe_keeps_edited_without_original_sibling() {
        let files = vec![entry("solo-edited.jpg"), entry("plain.jpg")];
        assert_eq!(dedupe_google_photos_edits(files).len(), 2);
    }

    #[test]
    fn dedupe_matches_original_case_insensitively() {
        let files = vec![entry("Photo.JPG"), entry("photo-edited.jpg")];
        let kept = names(dedupe_google_photos_edits(files));
        assert_eq!(kept, vec!["photo-edited.jpg".to_string()]);
    }
}
