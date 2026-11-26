//! Low-level file transfer and sync-log helpers shared by the sync engine.
//!
//! Extracted from the monolithic `sync.rs` to keep the orchestration logic
//! (delta sync, phase ordering) separate from the I/O plumbing (HTTP
//! requests, SHA-256 checksums, log updates).

use std::collections::HashSet;

use chrono::Utc;
use percent_encoding::{utf8_percent_encode, CONTROLS};
use sha2::{Digest, Sha256};

use crate::photos::utils::normalize_iso_timestamp;

use super::models::{
    EncryptedPhotoRow, EncryptedTrashRow, RowBatch, RowBatchResult, MAX_ROWS_PER_BATCH,
};

// ── Sync data models ─────────────────────────────────────────────────────────

/// All metadata columns needed to faithfully replicate a blob entry.
#[derive(Debug, sqlx::FromRow)]
pub struct BlobToSync {
    pub id: String,
    pub user_id: String,
    pub blob_type: String,
    pub size_bytes: i64,
    pub client_hash: Option<String>,
    pub upload_time: String,
    pub storage_path: String,
    pub content_hash: Option<String>,
}

/// All metadata columns needed to faithfully replicate a photo entry.
#[derive(Debug, sqlx::FromRow)]
pub struct PhotoToSync {
    pub id: String,
    pub user_id: String,
    pub filename: String,
    pub file_path: String,
    pub mime_type: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub camera_model: Option<String>,
    pub is_favorite: bool,
    pub photo_hash: Option<String>,
    pub crop_metadata: Option<String>,
    pub created_at: String,
    pub encrypted_blob_id: Option<String>,
    pub encrypted_thumb_blob_id: Option<String>,
}

/// All metadata columns needed to faithfully replicate a trash item.
#[derive(Debug, sqlx::FromRow)]
pub struct TrashToSync {
    pub id: String,
    /// The original photo UUID — different from `id` (the trash row UUID).
    /// Used by the backup to remove the item from its own `photos` table.
    pub photo_id: String,
    pub user_id: String,
    pub filename: String,
    pub file_path: String,
    pub mime_type: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub camera_model: Option<String>,
    pub is_favorite: bool,
    pub photo_hash: Option<String>,
    pub crop_metadata: Option<String>,
    pub deleted_at: String,
    pub expires_at: String,
    pub encrypted_blob_id: Option<String>,
    pub encrypted_thumb_blob_id: Option<String>,
}

/// A row with no plaintext file: its bytes are a blob, so it travels as a
/// row (`receive-*-rows`), never through `send_file` (TODO.md F9).
pub fn is_row_only(file_path: &str) -> bool {
    file_path.trim().is_empty()
}

impl PhotoToSync {
    pub fn to_row(&self, tags: Vec<String>) -> EncryptedPhotoRow {
        EncryptedPhotoRow {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            filename: self.filename.clone(),
            mime_type: self.mime_type.clone(),
            media_type: self.media_type.clone(),
            size_bytes: self.size_bytes,
            width: self.width,
            height: self.height,
            duration_secs: self.duration_secs,
            taken_at: self.taken_at.clone(),
            latitude: self.latitude,
            longitude: self.longitude,
            camera_model: self.camera_model.clone(),
            is_favorite: self.is_favorite,
            photo_hash: self.photo_hash.clone(),
            crop_metadata: self.crop_metadata.clone(),
            created_at: self.created_at.clone(),
            encrypted_blob_id: self.encrypted_blob_id.clone(),
            encrypted_thumb_blob_id: self.encrypted_thumb_blob_id.clone(),
            tags,
        }
    }
}

