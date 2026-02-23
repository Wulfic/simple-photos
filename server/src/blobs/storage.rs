use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::error::AppError;

/// Build the on-disk path for a blob: {root}/{user_id[0..2]}/{user_id}/{blob_id[0..2]}/{blob_id}.bin
pub fn blob_path(root: &Path, user_id: &str, blob_id: &str) -> PathBuf {
    let user_prefix = &user_id[..2.min(user_id.len())];
    let blob_prefix = &blob_id[..2.min(blob_id.len())];
    root.join(user_prefix)
        .join(user_id)
        .join(blob_prefix)
        .join(format!("{}.bin", blob_id))
}

/// Relative storage path stored in DB (root can be moved without migration)
pub fn relative_path(user_id: &str, blob_id: &str) -> String {
    let user_prefix = &user_id[..2.min(user_id.len())];
    let blob_prefix = &blob_id[..2.min(blob_id.len())];
    format!("{}/{}/{}/{}.bin", user_prefix, user_id, blob_prefix, blob_id)
}

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

pub async fn read_blob(root: &Path, storage_path: &str) -> Result<Vec<u8>, AppError> {
    let path = root.join(storage_path);
    tokio::fs::read(&path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to read blob: {}", e)),
        })
}

pub async fn delete_blob(root: &Path, storage_path: &str) -> Result<(), AppError> {
    let path = root.join(storage_path);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Internal(format!("Failed to delete blob: {}", e))),
    }
}
