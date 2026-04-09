//! Metadata table synchronization from primary → backup server.
//!
//! Performs a full-state sync (not delta) of lightweight metadata tables:
//! - `edit_copies`          — metadata-only photo edits
//! - `photo_metadata`       — Google Photos import metadata
//! - `shared_albums`        — shared album definitions
//! - `shared_album_members` — album membership
//! - `shared_album_photos`  — album contents

/// Sync all metadata tables to the backup server as a single JSON payload.
///
/// Full-state replacement: the backup upserts all rows and deletes any that
/// no longer exist on the primary.
pub async fn sync_metadata_to_backup(
    pool: &sqlx::SqlitePool,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
) {
    // ── edit_copies ──────────────────────────────────────────────────────
    let edit_copies: Vec<(String, String, String, String, String, String)> =
        match sqlx::query_as(
            "SELECT id, photo_id, user_id, name, edit_metadata, created_at FROM edit_copies",
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("Failed to fetch edit_copies for backup sync: {}", e);
                Vec::new()
            }
        };

    // ── photo_metadata ───────────────────────────────────────────────────
    #[derive(sqlx::FromRow)]
    struct PhotoMetaRow {
        id: String,
        user_id: String,
        photo_id: Option<String>,
        blob_id: Option<String>,
        source: String,
        title: Option<String>,
        description: Option<String>,
        taken_at: Option<String>,
        created_at_src: Option<String>,
        latitude: Option<f64>,
        longitude: Option<f64>,
        altitude: Option<f64>,
        image_views: Option<i64>,
        original_url: Option<String>,
        storage_path: Option<String>,
        imported_at: String,
    }

    let photo_metadata: Vec<PhotoMetaRow> = match sqlx::query_as(
        "SELECT id, user_id, photo_id, blob_id, source, title, description, \
         taken_at, created_at_src, latitude, longitude, altitude, \
         image_views, original_url, storage_path, imported_at \
         FROM photo_metadata",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to fetch photo_metadata for backup sync: {}", e);
            Vec::new()
        }
    };

    // ── shared_albums ────────────────────────────────────────────────────
    let shared_albums: Vec<(String, String, String, String)> = match sqlx::query_as(
        "SELECT id, owner_user_id, name, created_at FROM shared_albums",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to fetch shared_albums for backup sync: {}", e);
            Vec::new()
        }
    };

    // ── shared_album_members ─────────────────────────────────────────────
    let shared_members: Vec<(String, String, String, String)> = match sqlx::query_as(
        "SELECT id, album_id, user_id, added_at FROM shared_album_members",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to fetch shared_album_members for backup sync: {}", e);
            Vec::new()
        }
    };

    // ── shared_album_photos ──────────────────────────────────────────────
    let shared_photos: Vec<(String, String, String, String, String)> = match sqlx::query_as(
        "SELECT id, album_id, photo_ref, ref_type, added_at FROM shared_album_photos",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to fetch shared_album_photos for backup sync: {}", e);
            Vec::new()
        }
    };

    // ── Build JSON payload ───────────────────────────────────────────────

    let edit_copies_json: Vec<serde_json::Value> = edit_copies
        .iter()
        .map(|(id, photo_id, user_id, name, edit_metadata, created_at)| {
            serde_json::json!({
                "id": id,
                "photo_id": photo_id,
                "user_id": user_id,
                "name": name,
                "edit_metadata": edit_metadata,
                "created_at": created_at,
            })
        })
        .collect();

    let photo_metadata_json: Vec<serde_json::Value> = photo_metadata
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "user_id": m.user_id,
                "photo_id": m.photo_id,
                "blob_id": m.blob_id,
                "source": m.source,
                "title": m.title,
                "description": m.description,
                "taken_at": m.taken_at,
                "created_at_src": m.created_at_src,
                "latitude": m.latitude,
                "longitude": m.longitude,
                "altitude": m.altitude,
                "image_views": m.image_views,
                "original_url": m.original_url,
                "storage_path": m.storage_path,
                "imported_at": m.imported_at,
            })
        })
        .collect();

    let shared_albums_json: Vec<serde_json::Value> = shared_albums
        .iter()
        .map(|(id, owner_user_id, name, created_at)| {
            serde_json::json!({
                "id": id,
                "owner_user_id": owner_user_id,
                "name": name,
                "created_at": created_at,
            })
        })
        .collect();

    let shared_members_json: Vec<serde_json::Value> = shared_members
        .iter()
        .map(|(id, album_id, user_id, added_at)| {
            serde_json::json!({
                "id": id,
                "album_id": album_id,
                "user_id": user_id,
                "added_at": added_at,
            })
        })
        .collect();

    let shared_photos_json: Vec<serde_json::Value> = shared_photos
        .iter()
        .map(|(id, album_id, photo_ref, ref_type, added_at)| {
            serde_json::json!({
                "id": id,
                "album_id": album_id,
                "photo_ref": photo_ref,
                "ref_type": ref_type,
                "added_at": added_at,
            })
        })
        .collect();

    // ── photo_states (is_favorite, crop_metadata) ────────────────────────
    // Photos are delta-synced by ID: once a photo lands on the backup its
    // file is never re-sent.  Mutable fields like is_favorite and
    // crop_metadata therefore need their own full-state sync channel.
    let photo_states: Vec<(String, bool, Option<String>, Option<String>, Option<String>)> = match sqlx::query_as(
        "SELECT id, is_favorite, crop_metadata, encrypted_blob_id, encrypted_thumb_blob_id FROM photos",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to fetch photo states for backup sync: {}", e);
            Vec::new()
        }
    };

    let photo_states_json: Vec<serde_json::Value> = photo_states
        .iter()
        .map(|(id, is_fav, crop, enc_blob, enc_thumb)| {
            serde_json::json!({
                "id": id,
                "is_favorite": *is_fav,
                "crop_metadata": crop,
                "encrypted_blob_id": enc_blob,
                "encrypted_thumb_blob_id": enc_thumb,
            })
        })
        .collect();

    let body = serde_json::json!({
        "edit_copies": edit_copies_json,
        "photo_metadata": photo_metadata_json,
        "shared_albums": shared_albums_json,
        "shared_album_members": shared_members_json,
        "shared_album_photos": shared_photos_json,
        "photo_states": photo_states_json,
    });

    let mut req = client
        .post(format!("{}/backup/sync-metadata", base_url))
        .json(&body);
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                "Synced metadata to backup: {} edit_copies, {} photo_metadata, \
                 {} shared_albums, {} members, {} album_photos, {} photo_states",
                edit_copies.len(),
                photo_metadata.len(),
                shared_albums.len(),
                shared_members.len(),
                shared_photos.len(),
                photo_states.len(),
            );
        }
        Ok(resp) => {
            tracing::warn!(
                "sync-metadata returned HTTP {}",
                resp.status()
            );
        }
        Err(e) => {
            tracing::warn!("sync-metadata request failed: {}", e);
        }
    }
}