impl TrashToSync {
    pub fn to_row(&self) -> EncryptedTrashRow {
        EncryptedTrashRow {
            id: self.id.clone(),
            photo_id: self.photo_id.clone(),
            user_id: self.user_id.clone(),
            filename: self.filename.clone(),
            mime_type: self.mime_type.clone(),
            media_type: self.media_type.clone(),
            size_bytes: self.size_bytes,
            width: self.width,
            height: self.height,
            duration_secs: self.duration_secs,
            taken_at: self.taken_at.clone(),
            latitude: self.latitude,
            longitude: self.longitude,
            camera_model: self.camera_model.clone(),
            is_favorite: self.is_favorite,
            photo_hash: self.photo_hash.clone(),
            crop_metadata: self.crop_metadata.clone(),
            deleted_at: self.deleted_at.clone(),
            expires_at: self.expires_at.clone(),
            encrypted_blob_id: self.encrypted_blob_id.clone(),
            encrypted_thumb_blob_id: self.encrypted_thumb_blob_id.clone(),
        }
    }
}

// ── Header building ──────────────────────────────────────────────────────────

/// Build metadata headers for a blob transfer.
pub fn build_blob_headers(blob: &BlobToSync) -> Vec<(String, String)> {
    let mut headers = vec![
        ("X-User-Id".to_string(), blob.user_id.clone()),
        ("X-Blob-Type".to_string(), blob.blob_type.clone()),
        ("X-Upload-Time".to_string(), blob.upload_time.clone()),
        ("X-Size-Bytes".to_string(), blob.size_bytes.to_string()),
    ];
    if let Some(ref v) = blob.client_hash {
        headers.push(("X-Client-Hash".to_string(), v.clone()));
    }
    if let Some(ref v) = blob.content_hash {
        headers.push(("X-Original-Content-Hash".to_string(), v.clone()));
    }
    headers
}

/// Build the metadata headers that are common to both photo and trash
/// transfers: optional fields like taken_at, GPS, camera model, etc.
///
/// Returns a `Vec<(header-name, header-value)>` ready to attach to a request.
fn push_common_optional_headers(
    headers: &mut Vec<(String, String)>,
    taken_at: &Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    duration_secs: Option<f64>,
    camera_model: &Option<String>,
    photo_hash: &Option<String>,
    crop_metadata: &Option<String>,
) {
    if let Some(ref v) = taken_at {
        headers.push(("X-Taken-At".to_string(), normalize_iso_timestamp(v)));
    }
    if let Some(v) = latitude {
        headers.push(("X-Latitude".to_string(), v.to_string()));
    }
    if let Some(v) = longitude {
        headers.push(("X-Longitude".to_string(), v.to_string()));
    }
    if let Some(v) = duration_secs {
        headers.push(("X-Duration-Secs".to_string(), v.to_string()));
    }
    if let Some(ref v) = camera_model {
        headers.push((
            "X-Camera-Model".to_string(),
            utf8_percent_encode(v, CONTROLS).to_string(),
        ));
    }
    if let Some(ref v) = photo_hash {
        headers.push(("X-Photo-Hash".to_string(), v.clone()));
    }
    if let Some(ref v) = crop_metadata {
        headers.push((
            "X-Crop-Metadata".to_string(),
            utf8_percent_encode(v, CONTROLS).to_string(),
        ));
    }
}

/// Build the full set of metadata headers for a photo transfer.
pub fn build_photo_headers(photo: &PhotoToSync, tags: &[String]) -> Vec<(String, String)> {
    let mut headers = vec![
        ("X-User-Id".to_string(), photo.user_id.clone()),
        (
            "X-Original-Created-At".to_string(),
            normalize_iso_timestamp(&photo.created_at),
        ),
        (
            "X-Filename".to_string(),
            utf8_percent_encode(&photo.filename, CONTROLS).to_string(),
        ),
        ("X-Mime-Type".to_string(), photo.mime_type.clone()),
        ("X-Media-Type".to_string(), photo.media_type.clone()),
        ("X-Width".to_string(), photo.width.to_string()),
        ("X-Height".to_string(), photo.height.to_string()),
        (
            "X-Is-Favorite".to_string(),
            if photo.is_favorite { "1" } else { "0" }.to_string(),
        ),
    ];

    push_common_optional_headers(
        &mut headers,
        &photo.taken_at,
        photo.latitude,
        photo.longitude,
        photo.duration_secs,
        &photo.camera_model,
        &photo.photo_hash,
        &photo.crop_metadata,
    );

    if !tags.is_empty() {
        let tags_str = tags
            .iter()
            .map(|t| utf8_percent_encode(t, CONTROLS).to_string())
            .collect::<Vec<_>>()
            .join(",");
        headers.push(("X-Tags".to_string(), tags_str));
    }

    if let Some(ref ebi) = photo.encrypted_blob_id {
        headers.push(("X-Encrypted-Blob-Id".to_string(), ebi.clone()));
    }
    if let Some(ref etbi) = photo.encrypted_thumb_blob_id {
        headers.push(("X-Encrypted-Thumb-Blob-Id".to_string(), etbi.clone()));
    }

    headers
}

