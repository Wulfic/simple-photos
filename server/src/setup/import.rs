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

// ── Bulk server-side ingest (no browser round-trip) ──────────────────────────

/// Query parameters for bulk server-side ingest.
#[derive(Debug, Deserialize)]
pub struct ImportIngestQuery {
    pub path: Option<String>,
    /// "copy" (default) or "move" — whether to delete each source after import.
    pub mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportIngestResponse {
    /// Number of importable files discovered to process in the background.
    pub queued: usize,
    /// True when the path is already under the storage root (registered in
    /// place — no copy needed).
    pub in_place: bool,
    pub directory: String,
}

/// Admin-only: ingest every importable media file under `path` into the library
/// WITHOUT round-tripping bytes through the browser.
///
/// POST /api/admin/import/ingest?path=/some/path&mode=copy
///
/// - If `path` is under the storage root, files are registered in place by the
///   normal scan pipeline (no copy).
/// - Otherwise each file is stream-copied into the storage `uploads/` tree
///   (bounded concurrency, content-hash de-duplication, original mtime
///   preserved) and then registered.
///
/// Returns `202 Accepted` immediately with the number of files queued; the
/// copy + convert + encrypt work proceeds in the background and surfaces in the
/// existing conversion/encryption progress banners. The operation is idempotent
/// (hash de-dup), so it can be re-run to resume an interrupted ingest. This is
/// the canonical replacement for the old download-then-reupload flow, which
/// pulled the entire library through the browser and failed/​partially-imported
/// on large folders.
pub async fn import_ingest(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<ImportIngestQuery>,
) -> Result<(StatusCode, Json<ImportIngestResponse>), AppError> {
    require_admin(&state, &auth).await?;

    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let storage_root = (**state.storage_root.load()).clone();

    let scan_path = match &query.path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => storage_root.clone(),
    };
    if scan_path.to_string_lossy().contains("..") {
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

    // Is the source already inside the storage tree? Then the scan pipeline
    // registers it in place and no copy is needed.
    let in_place = tokio::fs::canonicalize(&storage_root)
        .await
        .map(|sr| canonical.starts_with(&sr))
        .unwrap_or(false);

    // Count importable files up front so the client can show a denominator.
    let candidates = collect_importable_files(&canonical).await;
    let queued = candidates.len();
    let move_after = query.mode.as_deref() == Some("move");

    tracing::info!(
        path = %canonical.display(),
        queued,
        in_place,
        move_after,
        "[INGEST] Bulk server-side ingest queued"
    );

    // Run the whole ingest in the background and return immediately — copying a
    // large library can take a long time and must not block the HTTP response.
    {
        let pool = state.pool.clone();
        let read_pool = state.read_pool.clone();
        let storage_root = storage_root.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        let scan_lock = state.scan_lock.clone();
        let geo_trigger = state.geo_trigger.clone();
        tokio::spawn(async move {
            bulk_ingest_from_path(
                pool,
                read_pool,
                storage_root,
                jwt_secret,
                scan_lock,
                geo_trigger,
                candidates,
                in_place,
                move_after,
            )
            .await;
        });
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(ImportIngestResponse {
            queued,
            in_place,
            directory: canonical.display().to_string(),
        }),
    ))
}

/// Recursively collect importable media files under `root` as (abs_path, name).
async fn collect_importable_files(root: &std::path::Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut queue = vec![root.to_path_buf()];
    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = ?dir, error = %e, "[INGEST] Skipping unreadable directory");
                continue;
            }
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && is_importable_file(&name) {
                    out.push((entry.path(), name));
                }
            }
        }
    }
    out
}

