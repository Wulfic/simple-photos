//! Full-state metadata sync endpoint: upserts every metadata table from the
//! primary's payload and prunes rows that no longer exist on the primary.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use crate::error::AppError;
use crate::state::AppState;

use super::validate_api_key;

/// POST /api/backup/sync-metadata
/// Receives the full state of metadata tables from the primary.
/// Full-state sync: upserts all rows, deletes rows no longer on primary.
///
/// Tables synced: edit_copies, photo_metadata, shared_albums,
///                shared_album_members, shared_album_photos
pub async fn backup_sync_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let mut tx = state.pool.begin().await?;

    // ── edit_copies ──────────────────────────────────────────────────────
    let edit_copies = body
        .get("edit_copies")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut ec_ids = std::collections::HashSet::new();
    for row in &edit_copies {
        let id = row["id"].as_str().unwrap_or_default();
        let photo_id = row["photo_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let name = row["name"].as_str().unwrap_or_default();
        let edit_metadata = row["edit_metadata"].as_str().unwrap_or("{}");
        let created_at = row["created_at"].as_str().unwrap_or_default();

        if id.is_empty() || photo_id.is_empty() {
            continue;
        }
        ec_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO edit_copies (id, photo_id, user_id, name, edit_metadata, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, \
               edit_metadata = excluded.edit_metadata",
        )
        .bind(id)
        .bind(photo_id)
        .bind(user_id)
        .bind(name)
        .bind(edit_metadata)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }

    if !ec_ids.is_empty() {
        let existing: Vec<String> = sqlx::query_scalar("SELECT id FROM edit_copies")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default();
        for eid in &existing {
            if !ec_ids.contains(eid) {
                sqlx::query("DELETE FROM edit_copies WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    // ── photo_metadata ───────────────────────────────────────────────────
    let photo_metadata = body
        .get("photo_metadata")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut pm_ids = std::collections::HashSet::new();
    for row in &photo_metadata {
        let id = row["id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let photo_id = row["photo_id"].as_str();
        let blob_id = row["blob_id"].as_str();
        let source = row["source"].as_str().unwrap_or("manual");
        let title = row["title"].as_str();
        let description = row["description"].as_str();
        let taken_at = row["taken_at"].as_str();
        let created_at_src = row["created_at_src"].as_str();
        let latitude = row["latitude"].as_f64();
        let longitude = row["longitude"].as_f64();
        let altitude = row["altitude"].as_f64();
        let image_views = row["image_views"].as_i64();
        let original_url = row["original_url"].as_str();
        let storage_path = row["storage_path"].as_str();
        let imported_at = row["imported_at"].as_str().unwrap_or_default();

        if id.is_empty() {
            continue;
        }
        pm_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO photo_metadata (id, user_id, photo_id, blob_id, source, title, \
             description, taken_at, created_at_src, latitude, longitude, altitude, \
             image_views, original_url, storage_path, imported_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               photo_id = excluded.photo_id, \
               blob_id = excluded.blob_id, \
               title = excluded.title, \
               description = excluded.description, \
               taken_at = excluded.taken_at, \
               latitude = excluded.latitude, \
               longitude = excluded.longitude",
        )
        .bind(id)
        .bind(user_id)
        .bind(photo_id)
        .bind(blob_id)
        .bind(source)
        .bind(title)
        .bind(description)
        .bind(taken_at)
        .bind(created_at_src)
        .bind(latitude)
        .bind(longitude)
        .bind(altitude)
        .bind(image_views)
        .bind(original_url)
        .bind(storage_path)
        .bind(imported_at)
        .execute(&mut *tx)
        .await?;
    }

    if !pm_ids.is_empty() {
        let existing: Vec<String> = sqlx::query_scalar("SELECT id FROM photo_metadata")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default();
        for eid in &existing {
            if !pm_ids.contains(eid) {
                sqlx::query("DELETE FROM photo_metadata WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    // ── shared_albums ────────────────────────────────────────────────────
    let shared_albums = body
        .get("shared_albums")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut sa_ids = std::collections::HashSet::new();
    for row in &shared_albums {
        let id = row["id"].as_str().unwrap_or_default();
        let owner_user_id = row["owner_user_id"].as_str().unwrap_or_default();
        let name = row["name"].as_str().unwrap_or_default();
        let created_at = row["created_at"].as_str().unwrap_or_default();

        if id.is_empty() {
            continue;
        }
        sa_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO shared_albums (id, owner_user_id, name, created_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name",
        )
        .bind(id)
        .bind(owner_user_id)
        .bind(name)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }

    // ── shared_album_members ─────────────────────────────────────────────
    let shared_members = body
        .get("shared_album_members")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut sm_ids = std::collections::HashSet::new();
    for row in &shared_members {
        let id = row["id"].as_str().unwrap_or_default();
        let album_id = row["album_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let added_at = row["added_at"].as_str().unwrap_or_default();

        if id.is_empty() {
            continue;
        }
        sm_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO shared_album_members (id, album_id, user_id, added_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               album_id = excluded.album_id, \
               user_id = excluded.user_id",
        )
        .bind(id)
        .bind(album_id)
        .bind(user_id)
        .bind(added_at)
        .execute(&mut *tx)
        .await?;
    }

    // ── shared_album_photos ──────────────────────────────────────────────
    let shared_photos = body
        .get("shared_album_photos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut sp_ids = std::collections::HashSet::new();
    for row in &shared_photos {
        let id = row["id"].as_str().unwrap_or_default();
        let album_id = row["album_id"].as_str().unwrap_or_default();
        let photo_ref = row["photo_ref"].as_str().unwrap_or_default();
        let ref_type = row["ref_type"].as_str().unwrap_or("photo");
        let added_at = row["added_at"].as_str().unwrap_or_default();

        if id.is_empty() {
            continue;
        }
        sp_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO shared_album_photos (id, album_id, photo_ref, ref_type, added_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               photo_ref = excluded.photo_ref, \
               ref_type = excluded.ref_type",
        )
        .bind(id)
        .bind(album_id)
        .bind(photo_ref)
        .bind(ref_type)
        .bind(added_at)
        .execute(&mut *tx)
        .await?;
    }

    // ── Prune deleted shared data ────────────────────────────────────────
    if !sp_ids.is_empty() {
        let existing: Vec<String> = sqlx::query_scalar("SELECT id FROM shared_album_photos")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default();
        for eid in &existing {
            if !sp_ids.contains(eid) {
                sqlx::query("DELETE FROM shared_album_photos WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    if !sm_ids.is_empty() {
        let existing: Vec<String> = sqlx::query_scalar("SELECT id FROM shared_album_members")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default();
        for eid in &existing {
            if !sm_ids.contains(eid) {
                sqlx::query("DELETE FROM shared_album_members WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    if !sa_ids.is_empty() {
        let existing: Vec<String> = sqlx::query_scalar("SELECT id FROM shared_albums")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default();
        for eid in &existing {
            if !sa_ids.contains(eid) {
                sqlx::query("DELETE FROM shared_albums WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    // ── photo_states (extended mutable photo columns) ────────────────────
    // Photos are delta-synced by file transfer (Phase 1) so mutable fields
    // that change after the initial sync need a separate update channel.
    // Includes geolocation cache, year/month bucketing, subtype detection,
    // motion-photo blob link, and the full extended EXIF column set.
    let photo_states = body
        .get("photo_states")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut ps_updated = 0usize;
    for row in &photo_states {
        let id = row["id"].as_str().unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let is_favorite = row["is_favorite"].as_bool().unwrap_or(false);
        let crop_metadata = row["crop_metadata"].as_str();
        let encrypted_blob_id = row["encrypted_blob_id"].as_str();
        let encrypted_thumb_blob_id = row["encrypted_thumb_blob_id"].as_str();
        let geo_city = row["geo_city"].as_str();
        let geo_state = row["geo_state"].as_str();
        let geo_country = row["geo_country"].as_str();
        let geo_country_code = row["geo_country_code"].as_str();
        let photo_year = row["photo_year"].as_i64();
        let photo_month = row["photo_month"].as_i64();
        let photo_subtype = row["photo_subtype"].as_str();
        let burst_id = row["burst_id"].as_str();
        let motion_video_blob_id = row["motion_video_blob_id"].as_str();
        let camera_make = row["camera_make"].as_str();
        let lens_model = row["lens_model"].as_str();
        let iso_speed = row["iso_speed"].as_i64();
        let f_number = row["f_number"].as_f64();
        let exposure_time = row["exposure_time"].as_str();
        let focal_length = row["focal_length"].as_f64();
        let flash = row["flash"].as_str();
        let white_balance = row["white_balance"].as_str();
        let exposure_program = row["exposure_program"].as_str();
        let metering_mode = row["metering_mode"].as_str();
        let orientation = row["orientation"].as_i64();
        let software = row["software"].as_str();
        let artist = row["artist"].as_str();
        let copyright = row["copyright"].as_str();
        let description = row["description"].as_str();
        let user_comment = row["user_comment"].as_str();
        let color_space = row["color_space"].as_str();
        let exposure_bias = row["exposure_bias"].as_f64();
        let scene_type = row["scene_type"].as_str();
        let digital_zoom = row["digital_zoom"].as_f64();
        let exif_overrides = row["exif_overrides"].as_str();

        let result = sqlx::query(
            "UPDATE photos SET \
                is_favorite = ?, \
                crop_metadata = COALESCE(?, crop_metadata), \
                encrypted_blob_id = COALESCE(?, encrypted_blob_id), \
                encrypted_thumb_blob_id = COALESCE(?, encrypted_thumb_blob_id), \
                geo_city = ?, geo_state = ?, geo_country = ?, geo_country_code = ?, \
                photo_year = ?, photo_month = ?, \
                photo_subtype = ?, burst_id = ?, motion_video_blob_id = ?, \
                camera_make = ?, lens_model = ?, iso_speed = ?, f_number = ?, \
                exposure_time = ?, focal_length = ?, flash = ?, white_balance = ?, \
                exposure_program = ?, metering_mode = ?, orientation = ?, \
                software = ?, artist = ?, copyright = ?, description = ?, \
                user_comment = ?, color_space = ?, exposure_bias = ?, \
                scene_type = ?, digital_zoom = ?, exif_overrides = ? \
             WHERE id = ?",
        )
        .bind(is_favorite)
        .bind(crop_metadata)
        .bind(encrypted_blob_id)
        .bind(encrypted_thumb_blob_id)
        .bind(geo_city)
        .bind(geo_state)
        .bind(geo_country)
        .bind(geo_country_code)
        .bind(photo_year)
        .bind(photo_month)
        .bind(photo_subtype)
        .bind(burst_id)
        .bind(motion_video_blob_id)
        .bind(camera_make)
        .bind(lens_model)
        .bind(iso_speed)
        .bind(f_number)
        .bind(exposure_time)
        .bind(focal_length)
        .bind(flash)
        .bind(white_balance)
        .bind(exposure_program)
        .bind(metering_mode)
        .bind(orientation)
        .bind(software)
        .bind(artist)
        .bind(copyright)
        .bind(description)
        .bind(user_comment)
        .bind(color_space)
        .bind(exposure_bias)
        .bind(scene_type)
        .bind(digital_zoom)
        .bind(exif_overrides)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() > 0 {
            ps_updated += 1;
        }
    }

    // ── photo_tags ───────────────────────────────────────────────────────
    // Full-state sync: we accept the primary's view as authoritative for
    // tags belonging to (photo_id, user_id) pairs that appear in the
    // payload, and for users who appear in the payload at all.  This
    // avoids wiping tags belonging to users that haven't been synced yet.
    let photo_tags = body
        .get("photo_tags")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut tag_users: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut tag_keys: std::collections::HashSet<(String, String, String)> =
        std::collections::HashSet::new();
    for row in &photo_tags {
        let photo_id = row["photo_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let tag = row["tag"].as_str().unwrap_or_default();
        let created_at = row["created_at"].as_str().unwrap_or_default();
        if photo_id.is_empty() || user_id.is_empty() || tag.is_empty() {
            continue;
        }
        tag_users.insert(user_id.to_string());
        tag_keys.insert((photo_id.to_string(), user_id.to_string(), tag.to_string()));

        sqlx::query(
            "INSERT INTO photo_tags (photo_id, user_id, tag, created_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(photo_id, user_id, tag) DO NOTHING",
        )
        .bind(photo_id)
        .bind(user_id)
        .bind(tag)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }
    // Prune: only consider users present in the payload to avoid wiping
    // tags belonging to users not yet synced.
    if !tag_users.is_empty() {
        let existing: Vec<(String, String, String)> =
            sqlx::query_as("SELECT photo_id, user_id, tag FROM photo_tags")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();
        for (pid, uid, tag) in &existing {
            if tag_users.contains(uid)
                && !tag_keys.contains(&(pid.clone(), uid.clone(), tag.clone()))
            {
                sqlx::query(
                    "DELETE FROM photo_tags WHERE photo_id = ? AND user_id = ? AND tag = ?",
                )
                .bind(pid)
                .bind(uid)
                .bind(tag)
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    // ── face_clusters ────────────────────────────────────────────────────
    let face_clusters = body
        .get("face_clusters")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut fc_users: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut fc_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for row in &face_clusters {
        let id = row["id"].as_i64().unwrap_or(0);
        let user_id = row["user_id"].as_str().unwrap_or_default();
        if id == 0 || user_id.is_empty() {
            continue;
        }
        fc_users.insert(user_id.to_string());
        fc_ids.insert(id);
        let label = row["label"].as_str();
        let representative = row["representative"].as_str();
        let photo_count = row["photo_count"].as_i64().unwrap_or(0);
        let created_at = row["created_at"].as_str().unwrap_or_default();
        let updated_at = row["updated_at"].as_str().unwrap_or_default();

        sqlx::query(
            "INSERT INTO face_clusters (id, user_id, label, representative, photo_count, \
                                        created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                label = excluded.label, \
                representative = excluded.representative, \
                photo_count = excluded.photo_count, \
                updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(user_id)
        .bind(label)
        .bind(representative)
        .bind(photo_count)
        .bind(created_at)
        .bind(updated_at)
        .execute(&mut *tx)
        .await?;
    }

    // ── face_detections ──────────────────────────────────────────────────
    let face_detections = body
        .get("face_detections")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut fd_users: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut fd_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for row in &face_detections {
        let id = row["id"].as_i64().unwrap_or(0);
        let photo_id = row["photo_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        if id == 0 || photo_id.is_empty() || user_id.is_empty() {
            continue;
        }
        fd_users.insert(user_id.to_string());
        fd_ids.insert(id);
        let cluster_id = row["cluster_id"].as_i64();
        let bbox_x = row["bbox_x"].as_f64().unwrap_or(0.0);
        let bbox_y = row["bbox_y"].as_f64().unwrap_or(0.0);
        let bbox_w = row["bbox_w"].as_f64().unwrap_or(0.0);
        let bbox_h = row["bbox_h"].as_f64().unwrap_or(0.0);
        let confidence = row["confidence"].as_f64().unwrap_or(0.0);
        let embedding: Option<Vec<u8>> = row["embedding_b64"].as_str().and_then(|s| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(s).ok()
        });
        let created_at = row["created_at"].as_str().unwrap_or_default();

        sqlx::query(
            "INSERT INTO face_detections (id, photo_id, user_id, cluster_id, \
                                          bbox_x, bbox_y, bbox_w, bbox_h, \
                                          confidence, embedding, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                cluster_id = excluded.cluster_id, \
                bbox_x = excluded.bbox_x, bbox_y = excluded.bbox_y, \
                bbox_w = excluded.bbox_w, bbox_h = excluded.bbox_h, \
                confidence = excluded.confidence, \
                embedding = excluded.embedding",
        )
        .bind(id)
        .bind(photo_id)
        .bind(user_id)
        .bind(cluster_id)
        .bind(bbox_x)
        .bind(bbox_y)
        .bind(bbox_w)
        .bind(bbox_h)
        .bind(confidence)
        .bind(embedding)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }

    // Prune face_detections + face_clusters for users in payload.
    if !fd_users.is_empty() {
        // Build comma-separated user list for IN clause.  Use parameter
        // binding via a manual loop to stay safe.
        for uid in &fd_users {
            let existing: Vec<i64> =
                sqlx::query_scalar("SELECT id FROM face_detections WHERE user_id = ?")
                    .bind(uid)
                    .fetch_all(&mut *tx)
                    .await
                    .unwrap_or_default();
            for eid in existing {
                if !fd_ids.contains(&eid) {
                    sqlx::query("DELETE FROM face_detections WHERE id = ?")
                        .bind(eid)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
    }
    if !fc_users.is_empty() {
        for uid in &fc_users {
            let existing: Vec<i64> =
                sqlx::query_scalar("SELECT id FROM face_clusters WHERE user_id = ?")
                    .bind(uid)
                    .fetch_all(&mut *tx)
                    .await
                    .unwrap_or_default();
            for eid in existing {
                if !fc_ids.contains(&eid) {
                    sqlx::query("DELETE FROM face_clusters WHERE id = ?")
                        .bind(eid)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
    }

    // ── object_detections ────────────────────────────────────────────────
    let object_detections = body
        .get("object_detections")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut od_users: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut od_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for row in &object_detections {
        let id = row["id"].as_i64().unwrap_or(0);
        let photo_id = row["photo_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        if id == 0 || photo_id.is_empty() || user_id.is_empty() {
            continue;
        }
        od_users.insert(user_id.to_string());
        od_ids.insert(id);
        let class_name = row["class_name"].as_str().unwrap_or_default();
        let confidence = row["confidence"].as_f64().unwrap_or(0.0);
        let bbox_x = row["bbox_x"].as_f64().unwrap_or(0.0);
        let bbox_y = row["bbox_y"].as_f64().unwrap_or(0.0);
        let bbox_w = row["bbox_w"].as_f64().unwrap_or(0.0);
        let bbox_h = row["bbox_h"].as_f64().unwrap_or(0.0);
        let created_at = row["created_at"].as_str().unwrap_or_default();

        sqlx::query(
            "INSERT INTO object_detections (id, photo_id, user_id, class_name, confidence, \
                                            bbox_x, bbox_y, bbox_w, bbox_h, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                class_name = excluded.class_name, \
                confidence = excluded.confidence, \
                bbox_x = excluded.bbox_x, bbox_y = excluded.bbox_y, \
                bbox_w = excluded.bbox_w, bbox_h = excluded.bbox_h",
        )
        .bind(id)
        .bind(photo_id)
        .bind(user_id)
        .bind(class_name)
        .bind(confidence)
        .bind(bbox_x)
        .bind(bbox_y)
        .bind(bbox_w)
        .bind(bbox_h)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }
    if !od_users.is_empty() {
        for uid in &od_users {
            let existing: Vec<i64> =
                sqlx::query_scalar("SELECT id FROM object_detections WHERE user_id = ?")
                    .bind(uid)
                    .fetch_all(&mut *tx)
                    .await
                    .unwrap_or_default();
            for eid in existing {
                if !od_ids.contains(&eid) {
                    sqlx::query("DELETE FROM object_detections WHERE id = ?")
                        .bind(eid)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
    }

    // ── ai_processed_photos ──────────────────────────────────────────────
    let ai_processed = body
        .get("ai_processed_photos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut ap_users: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut ap_keys: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for row in &ai_processed {
        let photo_id = row["photo_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let processed_at = row["processed_at"].as_str().unwrap_or_default();
        if photo_id.is_empty() || user_id.is_empty() {
            continue;
        }
        ap_users.insert(user_id.to_string());
        ap_keys.insert((photo_id.to_string(), user_id.to_string()));

        sqlx::query(
            "INSERT INTO ai_processed_photos (photo_id, user_id, processed_at) \
             VALUES (?, ?, ?) \
             ON CONFLICT(photo_id, user_id) DO UPDATE SET \
                processed_at = excluded.processed_at",
        )
        .bind(photo_id)
        .bind(user_id)
        .bind(processed_at)
        .execute(&mut *tx)
        .await?;
    }
    if !ap_users.is_empty() {
        for uid in &ap_users {
            let existing: Vec<String> =
                sqlx::query_scalar("SELECT photo_id FROM ai_processed_photos WHERE user_id = ?")
                    .bind(uid)
                    .fetch_all(&mut *tx)
                    .await
                    .unwrap_or_default();
            for pid in existing {
                if !ap_keys.contains(&(pid.clone(), uid.clone())) {
                    sqlx::query(
                        "DELETE FROM ai_processed_photos WHERE photo_id = ? AND user_id = ?",
                    )
                    .bind(&pid)
                    .bind(uid)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
    }

    // ── user_settings ────────────────────────────────────────────────────
    let user_settings = body
        .get("user_settings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut us_users: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut us_keys: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for row in &user_settings {
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let key = row["key"].as_str().unwrap_or_default();
        let value = row["value"].as_str().unwrap_or_default();
        let updated_at = row["updated_at"].as_str().unwrap_or_default();
        if user_id.is_empty() || key.is_empty() {
            continue;
        }
        us_users.insert(user_id.to_string());
        us_keys.insert((user_id.to_string(), key.to_string()));

        sqlx::query(
            "INSERT INTO user_settings (user_id, key, value, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(user_id, key) DO UPDATE SET \
                value = excluded.value, \
                updated_at = excluded.updated_at",
        )
        .bind(user_id)
        .bind(key)
        .bind(value)
        .bind(updated_at)
        .execute(&mut *tx)
        .await?;
    }
    if !us_users.is_empty() {
        for uid in &us_users {
            let existing: Vec<String> =
                sqlx::query_scalar("SELECT key FROM user_settings WHERE user_id = ?")
                    .bind(uid)
                    .fetch_all(&mut *tx)
                    .await
                    .unwrap_or_default();
            for k in existing {
                if !us_keys.contains(&(uid.clone(), k.clone())) {
                    sqlx::query("DELETE FROM user_settings WHERE user_id = ? AND key = ?")
                        .bind(uid)
                        .bind(&k)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
    }

    tx.commit().await?;

    tracing::info!(
        "Received metadata sync: {} edit_copies, {} photo_metadata, \
         {} shared_albums, {} members, {} album_photos, {} photo_states ({} updated), \
         {} photo_tags, {} face_clusters, {} face_detections, {} object_detections, \
         {} ai_processed, {} user_settings",
        edit_copies.len(),
        photo_metadata.len(),
        shared_albums.len(),
        shared_members.len(),
        shared_photos.len(),
        photo_states.len(),
        ps_updated,
        photo_tags.len(),
        face_clusters.len(),
        face_detections.len(),
        object_detections.len(),
        ai_processed.len(),
        user_settings.len(),
    );

    Ok(StatusCode::OK)
}