/// Build the full set of metadata headers for a trash item transfer.
pub fn build_trash_headers(item: &TrashToSync) -> Vec<(String, String)> {
    let mut headers = vec![
        ("X-User-Id".to_string(), item.user_id.clone()),
        (
            "X-Original-Created-At".to_string(),
            normalize_iso_timestamp(&item.deleted_at),
        ),
        (
            "X-Deleted-At".to_string(),
            normalize_iso_timestamp(&item.deleted_at),
        ),
        (
            "X-Expires-At".to_string(),
            normalize_iso_timestamp(&item.expires_at),
        ),
        ("X-Original-Photo-Id".to_string(), item.photo_id.clone()),
        (
            "X-Filename".to_string(),
            utf8_percent_encode(&item.filename, CONTROLS).to_string(),
        ),
        ("X-Mime-Type".to_string(), item.mime_type.clone()),
        ("X-Media-Type".to_string(), item.media_type.clone()),
        ("X-Width".to_string(), item.width.to_string()),
        ("X-Height".to_string(), item.height.to_string()),
        (
            "X-Is-Favorite".to_string(),
            if item.is_favorite { "1" } else { "0" }.to_string(),
        ),
    ];

    push_common_optional_headers(
        &mut headers,
        &item.taken_at,
        item.latitude,
        item.longitude,
        item.duration_secs,
        &item.camera_model,
        &item.photo_hash,
        &item.crop_metadata,
    );

    headers
}

// ── Network helpers ──────────────────────────────────────────────────────────

/// Fetch the set of IDs the remote backup already has for a given list endpoint.
/// Returns an empty set on any error (graceful degradation to full sync).
pub async fn fetch_remote_ids(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    api_key: &Option<String>,
) -> HashSet<String> {
    let mut req = client.get(format!("{base_url}{path}"));
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            #[derive(serde::Deserialize)]
            struct IdOnly {
                id: String,
            }
            match resp.json::<Vec<IdOnly>>().await {
                Ok(items) => {
                    tracing::info!(
                        path = %path,
                        count = items.len(),
                        "Fetched remote ID list successfully"
                    );
                    items.into_iter().map(|i| i.id).collect()
                }
                Err(e) => {
                    tracing::warn!("Failed to parse remote ID list from {}: {}", path, e);
                    HashSet::new()
                }
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::error!(
                path = %path,
                status = %status,
                body = %body,
                "Remote ID list fetch failed — this likely means the backup is unreachable or auth failed"
            );
            tracing::warn!(
                "Remote {} returned HTTP {} — falling back to full sync",
                path,
                status
            );
            HashSet::new()
        }
        Err(e) => {
            tracing::warn!(
                "Failed to fetch remote IDs from {}: {} — falling back to full sync",
                path,
                e
            );
            HashSet::new()
        }
    }
}