/// Background worker: copy out-of-tree files into storage (bounded concurrency,
/// hash de-dup, mtime preserved), then run the standard scan → encrypt →
/// convert pipeline. For in-place paths the copy step is skipped entirely.
#[allow(clippy::too_many_arguments)]
async fn bulk_ingest_from_path(
    pool: sqlx::SqlitePool,
    read_pool: sqlx::SqlitePool,
    storage_root: PathBuf,
    jwt_secret: String,
    scan_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    geo_trigger: std::sync::Arc<tokio::sync::Notify>,
    candidates: Vec<(PathBuf, String)>,
    in_place: bool,
    move_after: bool,
) {
    use futures_util::stream::{self, StreamExt};
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    if !in_place && !candidates.is_empty() {
        let uploads_dir = storage_root.join("uploads");
        if let Err(e) = tokio::fs::create_dir_all(&uploads_dir).await {
            tracing::error!(error = %e, "[INGEST] Failed to create uploads dir; aborting bulk ingest");
            return;
        }

        let copied = Arc::new(AtomicI64::new(0));
        let skipped = Arc::new(AtomicI64::new(0));
        let failed = Arc::new(AtomicI64::new(0));

        stream::iter(candidates)
            .map(|(src, name)| {
                let read_pool = read_pool.clone();
                let uploads_dir = uploads_dir.clone();
                let copied = copied.clone();
                let skipped = skipped.clone();
                let failed = failed.clone();
                async move {
                    // Content-hash de-dup: skip files already in the library so
                    // re-running is idempotent and never leaves orphan copies.
                    if let Some(hash) =
                        crate::photos::utils::compute_photo_hash_streaming(&src).await
                    {
                        let exists: bool = sqlx::query_scalar(
                            "SELECT EXISTS(SELECT 1 FROM photos WHERE photo_hash = ?)",
                        )
                        .bind(&hash)
                        .fetch_one(&read_pool)
                        .await
                        .unwrap_or(false);
                        if exists {
                            skipped.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                    }

                    match copy_into_uploads(&src, &name, &uploads_dir, move_after).await {
                        Ok(()) => {
                            copied.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!(
                                file = %name,
                                error = %e,
                                "[INGEST] Failed to copy file into storage"
                            );
                        }
                    }
                }
            })
            .buffer_unordered(4)
            .for_each(|_| async {})
            .await;

        tracing::info!(
            copied = copied.load(Ordering::Relaxed),
            skipped = skipped.load(Ordering::Relaxed),
            failed = failed.load(Ordering::Relaxed),
            "[INGEST] Bulk copy into storage complete"
        );
    }

    // Register everything (native in place), then encrypt + convert non-native.
    // Hold the scan lock so we don't race the periodic autoscan.
    let new_count = {
        let _guard = scan_lock.lock().await;
        crate::backup::autoscan::run_auto_scan_public(&pool, &storage_root).await
    };
    tracing::info!(
        new_count,
        "[INGEST] Bulk ingest scan registered new native files"
    );

    if new_count > 0 {
        geo_trigger.notify_one();
    }

    crate::photos::server_migrate::auto_migrate_after_scan(
        pool.clone(),
        storage_root.clone(),
        jwt_secret.clone(),
    )
    .await;
    crate::ingest::run_conversion_pass(pool, storage_root, jwt_secret).await;
}

/// Stream-copy (or move) a single source file into `uploads_dir` under a unique
/// name, preserving the source mtime. Uses `tokio::fs::copy`, which streams in
/// bounded chunks — never the whole file in RAM.
async fn copy_into_uploads(
    src: &std::path::Path,
    name: &str,
    uploads_dir: &std::path::Path,
    move_after: bool,
) -> std::io::Result<()> {
    let safe = crate::sanitize::sanitize_filename(name);

    // Unique destination name (different content, same name → keep both).
    let mut final_name = safe.clone();
    let mut counter = 1u32;
    while tokio::fs::try_exists(uploads_dir.join(&final_name))
        .await
        .unwrap_or(false)
    {
        let stem = std::path::Path::new(&safe)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = std::path::Path::new(&safe)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin");
        final_name = format!("{stem}-{counter}.{ext}");
        counter += 1;
    }
    let dest = uploads_dir.join(&final_name);

    // Preserve the source mtime so EXIF-less files land in the right timeline
    // slot (the scan pipeline falls back to on-disk mtime).
    let src_mtime = tokio::fs::metadata(src)
        .await
        .ok()
        .and_then(|m| m.modified().ok());

    if move_after {
        // Try a cheap same-filesystem rename first; fall back to copy + remove.
        if tokio::fs::rename(src, &dest).await.is_err() {
            tokio::fs::copy(src, &dest).await?;
            let _ = tokio::fs::remove_file(src).await;
        }
    } else {
        tokio::fs::copy(src, &dest).await?;
    }

    if let Some(mtime) = src_mtime {
        let dest_clone = dest.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&dest_clone) {
                let _ = f.set_modified(mtime);
            }
        })
        .await;
    }

    Ok(())
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

    // ── Bulk ingest helpers ──────────────────────────────────────────────

    fn unique_tmpdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sp_ingest_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn collect_importable_files_recurses_skips_dotfiles_and_nonmedia() {
        let root = unique_tmpdir();
        std::fs::write(root.join("a.jpg"), b"x").unwrap();
        std::fs::write(root.join("b.png"), b"x").unwrap();
        std::fs::write(root.join("note.txt"), b"x").unwrap(); // not media
        std::fs::write(root.join(".hidden.jpg"), b"x").unwrap(); // dotfile, skipped
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("c.mp4"), b"x").unwrap();

        let mut got: Vec<String> = collect_importable_files(&root)
            .await
            .into_iter()
            .map(|(_, n)| n)
            .collect();
        got.sort();
        assert_eq!(got, vec!["a.jpg", "b.png", "c.mp4"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn copy_into_uploads_copies_dedupes_names_and_moves() {
        let root = unique_tmpdir();
        let uploads = root.join("uploads");
        std::fs::create_dir_all(&uploads).unwrap();

        // Copy: source preserved, dest has identical bytes.
        let src = root.join("pic.jpg");
        std::fs::write(&src, b"hello").unwrap();
        copy_into_uploads(&src, "pic.jpg", &uploads, false)
            .await
            .unwrap();
        assert!(src.exists(), "copy must not remove the source");
        assert_eq!(std::fs::read(uploads.join("pic.jpg")).unwrap(), b"hello");

        // Name collision with different content → keep both as pic-1.jpg.
        let src2 = root.join("other.jpg");
        std::fs::write(&src2, b"world").unwrap();
        copy_into_uploads(&src2, "pic.jpg", &uploads, false)
            .await
            .unwrap();
        assert_eq!(std::fs::read(uploads.join("pic-1.jpg")).unwrap(), b"world");

        // Move: source removed after import.
        let src3 = root.join("move-me.jpg");
        std::fs::write(&src3, b"bye").unwrap();
        copy_into_uploads(&src3, "move-me.jpg", &uploads, true)
            .await
            .unwrap();
        assert!(!src3.exists(), "move must remove the source");
        assert_eq!(std::fs::read(uploads.join("move-me.jpg")).unwrap(), b"bye");

        let _ = std::fs::remove_dir_all(&root);
    }
}
