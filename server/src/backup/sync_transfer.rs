//! Low-level file transfer and sync-log helpers shared by the sync engine.
//!
//! Extracted from the monolithic `sync.rs` to keep the orchestration logic
//! (delta sync, phase ordering) separate from the I/O plumbing (HTTP
//! requests, SHA-256 checksums, log updates).

use std::collections::HashSet;

use chrono::Utc;
use percent_encoding::{utf8_percent_encode, CONTROLS};
use sha2::{Digest, Sha256};

// ── Sync data models ─────────────────────────────────────────────────────────

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
}

// ── Header building ──────────────────────────────────────────────────────────

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
        headers.push(("X-Taken-At".to_string(), v.clone()));
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
pub fn build_photo_headers(
    photo: &PhotoToSync,
    tags: &[String],
) -> Vec<(String, String)> {
    let mut headers = vec![
        ("X-User-Id".to_string(), photo.user_id.clone()),
        (
            "X-Original-Created-At".to_string(),
            photo.created_at.clone(),
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

    headers
}

/// Build the full set of metadata headers for a trash item transfer.
pub fn build_trash_headers(item: &TrashToSync) -> Vec<(String, String)> {
    let mut headers = vec![
        ("X-User-Id".to_string(), item.user_id.clone()),
        ("X-Original-Created-At".to_string(), item.deleted_at.clone()),
        ("X-Deleted-At".to_string(), item.deleted_at.clone()),
        ("X-Expires-At".to_string(), item.expires_at.clone()),
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
    let mut req = client.get(format!("{}{}", base_url, path));
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
        .map_err(|e| format!("read error: {}", e))?;

    // Compute SHA-256 checksum for integrity verification
    let hash = Sha256::digest(&file_data);
    let hash_hex = hex::encode(hash);

    // Percent-encode non-ASCII characters in file_path so the header
    // value stays within visible-ASCII (RFC 7230 §3.2.6).  The receiver
    // will percent-decode before using the path.
    let encoded_path = utf8_percent_encode(file_path, CONTROLS).to_string();

    let mut req = client
        .post(format!("{}/backup/receive", base_url))
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