/// Read a file from disk, compute its SHA-256 checksum, and POST it to the
/// backup server with integrity headers. Returns `Ok(())` on success or
/// `Err(description)` on failure.
pub async fn send_file(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    storage_root: &std::path::Path,
    item_id: &str,
    file_path: &str,
    source: &str,
    extra_headers: &[(String, String)],
) -> Result<(), String> {
    let full_path = storage_root.join(file_path);
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err("file not found on disk".to_string());
    }

    let file_data = tokio::fs::read(&full_path)
        .await
        .map_err(|e| format!("read error: {e}"))?;

    // Compute SHA-256 checksum for integrity verification
    let hash = Sha256::digest(&file_data);
    let hash_hex = hex::encode(hash);

    // Percent-encode non-ASCII characters in file_path so the header
    // value stays within visible-ASCII (RFC 7230 §3.2.6).  The receiver
    // will percent-decode before using the path.
    let encoded_path = utf8_percent_encode(file_path, CONTROLS).to_string();

    let mut req = client
        .post(format!("{base_url}/backup/receive"))
        .header("X-Photo-Id", item_id)
        .header("X-File-Path", encoded_path.as_str())
        .header("X-Source", source)
        .header("X-Content-Hash", hash_hex.as_str())
        .body(file_data);

    // Attach all metadata headers so the backup stores a faithful replica
    for (name, value) in extra_headers {
        if let Ok(hv) = reqwest::header::HeaderValue::from_str(value) {
            req = req.header(name.as_str(), hv);
        }
    }

    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => Ok(()),
        Ok(resp) => Err(format!("HTTP {}", resp.status())),
        Err(e) => Err(e.to_string()),
    }
}

/// Send a blob file to the backup's `/backup/receive-blob` endpoint.
/// Similar to [`send_file`] but targets the blob receive endpoint and
/// uses `X-Blob-Id` / `X-Storage-Path` headers instead of photo headers.
pub async fn send_blob_to_backup(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    storage_root: &std::path::Path,
    blob: &BlobToSync,
    extra_headers: &[(String, String)],
) -> Result<(), String> {
    let full_path = storage_root.join(&blob.storage_path);
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err("blob file not found on disk".to_string());
    }

    let file_data = tokio::fs::read(&full_path)
        .await
        .map_err(|e| format!("read error: {e}"))?;

    let hash = Sha256::digest(&file_data);
    let hash_hex = hex::encode(hash);

    let encoded_path = utf8_percent_encode(&blob.storage_path, CONTROLS).to_string();

    let mut req = client
        .post(format!("{base_url}/backup/receive-blob"))
        .header("X-Blob-Id", blob.id.as_str())
        .header("X-Storage-Path", encoded_path.as_str())
        .header("X-Content-Hash", hash_hex.as_str())
        .body(file_data);

    for (name, value) in extra_headers {
        if let Ok(hv) = reqwest::header::HeaderValue::from_str(value) {
            req = req.header(name.as_str(), hv);
        }
    }

    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => Ok(()),
        Ok(resp) => Err(format!("HTTP {}", resp.status())),
        Err(e) => Err(e.to_string()),
    }
}

// ── Row-only transfer (F9) ───────────────────────────────────────────────────

/// Send rows to `{base_url}{path}` in batches of [`MAX_ROWS_PER_BATCH`] and
/// return one outcome per row, in input order: `None` = stored, `Some(err)` =
/// not stored. Never fails as a whole — a transport error fails its batch.
pub async fn send_rows<T: serde::Serialize + Clone>(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    path: &str,
    rows: &[(String, T)],
) -> Vec<(String, Option<String>)> {
    let mut out = Vec::with_capacity(rows.len());
    for chunk in rows.chunks(MAX_ROWS_PER_BATCH) {
        let ids: Vec<String> = chunk.iter().map(|(id, _)| id.clone()).collect();
        let batch = RowBatch {
            rows: chunk.iter().map(|(_, r)| r.clone()).collect::<Vec<T>>(),
        };
        let mut req = client.post(format!("{base_url}{path}")).json(&batch);
        if let Some(ref key) = api_key {
            req = req.header("X-API-Key", key.as_str());
        }
        let response = match req.send().await {
            Ok(resp) if resp.status().is_success() => resp
                .json::<RowBatchResult>()
                .await
                .map_err(|e| format!("unreadable reply from {path}: {e}")),
            Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => Err(format!(
                "backup server has no {path} (older than this primary — upgrade it)"
            )),
            Ok(resp) => Err(format!("HTTP {} from {path}", resp.status())),
            Err(e) => Err(format!("{path}: {e}")),
        };
        out.extend(reconcile_row_results(&ids, response));
    }
    out
}

