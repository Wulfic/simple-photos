//! Blob receive endpoint: accept a client-encrypted blob pushed from the
//! primary, persist it to disk, upsert metadata, and deduplicate against any
//! migration-created blob with the same content hash.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};

use crate::error::AppError;
use crate::state::AppState;

use super::validate_api_key;

/// POST /api/backup/receive-blob
/// Receives a client-encrypted blob file pushed from the primary server.
/// Writes the file to disk and upserts the `blobs` table metadata.
pub async fn backup_receive_blob(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let blob_id = headers
        .get("X-Blob-Id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-Blob-Id header".into()))?
        .to_string();

    let storage_path_raw = headers
        .get("X-Storage-Path")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-Storage-Path header".into()))?;

    let storage_path = percent_encoding::percent_decode_str(storage_path_raw)
        .decode_utf8()
        .map_err(|_| AppError::BadRequest("Invalid X-Storage-Path encoding".into()))?
        .to_string();

    // Security: validate path to prevent traversal
    crate::sanitize::validate_relative_path(&storage_path)
        .map_err(|e| AppError::BadRequest(format!("Invalid storage path: {e}")))?;

    // Verify checksum
    if let Some(expected) = headers.get("X-Content-Hash").and_then(|v| v.to_str().ok()) {
        use sha2::Digest;
        let actual = hex::encode(sha2::Sha256::digest(&body));
        if actual != expected {
            return Err(AppError::BadRequest("Content hash mismatch".into()));
        }
    }

    // Parse metadata headers
    let user_id = headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let blob_type = headers
        .get("X-Blob-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("photo")
        .to_string();
    let upload_time = headers
        .get("X-Upload-Time")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let size_bytes = headers
        .get("X-Size-Bytes")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(body.len() as i64);
    let client_hash: Option<String> = headers
        .get("X-Client-Hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let content_hash: Option<String> = headers
        .get("X-Original-Content-Hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Write file to disk
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&storage_path);
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create blob dir: {e}")))?;
    }
    tokio::fs::write(&full_path, &body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write blob: {e}")))?;

    // Upsert blob metadata — fall back to admin if user doesn't exist locally
    let effective_user_id = if user_id.is_empty() {
        sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE role = 'admin' LIMIT 1")
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or_default()
    } else {
        let exists: bool =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE id = ?)")
                .bind(&user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(false);
        if exists {
            user_id
        } else {
            sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE role = 'admin' LIMIT 1")
                .fetch_optional(&state.pool)
                .await?
                .unwrap_or_default()
        }
    };

    if effective_user_id.is_empty() {
        tracing::warn!(
            "receive-blob: no valid user for blob {}, skipping DB insert",
            blob_id
        );
        return Ok(StatusCode::OK);
    }

    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           user_id = excluded.user_id, \
           blob_type = excluded.blob_type, \
           size_bytes = excluded.size_bytes, \
           client_hash = excluded.client_hash, \
           content_hash = excluded.content_hash, \
           storage_path = excluded.storage_path",
    )
    .bind(&blob_id)
    .bind(&effective_user_id)
    .bind(&blob_type)
    .bind(size_bytes)
    .bind(&client_hash)
    .bind(&upload_time)
    .bind(&storage_path)
    .bind(&content_hash)
    .execute(&state.pool)
    .await?;

    // Deduplicate: if migration already created a blob with the same content_hash,
    // reassign photo references to this (authoritative) synced blob and delete the duplicate.
    if let Some(ref ch) = content_hash {
        let dupes: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM blobs WHERE content_hash = ? AND user_id = ? AND id != ?",
        )
        .bind(ch)
        .bind(&effective_user_id)
        .bind(&blob_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        for (dupe_id,) in &dupes {
            sqlx::query("UPDATE photos SET encrypted_blob_id = ? WHERE encrypted_blob_id = ?")
                .bind(&blob_id)
                .bind(dupe_id)
                .execute(&state.pool)
                .await
                .ok();
            sqlx::query(
                "UPDATE photos SET encrypted_thumb_blob_id = ? WHERE encrypted_thumb_blob_id = ?",
            )
            .bind(&blob_id)
            .bind(dupe_id)
            .execute(&state.pool)
            .await
            .ok();
            sqlx::query("DELETE FROM blobs WHERE id = ?")
                .bind(dupe_id)
                .execute(&state.pool)
                .await
                .ok();
            tracing::info!(
                "[BLOB_DEDUP] replaced migration blob {} with synced blob {} (content_hash={})",
                dupe_id,
                blob_id,
                ch
            );
        }
    }

    Ok(StatusCode::OK)
}
