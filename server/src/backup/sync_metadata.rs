//! Metadata table synchronization from primary → backup server.
//!
//! Performs a full-state sync (not delta) of lightweight metadata tables:
//! - `edit_copies`          — metadata-only photo edits
//! - `photo_metadata`       — Google Photos import metadata
//! - `shared_albums`        — shared album definitions
//! - `shared_album_members` — album membership
//! - `shared_album_photos`  — album contents
//! - `photo_tags`           — user-applied tags on photos
//! - `face_clusters`        — AI face clusters (one per identity)
//! - `face_detections`      — AI per-photo face detections
//! - `object_detections`    — AI per-photo object detections
//! - `ai_processed_photos`  — AI processing watermark
//! - `user_settings`        — per-user preferences (incl. AI/geo toggles)
//!
//! Plus extended `photo_states` covering all mutable photo columns:
//! is_favorite, crop_metadata, encrypted_blob_id, encrypted_thumb_blob_id,
//! geo_city/state/country/country_code, photo_year/photo_month,
//! photo_subtype, burst_id, motion_video_blob_id, and the full extended
//! EXIF column set (camera_make, lens_model, iso_speed, f_number, etc.).

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
    let edit_copies: Vec<(String, String, String, String, String, String)> = match sqlx::query_as(
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
    let shared_albums: Vec<(String, String, String, String)> =
        match sqlx::query_as("SELECT id, owner_user_id, name, created_at FROM shared_albums")
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
    let shared_members: Vec<(String, String, String, String)> =
        match sqlx::query_as("SELECT id, album_id, user_id, added_at FROM shared_album_members")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    "Failed to fetch shared_album_members for backup sync: {}",
                    e
                );
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

    // ── photo_states (extended mutable photo columns) ────────────────────
    // Photos are delta-synced by ID: once a photo lands on the backup its
    // file is never re-sent.  Mutable fields therefore need their own
    // full-state sync channel.  This now includes geolocation cache,
    // year/month bucketing, subtype detection, motion-photo blob link,
    // and the full extended EXIF column set so the backup is a true 1:1
    // replica usable for disaster recovery.
    #[derive(sqlx::FromRow)]
    struct PhotoStateRow {
        id: String,
        is_favorite: bool,
        crop_metadata: Option<String>,
        encrypted_blob_id: Option<String>,
        encrypted_thumb_blob_id: Option<String>,
        // Geolocation cache (017/018)
        geo_city: Option<String>,
        geo_state: Option<String>,
        geo_country: Option<String>,
        geo_country_code: Option<String>,
        photo_year: Option<i64>,
        photo_month: Option<i64>,
        // Subtype detection (016)
        photo_subtype: Option<String>,
        burst_id: Option<String>,
        motion_video_blob_id: Option<String>,
        // Extended EXIF (019)
        camera_make: Option<String>,
        lens_model: Option<String>,
        iso_speed: Option<i64>,
        f_number: Option<f64>,
        exposure_time: Option<String>,
        focal_length: Option<f64>,
        flash: Option<String>,
        white_balance: Option<String>,
        exposure_program: Option<String>,
        metering_mode: Option<String>,
        orientation: Option<i64>,
        software: Option<String>,
        artist: Option<String>,
        copyright: Option<String>,
        description: Option<String>,
        user_comment: Option<String>,
        color_space: Option<String>,
        exposure_bias: Option<f64>,
        scene_type: Option<String>,
        digital_zoom: Option<f64>,
        exif_overrides: Option<String>,
    }

    let photo_states: Vec<PhotoStateRow> = match sqlx::query_as(
        "SELECT id, is_favorite, crop_metadata, encrypted_blob_id, encrypted_thumb_blob_id, \
                geo_city, geo_state, geo_country, geo_country_code, photo_year, photo_month, \
                photo_subtype, burst_id, motion_video_blob_id, \
                camera_make, lens_model, iso_speed, f_number, exposure_time, focal_length, \
                flash, white_balance, exposure_program, metering_mode, orientation, \
                software, artist, copyright, description, user_comment, color_space, \
                exposure_bias, scene_type, digital_zoom, exif_overrides \
         FROM photos",
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
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "is_favorite": p.is_favorite,
                "crop_metadata": p.crop_metadata,
                "encrypted_blob_id": p.encrypted_blob_id,
                "encrypted_thumb_blob_id": p.encrypted_thumb_blob_id,
                "geo_city": p.geo_city,
                "geo_state": p.geo_state,
                "geo_country": p.geo_country,
                "geo_country_code": p.geo_country_code,
                "photo_year": p.photo_year,
                "photo_month": p.photo_month,
                "photo_subtype": p.photo_subtype,
                "burst_id": p.burst_id,
                "motion_video_blob_id": p.motion_video_blob_id,
                "camera_make": p.camera_make,
                "lens_model": p.lens_model,
                "iso_speed": p.iso_speed,
                "f_number": p.f_number,
                "exposure_time": p.exposure_time,
                "focal_length": p.focal_length,
                "flash": p.flash,
                "white_balance": p.white_balance,
                "exposure_program": p.exposure_program,
                "metering_mode": p.metering_mode,
                "orientation": p.orientation,
                "software": p.software,
                "artist": p.artist,
                "copyright": p.copyright,
                "description": p.description,
                "user_comment": p.user_comment,
                "color_space": p.color_space,
                "exposure_bias": p.exposure_bias,
                "scene_type": p.scene_type,
                "digital_zoom": p.digital_zoom,
                "exif_overrides": p.exif_overrides,
            })
        })
        .collect();

    // ── photo_tags ───────────────────────────────────────────────────────
    // User-applied tags. Initial photo transfer carries tags via X-Tags
    // header but later edits (add/remove tag) need full-state sync.
    let photo_tags: Vec<(String, String, String, String)> =
        match sqlx::query_as("SELECT photo_id, user_id, tag, created_at FROM photo_tags")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("Failed to fetch photo_tags for backup sync: {}", e);
                Vec::new()
            }
        };
    let photo_tags_json: Vec<serde_json::Value> = photo_tags
        .iter()
        .map(|(photo_id, user_id, tag, created_at)| {
            serde_json::json!({
                "photo_id": photo_id,
                "user_id": user_id,
                "tag": tag,
                "created_at": created_at,
            })
        })
        .collect();

    // ── face_clusters (AI) ───────────────────────────────────────────────
    #[derive(sqlx::FromRow)]
    struct FaceClusterRow {
        id: i64,
        user_id: String,
        label: Option<String>,
        representative: Option<String>,
        photo_count: i64,
        created_at: String,
        updated_at: String,
    }
    let face_clusters: Vec<FaceClusterRow> = match sqlx::query_as(
        "SELECT id, user_id, label, representative, photo_count, created_at, updated_at \
         FROM face_clusters",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to fetch face_clusters for backup sync: {}", e);
            Vec::new()
        }
    };
    let face_clusters_json: Vec<serde_json::Value> = face_clusters
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "user_id": c.user_id,
                "label": c.label,
                "representative": c.representative,
                "photo_count": c.photo_count,
                "created_at": c.created_at,
                "updated_at": c.updated_at,
            })
        })
        .collect();

    // ── face_detections (AI) ─────────────────────────────────────────────
    #[derive(sqlx::FromRow)]
    struct FaceDetRow {
        id: i64,
        photo_id: String,
        user_id: String,
        cluster_id: Option<i64>,
        bbox_x: f64,
        bbox_y: f64,
        bbox_w: f64,
        bbox_h: f64,
        confidence: f64,
        embedding: Option<Vec<u8>>,
        created_at: String,
    }
    let face_detections: Vec<FaceDetRow> = match sqlx::query_as(
        "SELECT id, photo_id, user_id, cluster_id, bbox_x, bbox_y, bbox_w, bbox_h, \
                confidence, embedding, created_at FROM face_detections",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to fetch face_detections for backup sync: {}", e);
            Vec::new()
        }
    };
    let face_detections_json: Vec<serde_json::Value> = face_detections
        .iter()
        .map(|d| {
            // Encode embedding bytes as base64 for JSON transport.
            let embedding_b64 = d.embedding.as_ref().map(|b| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(b)
            });
            serde_json::json!({
                "id": d.id,
                "photo_id": d.photo_id,
                "user_id": d.user_id,
                "cluster_id": d.cluster_id,
                "bbox_x": d.bbox_x,
                "bbox_y": d.bbox_y,
                "bbox_w": d.bbox_w,
                "bbox_h": d.bbox_h,
                "confidence": d.confidence,
                "embedding_b64": embedding_b64,
                "created_at": d.created_at,
            })
        })
        .collect();

    // ── object_detections (AI) ───────────────────────────────────────────
    #[derive(sqlx::FromRow)]
    struct ObjectDetRow {
        id: i64,
        photo_id: String,
        user_id: String,
        class_name: String,
        confidence: f64,
        bbox_x: f64,
        bbox_y: f64,
        bbox_w: f64,
        bbox_h: f64,
        created_at: String,
    }
    let object_detections: Vec<ObjectDetRow> = match sqlx::query_as(
        "SELECT id, photo_id, user_id, class_name, confidence, bbox_x, bbox_y, \
                bbox_w, bbox_h, created_at FROM object_detections",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Failed to fetch object_detections for backup sync: {}", e);
            Vec::new()
        }
    };
    let object_detections_json: Vec<serde_json::Value> = object_detections
        .iter()
        .map(|o| {
            serde_json::json!({
                "id": o.id,
                "photo_id": o.photo_id,
                "user_id": o.user_id,
                "class_name": o.class_name,
                "confidence": o.confidence,
                "bbox_x": o.bbox_x,
                "bbox_y": o.bbox_y,
                "bbox_w": o.bbox_w,
                "bbox_h": o.bbox_h,
                "created_at": o.created_at,
            })
        })
        .collect();

    // ── ai_processed_photos ──────────────────────────────────────────────
    let ai_processed: Vec<(String, String, String)> =
        match sqlx::query_as("SELECT photo_id, user_id, processed_at FROM ai_processed_photos")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("Failed to fetch ai_processed_photos for backup sync: {}", e);
                Vec::new()
            }
        };
    let ai_processed_json: Vec<serde_json::Value> = ai_processed
        .iter()
        .map(|(photo_id, user_id, processed_at)| {
            serde_json::json!({
                "photo_id": photo_id,
                "user_id": user_id,
                "processed_at": processed_at,
            })
        })
        .collect();

    // ── user_settings ────────────────────────────────────────────────────
    // Per-user preferences shared by the AI and Geo modules.
    let user_settings: Vec<(String, String, String, String)> =
        match sqlx::query_as("SELECT user_id, key, value, updated_at FROM user_settings")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("Failed to fetch user_settings for backup sync: {}", e);
                Vec::new()
            }
        };
    let user_settings_json: Vec<serde_json::Value> = user_settings
        .iter()
        .map(|(user_id, key, value, updated_at)| {
            serde_json::json!({
                "user_id": user_id,
                "key": key,
                "value": value,
                "updated_at": updated_at,
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
        "photo_tags": photo_tags_json,
        "face_clusters": face_clusters_json,
        "face_detections": face_detections_json,
        "object_detections": object_detections_json,
        "ai_processed_photos": ai_processed_json,
        "user_settings": user_settings_json,
    });

    let mut req = client
        .post(format!("{base_url}/backup/sync-metadata"))
        .json(&body);
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                "Synced metadata to backup: {} edit_copies, {} photo_metadata, \
                 {} shared_albums, {} members, {} album_photos, {} photo_states, \
                 {} photo_tags, {} face_clusters, {} face_detections, \
                 {} object_detections, {} ai_processed, {} user_settings",
                edit_copies.len(),
                photo_metadata.len(),
                shared_albums.len(),
                shared_members.len(),
                shared_photos.len(),
                photo_states.len(),
                photo_tags.len(),
                face_clusters.len(),
                face_detections.len(),
                object_detections.len(),
                ai_processed.len(),
                user_settings.len(),
            );
        }
        Ok(resp) => {
            tracing::warn!("sync-metadata returned HTTP {}", resp.status());
        }
        Err(e) => {
            tracing::warn!("sync-metadata request failed: {}", e);
        }
    }
}