/// Map a batch reply back onto the ids that were sent. An id the backup
/// neither accepted nor rejected counts as NOT stored — a reply that drops a
/// row must never read as success.
pub fn reconcile_row_results(
    sent: &[String],
    reply: Result<RowBatchResult, String>,
) -> Vec<(String, Option<String>)> {
    match reply {
        Err(e) => sent
            .iter()
            .map(|id| (id.clone(), Some(e.clone())))
            .collect(),
        Ok(res) => {
            let accepted: HashSet<&str> = res.accepted.iter().map(String::as_str).collect();
            let failed: std::collections::HashMap<&str, &str> = res
                .failed
                .iter()
                .map(|f| (f.id.as_str(), f.error.as_str()))
                .collect();
            sent.iter()
                .map(|id| {
                    let outcome = if let Some(err) = failed.get(id.as_str()) {
                        Some((*err).to_string())
                    } else if accepted.contains(id.as_str()) {
                        None
                    } else {
                        Some("backup reply did not mention this row".to_string())
                    };
                    (id.clone(), outcome)
                })
                .collect()
        }
    }
}

// ── Sync log helpers ─────────────────────────────────────────────────────────

/// Update the backup_sync_log row with final status and counters.
pub async fn update_sync_log(
    pool: &sqlx::SqlitePool,
    log_id: &str,
    status: &str,
    photos_synced: i64,
    bytes_synced: i64,
    error: Option<&str>,
) {
    let now = Utc::now().to_rfc3339();
    if let Err(e) = sqlx::query(
        "UPDATE backup_sync_log SET completed_at = ?, status = ?, photos_synced = ?, \
         bytes_synced = ?, error = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(status)
    .bind(photos_synced)
    .bind(bytes_synced)
    .bind(error)
    .bind(log_id)
    .execute(pool)
    .await
    {
        tracing::error!(log_id = log_id, error = %e, "Failed to update backup sync log — row stuck in 'running' state");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::models::RowFailure;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_and_blank_paths_are_row_only() {
        assert!(is_row_only(""));
        assert!(is_row_only("  "));
        assert!(!is_row_only("uploads/a.jpg"));
    }

    #[test]
    fn reconcile_maps_accepted_and_failed_in_send_order() {
        let reply = RowBatchResult {
            accepted: ids(&["c", "a"]),
            failed: vec![RowFailure {
                id: "b".into(),
                error: "hash conflict".into(),
            }],
        };
        let out = reconcile_row_results(&ids(&["a", "b", "c"]), Ok(reply));
        assert_eq!(
            out,
            vec![
                ("a".to_string(), None),
                ("b".to_string(), Some("hash conflict".to_string())),
                ("c".to_string(), None),
            ]
        );
    }

    #[test]
    fn reconcile_counts_a_row_missing_from_the_reply_as_not_stored() {
        let reply = RowBatchResult {
            accepted: ids(&["a"]),
            failed: vec![],
        };
        let out = reconcile_row_results(&ids(&["a", "silent"]), Ok(reply));
        assert_eq!(out[0], ("a".to_string(), None));
        assert!(out[1].1.is_some(), "a dropped row must not read as success");
    }

    #[test]
    fn reconcile_fails_every_row_of_a_failed_batch() {
        let out = reconcile_row_results(&ids(&["a", "b"]), Err("HTTP 404".into()));
        assert!(out.iter().all(|(_, e)| e.as_deref() == Some("HTTP 404")));
    }

    #[test]
    fn photo_payload_carries_no_path() {
        let json = serde_json::to_value(EncryptedPhotoRow {
            id: "p".into(),
            user_id: "u".into(),
            filename: "f.jpg".into(),
            mime_type: "image/jpeg".into(),
            media_type: "photo".into(),
            size_bytes: 0,
            width: 1,
            height: 1,
            duration_secs: None,
            taken_at: None,
            latitude: None,
            longitude: None,
            camera_model: None,
            is_favorite: false,
            photo_hash: None,
            crop_metadata: None,
            created_at: "x".into(),
            encrypted_blob_id: None,
            encrypted_thumb_blob_id: None,
            tags: vec![],
        })
        .unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert!(
            !keys.iter().any(|k| k.contains("path")),
            "row payload must not carry a path: {keys:?}"
        );
    }
}
