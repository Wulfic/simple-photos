use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::error::AppError;

/// Build the on-disk path for a blob: `{root}/blobs/{user_id[0..2]}/{user_id}/{blob_id[0..2]}/{blob_id}.bin`
///
/// # Panics / edge cases
/// If `user_id` or `blob_id` is empty the prefix slicing produces `""`,
/// resulting in degenerate paths like `blobs///.bin`. Callers must
/// validate that both IDs are non-empty before calling this function.
pub fn blob_path(root: &Path, user_id: &str, blob_id: &str) -> PathBuf {
    let user_prefix = &user_id[..2.min(user_id.len())];
    let blob_prefix = &blob_id[..2.min(blob_id.len())];
    root.join("blobs")
        .join(user_prefix)
        .join(user_id)
        .join(blob_prefix)
        .join(format!("{}.bin", blob_id))
}

/// Relative storage path stored in DB (root can be moved without migration)
pub fn relative_path(user_id: &str, blob_id: &str) -> String {
    let user_prefix = &user_id[..2.min(user_id.len())];
    let blob_prefix = &blob_id[..2.min(blob_id.len())];
    format!("blobs/{}/{}/{}/{}.bin", user_prefix, user_id, blob_prefix, blob_id)
}

/// Build the on-disk path for a metadata file: {root}/metadata/{user_id[0..2]}/{user_id}/{blob_id}.json
pub fn metadata_path(root: &Path, user_id: &str, blob_id: &str) -> PathBuf {
    let user_prefix = &user_id[..2.min(user_id.len())];
    root.join("metadata")
        .join(user_prefix)
        .join(user_id)
        .join(format!("{}.json", blob_id))
}

/// Relative metadata path stored in DB
pub fn metadata_relative_path(user_id: &str, blob_id: &str) -> String {
    let user_prefix = &user_id[..2.min(user_id.len())];
    format!("metadata/{}/{}/{}.json", user_prefix, user_id, blob_id)
}

/// Write metadata JSON to the metadata subtree.
/// Creates parent directories if they don't exist. Returns the relative
/// storage path suitable for storing in the database.
pub async fn write_metadata(
    root: &Path,
    user_id: &str,
    blob_id: &str,
    data: &[u8],
) -> Result<String, AppError> {
    let path = metadata_path(root, user_id, blob_id);

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create metadata directory: {}", e)))?;
    }

    tokio::fs::write(&path, data)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write metadata: {}", e)))?;

    Ok(metadata_relative_path(user_id, blob_id))
}

/// Read metadata from disk.
pub async fn read_metadata(root: &Path, storage_path: &str) -> Result<Vec<u8>, AppError> {
    let path = root.join(storage_path);
    tokio::fs::read(&path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to read metadata: {}", e)),
        })
}

/// Delete metadata from disk.
pub async fn delete_metadata(root: &Path, storage_path: &str) -> Result<(), AppError> {
    let path = root.join(storage_path);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Internal(format!("Failed to delete metadata: {}", e))),
    }
}

/// Write a blob's bytes to disk. Creates parent directories as needed.
/// Returns the relative storage path for DB storage.
pub async fn write_blob(root: &Path, user_id: &str, blob_id: &str, data: &[u8]) -> Result<String, AppError> {
    let path = blob_path(root, user_id, blob_id);

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create blob directory: {}", e)))?;
    }

    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create blob file: {}", e)))?;

    file.write_all(data)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write blob: {}", e)))?;

    file.flush()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to flush blob: {}", e)))?;

    Ok(relative_path(user_id, blob_id))
}

/// Read a blob's bytes from disk. Returns `AppError::NotFound` if the file is missing.
pub async fn read_blob(root: &Path, storage_path: &str) -> Result<Vec<u8>, AppError> {
    let path = root.join(storage_path);
    tokio::fs::read(&path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to read blob: {}", e)),
        })
}

/// Delete a blob file from disk. Silently succeeds if the file is already gone.
pub async fn delete_blob(root: &Path, storage_path: &str) -> Result<(), AppError> {
    let path = root.join(storage_path);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Internal(format!("Failed to delete blob: {}", e))),
    }
}
