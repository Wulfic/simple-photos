//! Secure gallery metadata synchronization from primary → backup server.
//!
//! Replicates `encrypted_galleries` and `encrypted_gallery_items` rows so the
//! backup server knows which `photos` rows are secure-album clones and can
//! filter them from the regular gallery view (via `encrypted-sync`).

/// Sync all secure gallery metadata to the backup server.
///
/// Sends the full `encrypted_galleries` + `encrypted_gallery_items` tables as
/// a single JSON payload.  The backup upserts all rows and deletes any that
/// no longer exist on the primary (full-state sync, not delta).
pub async fn sync_secure_galleries_to_backup(
    pool: &sqlx::SqlitePool,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
) {
    // Fetch all galleries
    let galleries: Vec<(String, String, String, String, String)> = match sqlx::query_as(
        "SELECT id, user_id, name, password_hash, created_at FROM encrypted_galleries",
    )
    .fetch_all(pool)
    .await
    {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!("Failed to fetch secure galleries for backup sync: {}", e);
            return;
        }
    };

    // Fetch all gallery items, joining photos to get encrypted_blob_id and
    // encrypted_thumb_blob_id for server-side clones (needed by the backup's
    // list_gallery_items COALESCE query and thumbnail display).
    // Use COALESCE to fall back to the egi columns when the clone photos row
    // doesn't exist (e.g. on a backup server running recovery push-sync —
    // clone photos are excluded from sync_photos, so backup has no photos
    // row for the clone, but the egi columns were populated by the primary's
    // earlier gallery sync).
    let items: Vec<(String, String, String, String, Option<String>, Option<String>, Option<String>)> = match sqlx::query_as(
        "SELECT gi.id, gi.gallery_id, gi.blob_id, gi.added_at, gi.original_blob_id, \
                COALESCE(p.encrypted_blob_id, gi.encrypted_blob_id), \
                COALESCE(p.encrypted_thumb_blob_id, gi.encrypted_thumb_blob_id) \
         FROM encrypted_gallery_items gi \
         LEFT JOIN photos p ON p.id = gi.blob_id",
    )
    .fetch_all(pool)
    .await
    {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!("Failed to fetch secure gallery items for backup sync: {}", e);
            return;
        }
    };

    if galleries.is_empty() && items.is_empty() {
        return;
    }

    let galleries_json: Vec<serde_json::Value> = galleries
        .iter()
        .map(|(id, user_id, name, password_hash, created_at)| {
            serde_json::json!({
                "id": id,
                "user_id": user_id,
                "name": name,
                "password_hash": password_hash,
                "created_at": created_at,
            })
        })
        .collect();

    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|(id, gallery_id, blob_id, added_at, original_blob_id, encrypted_blob_id, encrypted_thumb_blob_id)| {
            serde_json::json!({
                "id": id,
                "gallery_id": gallery_id,
                "blob_id": blob_id,
                "added_at": added_at,
                "original_blob_id": original_blob_id,
                "encrypted_blob_id": encrypted_blob_id,
                "encrypted_thumb_blob_id": encrypted_thumb_blob_id,
            })
        })
        .collect();

    let body = serde_json::json!({
        "galleries": galleries_json,
        "items": items_json,
    });

    let mut req = client
        .post(format!("{}/backup/sync-secure-galleries", base_url))
        .json(&body);
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                "Synced {} secure galleries ({} items) to backup",
                galleries.len(),
                items.len()
            );
        }
        Ok(resp) => {
            tracing::warn!(
                "sync-secure-galleries returned HTTP {}",
                resp.status()
            );
        }
        Err(e) => {
            tracing::warn!("sync-secure-galleries request failed: {}", e);
        }
    }
}
