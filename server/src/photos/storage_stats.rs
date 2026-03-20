//! Storage statistics endpoint.
//!
//! Returns the current user's blob usage broken down by media type,
//! plus filesystem-level totals so the UI can show a full storage picture.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// GET /api/settings/storage-stats
///
/// Returns storage usage stats for the authenticated user and the filesystem.
pub async fn get_storage_stats(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<StorageStatsResponse>, AppError> {
    // ── Per-type blob sizes for the current user ──────────────────────────
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT blob_type, COALESCE(SUM(size_bytes), 0) AS total_bytes, COUNT(*) AS count \
         FROM blobs WHERE user_id = ? GROUP BY blob_type",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    let mut photo_bytes: i64 = 0;
    let mut photo_count: i64 = 0;
    let mut video_bytes: i64 = 0;
    let mut video_count: i64 = 0;
    let mut other_blob_bytes: i64 = 0; // thumbnails, manifests, etc.
    let mut other_blob_count: i64 = 0;

    for (blob_type, bytes, count) in &rows {
        match blob_type.as_str() {
            "photo" | "gif" => {
                photo_bytes += bytes;
                photo_count += count;
            }
            "video" => {
                video_bytes += bytes;
                video_count += count;
            }
            _ => {
                other_blob_bytes += bytes;
                other_blob_count += count;
            }
        }
    }

    let user_total_bytes = photo_bytes + video_bytes + other_blob_bytes;

    // ── Filesystem-level stats ────────────────────────────────────────────
    // We read the storage root's filesystem via statvfs so the user sees
    // total disk capacity, free space, and can compute "other" usage.
    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let (fs_total, fs_free) = get_fs_stats(&storage_root);

    Ok(Json(StorageStatsResponse {
        photo_bytes,
        photo_count,
        video_bytes,
        video_count,
        other_blob_bytes,
        other_blob_count,
        user_total_bytes,
        fs_total_bytes: fs_total,
        fs_free_bytes: fs_free,
    }))
}

#[derive(Debug, Serialize)]
pub struct StorageStatsResponse {
    /// Total bytes of photo + GIF blobs for this user (encrypted mode)
    pub photo_bytes: i64,
    /// Number of photo + GIF blobs
    pub photo_count: i64,
    /// Total bytes of video blobs for this user (encrypted mode)
    pub video_bytes: i64,
    /// Number of video blobs
    pub video_count: i64,
    /// Total bytes of other blobs (thumbnails, album manifests, etc.)
    pub other_blob_bytes: i64,
    /// Count of other blobs
    pub other_blob_count: i64,
    /// Combined total of all blobs for this user
    pub user_total_bytes: i64,
    /// Filesystem total capacity in bytes
    pub fs_total_bytes: i64,
    /// Filesystem free space in bytes
    pub fs_free_bytes: i64,
}

/// Read filesystem stats from the storage root directory using statvfs.
#[cfg(unix)]
fn get_fs_stats(path: &std::path::Path) -> (i64, i64) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = match CString::new(path.as_os_str().as_bytes()) {
        Ok(p) => p,
        Err(_) => return (0, 0),
    };

    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            let total = stat.f_blocks as i64 * stat.f_frsize as i64;
            let free = stat.f_bavail as i64 * stat.f_frsize as i64;
            (total, free)
        } else {
            (0, 0)
        }
    }
}

#[cfg(windows)]
fn get_fs_stats(path: &std::path::Path) -> (i64, i64) {
    use std::os::windows::ffi::OsStrExt;

    // Encode path as null-terminated wide string for Win32 API
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

    let mut free_available: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut _total_free: u64 = 0;

    // SAFETY: calling GetDiskFreeSpaceExW with valid pointers.
    // On failure the function returns 0 and we fall through to (0, 0).
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(
                lpDirectoryName: *const u16,
                lpFreeBytesAvailableToCaller: *mut u64,
                lpTotalNumberOfBytes: *mut u64,
                lpTotalNumberOfFreeBytes: *mut u64,
            ) -> i32;
        }

        if GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_available,
            &mut total_bytes,
            &mut _total_free,
        ) != 0
        {
            return (total_bytes as i64, free_available as i64);
        }
    }

    (0, 0)
}

#[cfg(not(any(unix, windows)))]
fn get_fs_stats(_path: &std::path::Path) -> (i64, i64) {
    (0, 0)
}
