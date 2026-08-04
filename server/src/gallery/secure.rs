//! Secure gallery management endpoints.
//!
//! Secure galleries use the user's account password for authentication,
//! not a separate gallery-specific password.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::crypto;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use crate::photos::models::*;

/// Correlates a membership row with the photo a caller is naming, in either of
/// the two id spaces a client may use: the canonical **photo id** (stored as
/// `original_blob_id`) or the **clone blob id** the secure album serves.
///
/// `?1` = the resolved canonical original id, `?2` = the id as the client sent
/// it. Both are needed: Android sends `serverBlobId` (a `blobs` row) while web
/// sends the photo id, and a second add must recognise the first one's row
/// whichever space it arrived in.
///
/// Shared verbatim by [`existing_memberships`] (which decides whether an add is
/// a duplicate, a new clone, or an adoption) and by the tests, rather than being
/// re-typed at each site. This repo has now recorded nine separate instances of
/// one list derived twice and drifting; the membership correlation is a tenth
/// candidate and is deliberately not written out by hand anywhere.
pub const SECURE_MEMBERSHIP_MATCH: &str = "(gi.original_blob_id = ?1 OR gi.blob_id = ?2)";

/// One existing secure-album membership for a photo, in any of the user's
/// galleries. Carries everything an *adoption* needs to reuse the clone rather
/// than paying for a second one.
#[derive(Debug, Clone, FromRow)]
pub struct ExistingMembership {
    pub id: String,
    pub gallery_id: String,
    pub blob_id: String,
    pub original_blob_id: Option<String>,
    pub original_photo_hash: Option<String>,
    pub encrypted_blob_id: Option<String>,
    pub encrypted_thumb_blob_id: Option<String>,
    pub crop_metadata: Option<String>,
}

/// Every secure-album membership this photo already has, across all galleries
/// the user owns. Empty means "not secured anywhere".
///
/// A photo may now live in **several** secure albums (Z1), so this returns a
/// list where it once returned an `Option`. The two callers ask different
/// questions of it: `add_gallery_item` asks "is one of these in the album I am
/// adding to" (a true duplicate) and "is there one anywhere else" (a clone it
/// can adopt).
pub async fn existing_memberships(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    canonical_original_id: &str,
    requested_blob_id: &str,
) -> Result<Vec<ExistingMembership>, AppError> {
    let rows = sqlx::query_as::<_, ExistingMembership>(&format!(
        "SELECT gi.id, gi.gallery_id, gi.blob_id, gi.original_blob_id, \
                gi.original_photo_hash, gi.encrypted_blob_id, gi.encrypted_thumb_blob_id, \
                gi.crop_metadata \
         FROM encrypted_gallery_items gi \
         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
         WHERE g.user_id = ?3 AND {SECURE_MEMBERSHIP_MATCH} \
         ORDER BY gi.added_at ASC"
    ))
    .bind(canonical_original_id)
    .bind(requested_blob_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Whether some **other** membership row still points at `clone_blob_id`.
///
/// This is the guard that makes multi-album membership safe to remove from.
/// [`remove_gallery_item`] destroys the clone blob, the clone `photos` row, its
/// encrypted blobs and its thumbnail files — correct when the row being removed
/// is the only one referencing them, and **silent data loss for every other
/// album** the moment it is not. Removing a photo from one album would blank its
/// tile in the others while leaving their membership rows intact, which reads as
/// corruption rather than as a deletion.
///
/// Deliberately scoped to galleries owned by `user_id`, matching every other
/// query here: a clone blob is never shared across users (each add clones), so a
/// row belonging to somebody else can only be a bug, and treating it as a
/// reference would leak a blob rather than protect one.
pub async fn clone_is_shared(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    clone_blob_id: &str,
    excluding_item_id: &str,
) -> Result<bool, AppError> {
    let shared: bool = sqlx::query_scalar(
        "SELECT EXISTS(\
           SELECT 1 FROM encrypted_gallery_items gi \
           JOIN encrypted_galleries g ON g.id = gi.gallery_id \
           WHERE g.user_id = ? AND gi.blob_id = ? AND gi.id != ?\
         )",
    )
    .bind(user_id)
    .bind(clone_blob_id)
    .bind(excluding_item_id)
    .fetch_one(pool)
    .await?;
    Ok(shared)
}

/// One row of the aggregate secure feed: an item plus the gallery it is filed
/// in. A photo in N secure albums produces N of these.
#[derive(Debug, Clone, FromRow)]
pub struct AllGalleryItemRow {
    pub id: String,
    pub blob_id: String,
    /// Identity key for the collapse — the clone blob every membership of one
    /// photo shares. Selected raw rather than `COALESCE`d like `blob_id`,
    /// because it is precisely the column an adoption reuses.
    pub clone_blob_id: String,
    pub added_at: String,
    pub gallery_id: String,
    pub gallery_name: String,
    pub encrypted_thumb_blob_id: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub media_type: Option<String>,
    pub photo_subtype: Option<String>,
    pub burst_id: Option<String>,
    pub duration_secs: Option<f64>,
    pub motion_video_blob_id: Option<String>,
    pub crop_metadata: Option<String>,
}

/// One tile of the aggregate feed after multi-album memberships are collapsed.
pub struct CollapsedItem<'a> {
    /// The membership that represents the tile.
    pub rep: &'a AllGalleryItemRow,
    /// Every album the photo is in, **oldest membership first**, as
    /// `(gallery_id, gallery_name)`.
    pub galleries: Vec<(&'a str, &'a str)>,
}

/// Collapse multi-album memberships into one tile per photo (Z1).
///
/// A photo may now sit in several secure albums sharing a clone, which would
/// otherwise surface in the aggregate feed as N identical tiles. That feed is
/// not merely a listing: the secure smart albums are derived from it, so a
/// duplicated row becomes a **double-counted** tile in "Secure Videos" and
/// friends. Raw rows vs collapsed tiles is the single most repeated defect
/// shape in this repo's history, which is why the collapse happens once, at the
/// source of the feed, instead of in each consumer.
///
/// `rows` is expected in `added_at DESC` order (as the query returns them). The
/// representative is the **oldest** membership, so the tile's `added_at` is when
/// the photo entered the secure domain rather than when it was later filed into
/// an additional album — a photo does not become "new" again because it was
/// added to a second album. Tile order follows first appearance, i.e. the
/// newest membership, so filing a photo into another album does surface it.
pub fn collapse_by_clone(rows: &[AllGalleryItemRow]) -> Vec<CollapsedItem<'_>> {
    let mut out: Vec<CollapsedItem<'_>> = Vec::new();
    let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();

    for r in rows {
        match index.get(r.clone_blob_id.as_str()) {
            Some(&i) => {
                let slot: &mut CollapsedItem<'_> = &mut out[i];
                // Rows arrive newest-first, so each later row is older: it
                // becomes the representative, and prepending keeps `galleries`
                // oldest-first.
                slot.rep = r;
                slot.galleries.insert(0, (&r.gallery_id, &r.gallery_name));
            }
            None => {
                index.insert(r.clone_blob_id.as_str(), out.len());
                out.push(CollapsedItem {
                    rep: r,
                    galleries: vec![(&r.gallery_id, &r.gallery_name)],
                });
            }
        }
    }
    out
}

/// GET /api/galleries/secure
/// List secure galleries for the authenticated user.
pub async fn list_secure_galleries(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<SecureGalleryListResponse>, AppError> {
    let galleries = sqlx::query_as::<_, SecureGalleryRecord>(
        "SELECT g.id, g.name, g.created_at, \
         (SELECT COUNT(*) FROM encrypted_gallery_items WHERE gallery_id = g.id) as item_count \
         FROM encrypted_galleries g WHERE g.user_id = ? ORDER BY g.created_at DESC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(SecureGalleryListResponse { galleries }))
}

/// POST /api/galleries/secure
/// Create a new secure gallery (no separate password — uses account password).
pub async fn create_secure_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateSecureGalleryRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let name = sanitize::sanitize_display_name(&req.name, 100)
        .map_err(|reason| AppError::BadRequest(reason.into()))?;

    let gallery_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    // Store a placeholder for password_hash (column is NOT NULL for legacy compat).
    // Auth is handled via the user's account password at unlock time.
    sqlx::query(
        "INSERT INTO encrypted_galleries (id, user_id, name, password_hash, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .bind(&name)
    .bind("account-auth") // placeholder — not used for verification
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "gallery_id": gallery_id,
            "name": name,
        })),
    ))
}

/// POST /api/galleries/secure/unlock
/// Verify the user's account password. Returns a gallery access token
/// (keyed-SHA256 signed, 1-hour TTL) that must be presented as
/// `X-Gallery-Token` to list a gallery's items.
///
/// The token is now verified server-side on the read path — see
/// [`crate::gallery::secure_token`] and [`list_gallery_items`].
pub async fn unlock_secure_galleries(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<UnlockSecureGalleryRequest>,
) -> Result<Json<SecureGalleryUnlockResponse>, AppError> {
    // Verify against the user's account password
    let password_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;

    let valid = bcrypt::verify(&req.password, &password_hash)
        .map_err(|e| AppError::Internal(format!("Bcrypt verify failed: {e}")))?;

    if !valid {
        return Err(AppError::Unauthorized("Invalid password".into()));
    }

    let (token, expires_in) =
        crate::gallery::secure_token::generate(&auth.user_id, &state.config.auth.jwt_secret);

    Ok(Json(SecureGalleryUnlockResponse {
        gallery_token: token,
        expires_in,
    }))
}

/// DELETE /api/galleries/secure/:id
/// Delete a secure gallery and its items.
///
/// Ownership is verified first — only the gallery owner can delete it.
pub async fn delete_secure_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Verify ownership BEFORE deleting items to prevent IDOR:
    // without this check any authenticated user who guesses a gallery UUID
    // could wipe another user's gallery items.
    let result = sqlx::query("DELETE FROM encrypted_galleries WHERE id = ? AND user_id = ?")
        .bind(&gallery_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // Now safe to delete items — we've confirmed the caller owns the gallery.
    sqlx::query("DELETE FROM encrypted_gallery_items WHERE gallery_id = ?")
        .bind(&gallery_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/galleries/secure/:gallery_id/items/:item_id
///
/// Remove an item from a secure gallery, returning the original photo to
/// the regular gallery.  This deletes the cloned blob (and clone photos
/// row, if any) created by `add_gallery_item`, and removes the
/// `encrypted_gallery_items` membership row.  The original photo —
/// referenced via `original_blob_id` — is automatically un-hidden the
/// next time the main gallery polls `/api/galleries/secure/blob-ids`.
///
/// Ownership of the gallery is verified before any deletion.
pub async fn remove_gallery_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((gallery_id, item_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    // Verify gallery ownership first.
    let owner: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
            .bind(&gallery_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;
    if owner == 0 {
        return Err(AppError::NotFound);
    }

    // Look up the item — we need the cloned blob_id (and encrypted_*) to
    // delete the underlying files and DB rows.
    let item: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT blob_id, encrypted_blob_id, encrypted_thumb_blob_id \
         FROM encrypted_gallery_items WHERE id = ? AND gallery_id = ?",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .fetch_optional(&state.pool)
    .await?;

    let (clone_blob_id, enc_blob_id, enc_thumb_blob_id) = item.ok_or(AppError::NotFound)?;

    // A photo may sit in several secure albums sharing ONE clone (Z1).  If any
    // other membership still points at this clone, removing it from *this*
    // album must drop the membership row and nothing else — the destruction
    // below would otherwise delete the bytes the other albums are still
    // displaying, leaving their rows intact and their tiles blank.  That is
    // silent data loss, and it looks like corruption rather than like a
    // deletion the user asked for.
    if clone_is_shared(&state.pool, &auth.user_id, &clone_blob_id, &item_id).await? {
        sqlx::query("DELETE FROM encrypted_gallery_items WHERE id = ? AND gallery_id = ?")
            .bind(&item_id)
            .bind(&gallery_id)
            .execute(&state.pool)
            .await?;

        tracing::info!(
            gallery_id = %gallery_id,
            item_id = %item_id,
            clone_blob_id = %clone_blob_id,
            "[DIAG:SECURE_REMOVE] Dropped membership only — clone still referenced by another secure album"
        );

        // Deliberately NOT returned to the regular gallery: the photo is still
        // secured elsewhere, so `list_secure_blob_ids` still reports its
        // original id and the main gallery keeps hiding it.  Un-hiding here
        // would surface a photo the user still has in a secure album.
        return Ok(StatusCode::NO_CONTENT);
    }

    let storage_root = (**state.storage_root.load()).clone();

    // Delete the cloned blob file + row (if owned by this user).
    let clone_blob: Option<(String,)> =
        sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
            .bind(&clone_blob_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await?;
    if let Some((sp,)) = clone_blob {
        let _ = crate::blobs::storage::delete_blob(&storage_root, &sp).await;
        let _ = sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
            .bind(&clone_blob_id)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;
    }

    // Delete server-side clone photos row (and its thumbnail file) if any.
    // The clone uses the same id as the cloned blob.
    let clone_photo: Option<(String, Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT file_path, thumb_path, encrypted_blob_id, encrypted_thumb_blob_id \
         FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&clone_blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;
    if let Some((fp, tp, photo_enc_blob, photo_enc_thumb)) = clone_photo {
        if !fp.is_empty() {
            let _ = crate::blobs::storage::delete_blob(&storage_root, &fp).await;
        }
        if let Some(tp) = tp {
            let _ = crate::blobs::storage::delete_blob(&storage_root, &tp).await;
        }
        // Delete encrypted blobs that belong only to this clone photo row
        for eb in [photo_enc_blob.as_deref(), photo_enc_thumb.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Ok(Some((sp,))) = sqlx::query_as::<_, (String,)>(
                "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
            )
            .bind(eb)
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await
            {
                let _ = crate::blobs::storage::delete_blob(&storage_root, &sp).await;
                let _ = sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
                    .bind(eb)
                    .bind(&auth.user_id)
                    .execute(&state.pool)
                    .await;
            }
        }
        sqlx::query("DELETE FROM photos WHERE id = ? AND user_id = ?")
            .bind(&clone_blob_id)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;
    }

    // Delete encrypted_blob_id / encrypted_thumb_blob_id stored on the item
    // (used on backup servers when there is no photos clone row).  Avoid
    // double-deleting blobs we already removed above.
    for eb in [enc_blob_id.as_deref(), enc_thumb_blob_id.as_deref()]
        .into_iter()
        .flatten()
        .filter(|id| *id != clone_blob_id)
    {
        if let Ok(Some((sp,))) = sqlx::query_as::<_, (String,)>(
            "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
        )
        .bind(eb)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        {
            let _ = crate::blobs::storage::delete_blob(&storage_root, &sp).await;
            let _ = sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
                .bind(eb)
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await;
        }
    }

    // Finally drop the membership row — the original photo becomes visible
    // again because `list_secure_blob_ids` will no longer include its id.
    sqlx::query("DELETE FROM encrypted_gallery_items WHERE id = ? AND gallery_id = ?")
        .bind(&item_id)
        .bind(&gallery_id)
        .execute(&state.pool)
        .await?;

    tracing::info!(
        gallery_id = %gallery_id,
        item_id = %item_id,
        clone_blob_id = %clone_blob_id,
        "[DIAG:SECURE_REMOVE] Removed item from secure gallery; original returned to gallery"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Request body for `POST /api/galleries/secure/{id}/items`.
/// Associates an encrypted blob with a secure gallery.
#[derive(Debug, Deserialize)]
pub struct AddGalleryItemRequest {
    pub blob_id: String,
}

/// POST /api/galleries/secure/{id}/items — add a blob to a secure gallery.
///
/// Creates an **independent copy** of the blob for the secure album rather
/// than sharing a reference to the original.  This ensures each secure album
/// folder has its own blob namespace, preventing mix-ups between main-gallery
/// and secure-album data.
pub async fn add_gallery_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
    Json(req): Json<AddGalleryItemRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
            .bind(&gallery_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    if count == 0 {
        return Err(AppError::NotFound);
    }

    // Determine the canonical "original" identity for this add: for a
    // server-side photo id it's the id itself; for a client-encrypted blob id
    // it's the owning photo's id (photos.encrypted_blob_id = req.blob_id).
    let candidate_original_id: String = sqlx::query_scalar::<_, String>(
        "SELECT id FROM photos WHERE encrypted_blob_id = ? AND user_id = ?",
    )
    .bind(&req.blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| req.blob_id.clone());

    // A photo may now live in SEVERAL secure albums (Z1).  What survives from
    // the old one-secure-album invariant is only its useful half: a photo may
    // not be added to the *same* album twice.  That still has to be enforced
    // here rather than client-side, for the reason the original invariant was
    // moved server-side in the first place — two windows, a stale picker, or a
    // raw API call can all double-add.
    let existing = existing_memberships(
        &state.pool,
        &auth.user_id,
        &candidate_original_id,
        &req.blob_id,
    )
    .await?;

    if let Some(dup) = existing.iter().find(|m| m.gallery_id == gallery_id) {
        tracing::info!(
            target_gallery_id = %gallery_id,
            existing_item_id = %dup.id,
            req_blob_id = %req.blob_id,
            candidate_original_id = %candidate_original_id,
            "[DIAG:SECURE_ADD] Rejected duplicate add — photo already in THIS secure album"
        );
        return Err(AppError::Conflict(
            "Photo is already in this secure album".into(),
        ));
    }

    // Already secured elsewhere → ADOPT that album's clone instead of making a
    // second one.  This is the whole reason multi-album membership is cheap:
    // the plaintext clone has already been produced and re-encrypted at rest,
    // so a second membership row costs one INSERT and zero bytes.  Cloning
    // again would double the storage, spend a second decrypt+encrypt pass on
    // (potentially) a multi-gigabyte video, and — because the two clones would
    // encrypt to different blobs — leave the album showing what is physically a
    // different file, so an edit applied in one album could never reach the
    // other.
    //
    // The original photo is already hidden from the main gallery by the first
    // membership, and `list_secure_blob_ids` dedups into a HashSet, so nothing
    // about the hiding behaviour changes.
    if let Some(donor) = existing.first() {
        let item_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO encrypted_gallery_items \
             (id, gallery_id, blob_id, added_at, original_blob_id, original_photo_hash, \
              encrypted_blob_id, encrypted_thumb_blob_id, crop_metadata) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&item_id)
        .bind(&gallery_id)
        .bind(&donor.blob_id)
        .bind(&now)
        .bind(&donor.original_blob_id)
        .bind(&donor.original_photo_hash)
        .bind(&donor.encrypted_blob_id)
        .bind(&donor.encrypted_thumb_blob_id)
        // Carry the crop across so the photo looks the same in both albums.
        // Edits stay per-item after this point (migration 032 put crop on the
        // membership row deliberately); this only sets the starting state, so
        // the second album does not open showing an uncropped photo the user
        // already framed.
        .bind(&donor.crop_metadata)
        .execute(&state.pool)
        .await?;

        tracing::info!(
            gallery_id = %gallery_id,
            donor_gallery_id = %donor.gallery_id,
            donor_item_id = %donor.id,
            clone_blob_id = %donor.blob_id,
            item_id = %item_id,
            total_memberships = existing.len() + 1,
            "[DIAG:SECURE_ADD] Adopted existing clone into an additional secure album"
        );

        return Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "item_id": item_id,
                "new_blob_id": donor.blob_id,
                "adopted": true,
            })),
        ));
    }

    // Fetch original blob metadata — first try the `blobs` table (encrypted
    // uploads), then fall back to the `photos` table (autoscanned/server-side
    // files).  The client may pass either a blob ID or a photo ID.
    let storage_root = (**state.storage_root.load()).clone();
    let now = Utc::now().to_rfc3339();

    let blob_row: Option<(String, String, i64, Option<String>, String, Option<String>)> =
        sqlx::query_as(
            "SELECT id, blob_type, size_bytes, client_hash, storage_path, content_hash \
             FROM blobs WHERE id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;

    // Track whether the source is a server-side photo (for cloning into photos table)
    let is_server_side = blob_row.is_none();

    /// Row shape for the full photos table query used when cloning server-side photos.
    #[derive(Debug, Clone, FromRow)]
    struct PhotoRowFull {
        filename: String,
        mime_type: String,
        media_type: String,
        file_path: String,
        size_bytes: i64,
        width: i32,
        height: i32,
        duration_secs: Option<f64>,
        taken_at: Option<String>,
        latitude: Option<f64>,
        longitude: Option<f64>,
        thumb_path: Option<String>,
        created_at: String,
        is_favorite: i32,
        crop_metadata: Option<String>,
        camera_model: Option<String>,
        photo_hash: Option<String>,
        encrypted_blob_id: Option<String>,
    }

    // Full photo row needed for server-side clones
    let photo_row_full: Option<PhotoRowFull> = if is_server_side {
        sqlx::query_as::<_, PhotoRowFull>(
            "SELECT filename, mime_type, media_type, file_path, size_bytes, width, height, \
                 duration_secs, taken_at, latitude, longitude, thumb_path, created_at, \
                 is_favorite, crop_metadata, camera_model, photo_hash, encrypted_blob_id \
                 FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?
    } else {
        None
    };

    // Resolve source file path, metadata, and determine blob_type
    let (blob_type, size_bytes, client_hash, storage_path, _content_hash): (
        String,
        i64,
        Option<String>,
        String,
        Option<String>,
    ) = if let Some((_id, bt, sz, ch, sp, coh)) = blob_row {
        (bt, sz, ch, sp, coh)
    } else {
        // Not in blobs table — use the photos table row
        let prf = photo_row_full
            .as_ref()
            .ok_or_else(|| AppError::BadRequest("Photo or blob not found".into()))?;

        // Derive blob_type from media_type (same logic as restore)
        let bt = match prf.media_type.as_str() {
            "gif" => "gif".to_string(),
            "video" => "video".to_string(),
            "audio" => "audio".to_string(),
            _ if prf.mime_type.starts_with("video/") => "video".to_string(),
            _ => "photo".to_string(),
        };
        (
            bt,
            prf.size_bytes,
            None,
            prf.file_path.clone(),
            prf.photo_hash.clone(),
        )
    };

    // Clone the source media to a fresh plaintext blob WITHOUT holding the whole
    // file in memory (the migration re-encrypts it afterward — chunked for large
    // files). For a server-side-encrypted source (empty file_path) we
    // stream-decrypt the encrypted blob frame-by-frame to disk; for a plaintext
    // source we stream-copy the file. This keeps secure-adding a multi-gigabyte
    // video off the heap, matching the import path.
    let new_blob_id = Uuid::new_v4().to_string();
    let new_blob_abs = crate::blobs::storage::blob_path(&storage_root, &auth.user_id, &new_blob_id);
    if let Some(parent) = new_blob_abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("Create clone directory: {e}")))?;
    }
    let new_storage_path = crate::blobs::storage::relative_path(&auth.user_id, &new_blob_id);

    if storage_path.is_empty() {
        // Encrypted source — stream-decrypt the encrypted blob to the clone.
        let enc_blob_id = photo_row_full
            .as_ref()
            .and_then(|p| p.encrypted_blob_id.as_deref())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::BadRequest("Photo has no file on disk and no encrypted blob".into())
            })?;

        let enc_key = crypto::load_wrapped_key(&state.pool, &state.config.auth.jwt_secret)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to load encryption key: {e}")))?
            .ok_or_else(|| AppError::Internal("No encryption key configured".into()))?;

        let enc_sp: Option<(String,)> =
            sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
                .bind(enc_blob_id)
                .bind(&auth.user_id)
                .fetch_optional(&state.pool)
                .await?;
        let (enc_storage_path,) = enc_sp
            .ok_or_else(|| AppError::Internal(format!("Encrypted blob {enc_blob_id} not found")))?;

        let src_abs = storage_root.join(&enc_storage_path);
        let dst_abs = new_blob_abs.clone();
        let k = enc_key;
        tokio::task::spawn_blocking(move || {
            crate::blobs::chunked::decrypt_blob_file_to_file(&k, &src_abs, &dst_abs)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Decrypt task panicked: {e}")))?
        .map_err(|e| AppError::Internal(format!("Decrypt failed: {e}")))?;

        tracing::info!(
            encrypted_blob_id = %enc_blob_id,
            "[DIAG:SECURE_ADD] Stream-decrypted encrypted photo for secure gallery clone"
        );
    } else {
        // Plaintext source on disk — stream-copy to the clone.
        let src_abs = storage_root.join(&storage_path);
        tokio::fs::copy(&src_abs, &new_blob_abs)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to copy source blob: {e}")))?;
    }

    // Actual on-disk size of the plaintext clone (the resolved source size may be
    // the *encrypted* size for an encrypted source).
    let clone_size = tokio::fs::metadata(&new_blob_abs)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(size_bytes);

    // Insert the cloned blob record.
    // content_hash is deliberately set to NULL so the server-side encryption
    // migration's dedup check does NOT match this plaintext clone blob.
    // Without this, the dedup incorrectly "reuses" the clone's own blob as
    // the encrypted_blob_id (pointing to unencrypted data → AES/GCM errors).
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&new_blob_id)
    .bind(&auth.user_id)
    .bind(&blob_type)
    .bind(clone_size)
    .bind(&client_hash)
    .bind(&now)
    .bind(&new_storage_path)
    .execute(&state.pool)
    .await?;

    // For server-side (autoscanned) photos, also create a `photos` table row
    // for the clone.  This ensures the viewer's `/api/photos/{id}/file` and
    // `/api/photos/{id}/thumbnail` endpoints can serve the cloned file.
    if let Some(prf) = &photo_row_full {
        // Resolve the thumbnail: copy the original thumbnail file if it exists
        let new_thumb_path = if let Some(tp) = &prf.thumb_path {
            let thumb_data = crate::blobs::storage::read_blob(&storage_root, tp)
                .await
                .ok(); // Non-fatal if thumbnail missing
            if let Some(td) = thumb_data {
                let thumb_id = format!("{new_blob_id}_thumb");
                crate::blobs::storage::write_blob(&storage_root, &auth.user_id, &thumb_id, &td)
                    .await
                    .ok()
            } else {
                None
            }
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
             thumb_path, created_at, is_favorite, crop_metadata, camera_model, photo_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&new_blob_id)
        .bind(&auth.user_id)
        .bind(&prf.filename)
        .bind(&new_storage_path)
        .bind(&prf.mime_type)
        .bind(&prf.media_type)
        .bind(prf.size_bytes)
        .bind(prf.width)
        .bind(prf.height)
        .bind(prf.duration_secs)
        .bind(&prf.taken_at)
        .bind(prf.latitude)
        .bind(prf.longitude)
        .bind(&new_thumb_path)
        .bind(&prf.created_at)
        .bind(prf.is_favorite)
        .bind(&prf.crop_metadata)
        .bind(&prf.camera_model)
        .bind(Option::<String>::None) // Don't copy photo_hash — it has a unique index per user
        .execute(&state.pool)
        .await?;

        tracing::info!(
            new_blob_id = %new_blob_id,
            original_id = %req.blob_id,
            mime_type = %prf.mime_type,
            "[DIAG:SECURE_ADD] Created photos table row for server-side clone"
        );
    }

    let item_id = Uuid::new_v4().to_string();

    // When the client sends an encrypted blob ID (Android: serverBlobId),
    // resolve the owning photo so we can store the **photo ID** as
    // original_blob_id.  This is critical because encrypted_sync hides
    // photos by `photos.id NOT IN (original_blob_id)`, and the blob
    // ID differs from the photo ID.
    let (resolved_original_id, original_enc_thumb): (String, Option<String>) = if !is_server_side {
        // The req.blob_id is a blobs-table ID.  Find the photo that owns it.
        let owner: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, photo_hash, encrypted_thumb_blob_id \
             FROM photos WHERE encrypted_blob_id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;

        if let Some((photo_id, _hash, thumb)) = owner {
            (photo_id, thumb)
        } else {
            // No owning photo found — fall back to blob ID
            (req.blob_id.clone(), None)
        }
    } else {
        (req.blob_id.clone(), None)
    };

    // Store the original photo's content hash so autoscan (run after recovery)
    // can skip files whose content matches a gallery-hidden original — even if
    // the file has been renamed or moved.
    let original_photo_hash: Option<String> = if is_server_side {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT photo_hash FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .flatten()
    } else {
        // Client-encrypted blob — use the resolved photo's hash
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT photo_hash FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&resolved_original_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .flatten()
    };

    sqlx::query(
        "INSERT OR IGNORE INTO encrypted_gallery_items \
         (id, gallery_id, blob_id, added_at, original_blob_id, original_photo_hash, encrypted_blob_id, encrypted_thumb_blob_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .bind(&new_blob_id)
    .bind(&now)
    .bind(&resolved_original_id) // Photo ID — so encrypted_sync can hide the original
    .bind(&original_photo_hash)
    .bind(if !is_server_side { Some(&new_blob_id) } else { None::<&String> }) // Clone of encrypted data is already "encrypted"
    .bind(&original_enc_thumb) // Copy the original photo's encrypted thumb
    .execute(&state.pool)
    .await?;

    tracing::info!(
        gallery_id = %gallery_id,
        original_blob_id = %resolved_original_id,
        req_blob_id = %req.blob_id,
        new_blob_id = %new_blob_id,
        item_id = %item_id,
        blob_type = %blob_type,
        is_server_side = is_server_side,
        "[DIAG:SECURE_ADD] Cloned blob into secure gallery"
    );

    // Trigger encryption migration for the newly created clone so the
    // EncryptionBanner doesn't report it as "pending" indefinitely.
    // Fire-and-forget — the response returns immediately.
    if is_server_side {
        let pool = state.pool.clone();
        let sr = (**state.storage_root.load()).clone();
        let jwt = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            crate::photos::server_migrate::auto_migrate_after_scan(pool, sr, jwt).await;
        });
    }

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "item_id": item_id,
            "new_blob_id": new_blob_id,
        })),
    ))
}

/// GET /api/galleries/secure/blob-ids
/// Return all blob IDs that live in any of the user's secure galleries.
/// This is used by the main gallery to filter out "private" items without
/// requiring the gallery unlock token.
pub async fn list_secure_blob_ids(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Return BOTH the cloned blob IDs and the original blob IDs so the
    // main gallery can hide originals that have been moved to secure albums.
    // Also include encrypted_blob_id and encrypted_thumb_blob_id of photos
    // in secure galleries so the web client can filter those from blob listings.
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT gi.blob_id, gi.original_blob_id \
         FROM encrypted_gallery_items gi \
         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
         WHERE g.user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let mut ids = std::collections::HashSet::new();
    for (cloned_id, original_id) in &rows {
        ids.insert(cloned_id.clone());
        if let Some(orig) = original_id {
            ids.insert(orig.clone());
        }
    }

    // Also include encrypted_blob_id and encrypted_thumb_blob_id of photos
    // that are in secure galleries.  These blobs are created by server-side
    // encryption migration and have different IDs from the photos.id entries.
    let enc_blob_rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT p.encrypted_blob_id, p.encrypted_thumb_blob_id \
         FROM photos p \
         WHERE (p.id IN (SELECT gi.blob_id FROM encrypted_gallery_items gi \
                         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
                         WHERE g.user_id = ?) \
                OR p.id IN (SELECT gi.original_blob_id FROM encrypted_gallery_items gi \
                            JOIN encrypted_galleries g ON g.id = gi.gallery_id \
                            WHERE g.user_id = ? AND gi.original_blob_id IS NOT NULL)) \
         AND (p.encrypted_blob_id IS NOT NULL OR p.encrypted_thumb_blob_id IS NOT NULL)",
    )
    .bind(&auth.user_id)
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    for (enc_blob, enc_thumb) in &enc_blob_rows {
        if let Some(eb) = enc_blob {
            ids.insert(eb.clone());
        }
        if let Some(et) = enc_thumb {
            ids.insert(et.clone());
        }
    }

    // Also include encrypted_blob_id and encrypted_thumb_blob_id stored
    // directly on encrypted_gallery_items (populated on backup servers by
    // gallery metadata sync).  On the primary these columns are typically
    // NULL, but on backup the clone photos row may not exist in the photos
    // table, so the JOIN above would miss them.
    let egi_enc_rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT gi.encrypted_blob_id, gi.encrypted_thumb_blob_id \
         FROM encrypted_gallery_items gi \
         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
         WHERE g.user_id = ? \
         AND (gi.encrypted_blob_id IS NOT NULL OR gi.encrypted_thumb_blob_id IS NOT NULL)",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    for (enc_blob, enc_thumb) in &egi_enc_rows {
        if let Some(eb) = enc_blob {
            ids.insert(eb.clone());
        }
        if let Some(et) = enc_thumb {
            ids.insert(et.clone());
        }
    }

    let id_vec: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();

    tracing::debug!(
        user_id = %auth.user_id,
        total_ids = id_vec.len(),
        cloned_count = rows.len(),
        "[DIAG:SECURE_IDS] Returning secure blob IDs (cloned + originals)"
    );

    Ok(Json(serde_json::json!({ "blob_ids": id_vec })))
}

/// GET /api/galleries/secure/:id/items
/// List items in a secure gallery (requires unlock token in header).
pub async fn list_gallery_items(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(gallery_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
            .bind(&gallery_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    if count == 0 {
        return Err(AppError::NotFound);
    }

    // Verify the unlock token: it must be a non-expired, correctly-signed
    // token issued to *this* user. Previously any non-empty string was
    // accepted, which made the password gate cosmetic.
    let token = headers
        .get("x-gallery-token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::Unauthorized("Gallery token required. Unlock the gallery first.".into())
        })?;

    if !crate::gallery::secure_token::verify(token, &auth.user_id, &state.config.auth.jwt_secret) {
        return Err(AppError::Unauthorized(
            "Invalid or expired gallery token. Unlock the gallery again.".into(),
        ));
    }

    // The subtype/burst/duration/motion fields live on the ORIGINAL photo
    // (`op`, joined via original_blob_id) — `add_gallery_item` does not copy
    // them onto the server-side clone row (`p`), so COALESCE falls through to
    // `op`. These let the Android secure viewer render videos, panoramas/360,
    // motion (LIVE) photos and collapse bursts the same way the main gallery
    // does.
    #[derive(FromRow)]
    struct GalleryItemRow {
        id: String,
        blob_id: String,
        added_at: String,
        encrypted_thumb_blob_id: Option<String>,
        width: Option<i64>,
        height: Option<i64>,
        media_type: Option<String>,
        photo_subtype: Option<String>,
        burst_id: Option<String>,
        duration_secs: Option<f64>,
        motion_video_blob_id: Option<String>,
        // Non-destructive edit metadata stored on the item itself (#31). Lives
        // on `gi` only — never falls through to the original photo, so an edit
        // in the secure album can't leak back onto the regular-gallery original.
        crop_metadata: Option<String>,
    }

    let items = sqlx::query_as::<_, GalleryItemRow>(
        "SELECT gi.id, \
                COALESCE(gi.encrypted_blob_id, p.encrypted_blob_id, gi.blob_id) as blob_id, \
                gi.added_at, \
                COALESCE(gi.encrypted_thumb_blob_id, p.encrypted_thumb_blob_id, op.encrypted_thumb_blob_id) as encrypted_thumb_blob_id, \
                COALESCE(p.width, op.width) as width, \
                COALESCE(p.height, op.height) as height, \
                COALESCE(p.media_type, op.media_type) as media_type, \
                COALESCE(p.photo_subtype, op.photo_subtype) as photo_subtype, \
                COALESCE(p.burst_id, op.burst_id) as burst_id, \
                COALESCE(p.duration_secs, op.duration_secs) as duration_secs, \
                COALESCE(p.motion_video_blob_id, op.motion_video_blob_id) as motion_video_blob_id, \
                gi.crop_metadata as crop_metadata \
         FROM encrypted_gallery_items gi \
         LEFT JOIN photos p ON p.id = gi.blob_id AND p.encrypted_blob_id IS NOT NULL \
         LEFT JOIN photos op ON op.id = gi.original_blob_id \
         WHERE gi.gallery_id = ? \
         ORDER BY gi.added_at DESC",
    )
    .bind(&gallery_id)
    .fetch_all(&state.pool)
    .await?;

    // The ladder of each secured video (#49). Keyed by item id — see
    // `list_renditions_for_secure_items` for why a secured video still has one
    // and why a video secured before its rung existed never will.
    let mut ladders = crate::transcode::renditions::list_renditions_for_secure_items(
        &state.pool,
        &auth.user_id,
        Some(&gallery_id),
    )
    .await?;

    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "blob_id": r.blob_id,
                "added_at": r.added_at,
                "gallery_id": gallery_id,
                "renditions": ladders.remove(&r.id).unwrap_or_default(),
                "encrypted_thumb_blob_id": r.encrypted_thumb_blob_id,
                "width": r.width,
                "height": r.height,
                "media_type": r.media_type,
                "photo_subtype": r.photo_subtype,
                "burst_id": r.burst_id,
                "duration_secs": r.duration_secs,
                "motion_video_blob_id": r.motion_video_blob_id,
                "crop_metadata": r.crop_metadata,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "items": items_json })))
}

/// GET /api/galleries/secure/items
/// List items across ALL of the user's secure galleries (requires unlock token
/// in header).  This is the aggregate feed the clients use to derive the
/// built-in secure smart albums (Secure Gallery / Photos / GIFs / Videos /
/// Audio) without an N+1 per-gallery fetch.
///
/// Each item carries its owning `gallery_id` (+ `gallery_name` for the detail
/// header) so a "remove" from a smart view can route to the real album.
///
/// Token verification is identical to [`list_gallery_items`]; there is no
/// gallery-id ownership check because the scope is simply `g.user_id = ?`.
pub async fn list_all_gallery_items(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify the unlock token — same contract as the per-gallery endpoint.
    let token = headers
        .get("x-gallery-token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::Unauthorized("Gallery token required. Unlock the gallery first.".into())
        })?;

    if !crate::gallery::secure_token::verify(token, &auth.user_id, &state.config.auth.jwt_secret) {
        return Err(AppError::Unauthorized(
            "Invalid or expired gallery token. Unlock the gallery again.".into(),
        ));
    }

    let items = sqlx::query_as::<_, AllGalleryItemRow>(
        "SELECT gi.id, \
                COALESCE(gi.encrypted_blob_id, p.encrypted_blob_id, gi.blob_id) as blob_id, \
                gi.blob_id as clone_blob_id, \
                gi.added_at, \
                gi.gallery_id, \
                g.name as gallery_name, \
                COALESCE(gi.encrypted_thumb_blob_id, p.encrypted_thumb_blob_id, op.encrypted_thumb_blob_id) as encrypted_thumb_blob_id, \
                COALESCE(p.width, op.width) as width, \
                COALESCE(p.height, op.height) as height, \
                COALESCE(p.media_type, op.media_type) as media_type, \
                COALESCE(p.photo_subtype, op.photo_subtype) as photo_subtype, \
                COALESCE(p.burst_id, op.burst_id) as burst_id, \
                COALESCE(p.duration_secs, op.duration_secs) as duration_secs, \
                COALESCE(p.motion_video_blob_id, op.motion_video_blob_id) as motion_video_blob_id, \
                gi.crop_metadata as crop_metadata \
         FROM encrypted_gallery_items gi \
         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
         LEFT JOIN photos p ON p.id = gi.blob_id AND p.encrypted_blob_id IS NOT NULL \
         LEFT JOIN photos op ON op.id = gi.original_blob_id \
         WHERE g.user_id = ? \
         ORDER BY gi.added_at DESC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    // Same ladder hydration as the per-gallery endpoint, unscoped. Both feeds
    // must agree: the smart albums (Secure Videos in particular) are derived
    // from this one, and a picker that appears in the album but not the smart
    // view would read as the album being a different place.
    let mut ladders = crate::transcode::renditions::list_renditions_for_secure_items(
        &state.pool,
        &auth.user_id,
        None,
    )
    .await?;

    let items_json: Vec<serde_json::Value> = collapse_by_clone(&items)
        .into_iter()
        .map(|CollapsedItem { rep: r, galleries }| {
            let galleries_json: Vec<serde_json::Value> = galleries
                .iter()
                .map(|(id, name)| serde_json::json!({ "id": id, "name": name }))
                .collect();
            serde_json::json!({
                "id": r.id,
                "blob_id": r.blob_id,
                "added_at": r.added_at,
                "gallery_id": r.gallery_id,
                "gallery_name": r.gallery_name,
                // Every album this photo is in. Existing clients read only the
                // singular pair above and keep working; a client routing a
                // "remove" needs the full set, because with N memberships
                // "which album am I removing it from" is a real question.
                "galleries": galleries_json,
                "renditions": ladders.remove(&r.id).unwrap_or_default(),
                "encrypted_thumb_blob_id": r.encrypted_thumb_blob_id,
                "width": r.width,
                "height": r.height,
                "media_type": r.media_type,
                "photo_subtype": r.photo_subtype,
                "burst_id": r.burst_id,
                "duration_secs": r.duration_secs,
                "motion_video_blob_id": r.motion_video_blob_id,
                "crop_metadata": r.crop_metadata,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "items": items_json })))
}

/// Request body for `POST /api/galleries/secure/{gallery_id}/items/{item_id}/move`.
#[derive(Debug, Deserialize)]
pub struct MoveGalleryItemRequest {
    /// Destination secure gallery (must be owned by the same user).
    pub target_gallery_id: String,
}

/// POST /api/galleries/secure/:gallery_id/items/:item_id/move
///
/// Move a secure item from one of the user's secure galleries to another (#31,
/// the cross-secure-album picker).  Because a photo may live in **at most one**
/// secure gallery (enforced in [`add_gallery_item`]), pulling media in "from
/// other secure albums" is a MOVE, not a copy — we simply reassign the
/// membership row's `gallery_id`.  No re-clone, no re-encryption: the encrypted
/// blob and the hidden original are untouched, so the one-secure-album invariant
/// and the "original stays hidden" behaviour both hold.
///
/// Ownership of BOTH the source and target gallery is verified first (IDOR
/// guard) — a caller can only shuffle items between galleries they own.
pub async fn move_gallery_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((gallery_id, item_id)): Path<(String, String)>,
    Json(req): Json<MoveGalleryItemRequest>,
) -> Result<StatusCode, AppError> {
    // Verify the caller owns the SOURCE gallery.
    let owns_source: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
            .bind(&gallery_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;
    if owns_source == 0 {
        return Err(AppError::NotFound);
    }

    // Verify the caller owns the TARGET gallery.
    let owns_target: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
            .bind(&req.target_gallery_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;
    if owns_target == 0 {
        return Err(AppError::BadRequest("Target secure album not found".into()));
    }

    // Multi-album membership (Z1) makes a same-album duplicate reachable here in
    // a way it was not when a photo could only live in one gallery: if the
    // TARGET already holds this photo, reassigning would leave it in the target
    // twice.  "At most once per album" is the half of the old invariant that
    // survives, so this is refused rather than merged — silently deleting the
    // source row would be a destructive reading of a request the user made as a
    // move.
    let clone_blob_id: Option<String> = sqlx::query_scalar(
        "SELECT blob_id FROM encrypted_gallery_items WHERE id = ? AND gallery_id = ?",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .fetch_optional(&state.pool)
    .await?;
    let clone_blob_id = clone_blob_id.ok_or(AppError::NotFound)?;

    let already_in_target: bool = sqlx::query_scalar(
        "SELECT EXISTS(\
           SELECT 1 FROM encrypted_gallery_items \
           WHERE gallery_id = ? AND blob_id = ? AND id != ?\
         )",
    )
    .bind(&req.target_gallery_id)
    .bind(&clone_blob_id)
    .bind(&item_id)
    .fetch_one(&state.pool)
    .await?;

    if already_in_target {
        tracing::info!(
            source_gallery_id = %gallery_id,
            target_gallery_id = %req.target_gallery_id,
            item_id = %item_id,
            "[DIAG:SECURE_MOVE] Rejected move — photo already in the target secure album"
        );
        return Err(AppError::Conflict(
            "Photo is already in the target secure album".into(),
        ));
    }

    // Reassign the membership row.  Scoped to (item_id, source gallery_id) so a
    // guessed item id from another gallery can't be moved.
    let result = sqlx::query(
        "UPDATE encrypted_gallery_items SET gallery_id = ? WHERE id = ? AND gallery_id = ?",
    )
    .bind(&req.target_gallery_id)
    .bind(&item_id)
    .bind(&gallery_id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    tracing::info!(
        source_gallery_id = %gallery_id,
        target_gallery_id = %req.target_gallery_id,
        item_id = %item_id,
        "[DIAG:SECURE_MOVE] Moved item between secure galleries"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Request body for `PUT /api/galleries/secure/{gallery_id}/items/{item_id}/crop`.
#[derive(Debug, Deserialize)]
pub struct SetGalleryItemCropRequest {
    /// Crop/edit metadata JSON (same shape as `photos.crop_metadata`), or
    /// `null` to clear all edits.
    pub crop_metadata: Option<String>,
}

/// PUT /api/galleries/secure/:gallery_id/items/:item_id/crop
///
/// Persist non-destructive edit metadata (crop / brightness / rotate / trim) for
/// a secure item (#31).  Stored on the item row itself, not the photos table, so
/// it stays inside the secure domain and never leaks onto the regular-gallery
/// original.  Clients apply it at display time exactly like `photos.crop_metadata`
/// — no re-render / re-encryption of the encrypted blob.
///
/// Ownership of the gallery is verified first (IDOR guard), matching
/// [`add_gallery_item`] / [`remove_gallery_item`].
pub async fn set_gallery_item_crop(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((gallery_id, item_id)): Path<(String, String)>,
    Json(req): Json<SetGalleryItemCropRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let owner: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
            .bind(&gallery_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;
    if owner == 0 {
        return Err(AppError::NotFound);
    }

    let result = sqlx::query(
        "UPDATE encrypted_gallery_items SET crop_metadata = ? WHERE id = ? AND gallery_id = ?",
    )
    .bind(&req.crop_metadata)
    .bind(&item_id)
    .bind(&gallery_id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    tracing::info!(
        gallery_id = %gallery_id,
        item_id = %item_id,
        has_crop = req.crop_metadata.is_some(),
        "[DIAG:SECURE_CROP] Updated secure item crop metadata"
    );

    Ok(Json(serde_json::json!({
        "item_id": item_id,
        "crop_metadata": req.crop_metadata,
    })))
}

#[cfg(test)]
mod tests {
    //! Core SQL behaviour for the #31 move + crop mutations, exercised against
    //! an in-memory DB with the REAL migrations (so migration 032's new
    //! `crop_metadata` column is proven to apply). The handlers themselves need
    //! a full `AppState`; these tests target the exact UPDATE statements the
    //! handlers run, plus their gallery-scoping (the IDOR guard's teeth).
    // The Z1 tests below drive the REAL helpers (`existing_memberships`,
    // `clone_is_shared`, `collapse_by_clone`) rather than a copy of their SQL,
    // which is why this module now imports its parent at all. The older tests
    // here re-type the statement under test — fine for a two-line UPDATE, and
    // exactly the drift this repo has recorded nine times for anything bigger.
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn mem_pool() -> SqlitePool {
        // FKs off: we insert bare gallery/item rows without the full users graph.
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// Insert a secure gallery owned by `user_id`.
    async fn insert_gallery(pool: &SqlitePool, id: &str, user_id: &str) {
        sqlx::query(
            "INSERT INTO encrypted_galleries (id, user_id, name, password_hash, created_at) \
             VALUES (?, ?, ?, 'x', '2026-07-18T00:00:00Z')",
        )
        .bind(id)
        .bind(user_id)
        .bind(format!("gallery-{id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert a membership row into `gallery_id`.
    async fn insert_item(pool: &SqlitePool, item_id: &str, gallery_id: &str) {
        sqlx::query(
            "INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at) \
             VALUES (?, ?, ?, '2026-07-18T00:00:00Z')",
        )
        .bind(item_id)
        .bind(gallery_id)
        .bind(format!("blob-{item_id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert a membership row with an explicit clone `blob_id` — the shape
    /// multi-album membership produces, where several rows share one clone.
    async fn insert_shared_item(
        pool: &SqlitePool,
        item_id: &str,
        gallery_id: &str,
        blob_id: &str,
        original_blob_id: &str,
        added_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at, original_blob_id) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(item_id)
        .bind(gallery_id)
        .bind(blob_id)
        .bind(added_at)
        .bind(original_blob_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Build an aggregate-feed row with only the fields the collapse reads.
    fn feed_row(
        id: &str,
        clone_blob_id: &str,
        gallery_id: &str,
        gallery_name: &str,
        added_at: &str,
    ) -> AllGalleryItemRow {
        AllGalleryItemRow {
            id: id.into(),
            blob_id: clone_blob_id.into(),
            clone_blob_id: clone_blob_id.into(),
            added_at: added_at.into(),
            gallery_id: gallery_id.into(),
            gallery_name: gallery_name.into(),
            encrypted_thumb_blob_id: None,
            width: None,
            height: None,
            media_type: None,
            photo_subtype: None,
            burst_id: None,
            duration_secs: None,
            motion_video_blob_id: None,
            crop_metadata: None,
        }
    }

    // ── Z1: multi-album secure membership ───────────────────────────────────

    #[tokio::test]
    async fn membership_is_found_through_either_id_space() {
        // Android sends the encrypted `blobs` id, web sends the photo id. A
        // second add must recognise the first one's row whichever space it
        // arrives in — the reason SECURE_MEMBERSHIP_MATCH takes two params.
        let pool = mem_pool().await;
        insert_gallery(&pool, "g1", "u1").await;
        insert_shared_item(
            &pool,
            "i1",
            "g1",
            "clone-1",
            "photo-1",
            "2026-08-01T00:00:00Z",
        )
        .await;

        let by_photo = existing_memberships(&pool, "u1", "photo-1", "photo-1")
            .await
            .unwrap();
        assert_eq!(by_photo.len(), 1, "photo-id space must match");

        let by_clone = existing_memberships(&pool, "u1", "unrelated", "clone-1")
            .await
            .unwrap();
        assert_eq!(by_clone.len(), 1, "clone/blob-id space must match");

        let neither = existing_memberships(&pool, "u1", "nope", "nope")
            .await
            .unwrap();
        assert!(
            neither.is_empty(),
            "an unsecured photo must match nothing — otherwise the two \
             assertions above pass for any input"
        );
    }

    #[tokio::test]
    async fn membership_lookup_is_scoped_to_the_owner() {
        // Another user's secure album must not make this user's photo look
        // already-secured (and must never donate its clone to an adoption).
        let pool = mem_pool().await;
        insert_gallery(&pool, "theirs", "u2").await;
        insert_shared_item(
            &pool,
            "i1",
            "theirs",
            "clone-1",
            "photo-1",
            "2026-08-01T00:00:00Z",
        )
        .await;

        let found = existing_memberships(&pool, "u1", "photo-1", "photo-1")
            .await
            .unwrap();
        assert!(
            found.is_empty(),
            "cross-user membership must not be visible"
        );
    }

    #[tokio::test]
    async fn a_photo_can_hold_memberships_in_several_albums() {
        // The core Z1 property: one clone, N membership rows, all discoverable.
        let pool = mem_pool().await;
        insert_gallery(&pool, "g1", "u1").await;
        insert_gallery(&pool, "g2", "u1").await;
        insert_shared_item(
            &pool,
            "i1",
            "g1",
            "clone-1",
            "photo-1",
            "2026-08-01T00:00:00Z",
        )
        .await;
        insert_shared_item(
            &pool,
            "i2",
            "g2",
            "clone-1",
            "photo-1",
            "2026-08-02T00:00:00Z",
        )
        .await;

        let found = existing_memberships(&pool, "u1", "photo-1", "photo-1")
            .await
            .unwrap();
        assert_eq!(found.len(), 2, "the photo is in both albums");
        // Oldest first — `add_gallery_item` adopts `first()`, and adopting the
        // original clone rather than a later one keeps the donor stable.
        assert_eq!(found[0].gallery_id, "g1");
        assert_eq!(found[1].gallery_id, "g2");
        assert!(
            found.iter().all(|m| m.blob_id == "clone-1"),
            "both memberships share ONE clone — a second clone is the storage \
             cost adoption exists to avoid"
        );
    }

    #[tokio::test]
    async fn a_lone_clone_is_not_shared_but_a_sibling_makes_it_shared() {
        // The guard that decides whether remove_gallery_item may destroy bytes.
        let pool = mem_pool().await;
        insert_gallery(&pool, "g1", "u1").await;
        insert_gallery(&pool, "g2", "u1").await;
        insert_shared_item(
            &pool,
            "i1",
            "g1",
            "clone-1",
            "photo-1",
            "2026-08-01T00:00:00Z",
        )
        .await;

        assert!(
            !clone_is_shared(&pool, "u1", "clone-1", "i1").await.unwrap(),
            "the only membership must NOT read as shared, or the clone is \
             never reclaimed and every removal leaks its bytes"
        );

        insert_shared_item(
            &pool,
            "i2",
            "g2",
            "clone-1",
            "photo-1",
            "2026-08-02T00:00:00Z",
        )
        .await;

        assert!(
            clone_is_shared(&pool, "u1", "clone-1", "i1").await.unwrap(),
            "a sibling membership must read as shared — this is the arm that \
             stops removal from one album blanking the photo in the other"
        );
        assert!(
            clone_is_shared(&pool, "u1", "clone-1", "i2").await.unwrap(),
            "and it must hold from either side"
        );
    }

    #[tokio::test]
    async fn clone_sharing_ignores_the_row_being_removed() {
        // Without the `id != ?` exclusion the predicate is TRUE for every
        // removal, so the destruction path becomes unreachable and every
        // secure-album removal silently leaks its clone forever.
        let pool = mem_pool().await;
        insert_gallery(&pool, "g1", "u1").await;
        insert_shared_item(
            &pool,
            "i1",
            "g1",
            "clone-1",
            "photo-1",
            "2026-08-01T00:00:00Z",
        )
        .await;

        assert!(!clone_is_shared(&pool, "u1", "clone-1", "i1").await.unwrap());
    }

    #[tokio::test]
    async fn clone_sharing_is_scoped_to_the_owner() {
        let pool = mem_pool().await;
        insert_gallery(&pool, "mine", "u1").await;
        insert_gallery(&pool, "theirs", "u2").await;
        insert_shared_item(
            &pool,
            "i1",
            "mine",
            "clone-1",
            "photo-1",
            "2026-08-01T00:00:00Z",
        )
        .await;
        insert_shared_item(
            &pool,
            "i2",
            "theirs",
            "clone-1",
            "photo-1",
            "2026-08-02T00:00:00Z",
        )
        .await;

        assert!(
            !clone_is_shared(&pool, "u1", "clone-1", "i1").await.unwrap(),
            "another user's row must not pin this user's clone — clones are \
             never shared across users, so that can only be a bug"
        );
    }

    #[tokio::test]
    async fn move_into_an_album_that_already_holds_the_photo_is_detected() {
        // "At most once per album" is the half of the old invariant that
        // survives; multi-membership is what makes this reachable via move.
        let pool = mem_pool().await;
        insert_gallery(&pool, "src", "u1").await;
        insert_gallery(&pool, "dst", "u1").await;
        insert_shared_item(
            &pool,
            "i1",
            "src",
            "clone-1",
            "photo-1",
            "2026-08-01T00:00:00Z",
        )
        .await;
        insert_shared_item(
            &pool,
            "i2",
            "dst",
            "clone-1",
            "photo-1",
            "2026-08-02T00:00:00Z",
        )
        .await;

        // The exact predicate move_gallery_item runs.
        let already: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM encrypted_gallery_items \
             WHERE gallery_id = ? AND blob_id = ? AND id != ?)",
        )
        .bind("dst")
        .bind("clone-1")
        .bind("i1")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(already, "moving into dst would duplicate the photo there");

        let fresh: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM encrypted_gallery_items \
             WHERE gallery_id = ? AND blob_id = ? AND id != ?)",
        )
        .bind("dst")
        .bind("clone-2")
        .bind("i1")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!fresh, "an unrelated photo must still be movable into dst");
    }

    #[test]
    fn collapse_folds_multi_album_memberships_into_one_tile() {
        // Rows arrive added_at DESC, as the query orders them.
        let rows = vec![
            feed_row("i2", "clone-1", "g2", "Trip", "2026-08-02T00:00:00Z"),
            feed_row("i1", "clone-1", "g1", "Private", "2026-08-01T00:00:00Z"),
        ];
        let out = collapse_by_clone(&rows);

        assert_eq!(out.len(), 1, "one photo in two albums is ONE tile");
        assert_eq!(
            out[0].galleries,
            vec![("g1", "Private"), ("g2", "Trip")],
            "every album the photo is in, oldest membership first"
        );
        assert_eq!(
            out[0].rep.id, "i1",
            "the oldest membership represents the tile, so added_at is when the \
             photo was secured — not when it was filed into a second album"
        );
    }

    #[test]
    fn collapse_keeps_distinct_photos_distinct() {
        // Vacuity guard for the test above: collapsing EVERYTHING to a single
        // tile satisfies `out.len() == 1` while destroying the feed.
        let rows = vec![
            feed_row("i2", "clone-2", "g1", "Private", "2026-08-02T00:00:00Z"),
            feed_row("i1", "clone-1", "g1", "Private", "2026-08-01T00:00:00Z"),
        ];
        let out = collapse_by_clone(&rows);

        assert_eq!(out.len(), 2, "two different photos are two tiles");
        assert_eq!(out[0].rep.id, "i2", "feed order is preserved");
        assert_eq!(out[1].rep.id, "i1");
        assert!(
            out.iter().all(|c| c.galleries.len() == 1),
            "a photo in one album lists exactly one gallery"
        );
    }

    #[test]
    fn collapse_of_a_single_album_feed_changes_nothing() {
        // The no-op property: before anyone uses multi-album membership, this
        // feed must be byte-identical to what it was. If this fails, the
        // collapse is a regression for every existing library.
        let rows = vec![
            feed_row("i3", "clone-3", "g1", "Private", "2026-08-03T00:00:00Z"),
            feed_row("i2", "clone-2", "g1", "Private", "2026-08-02T00:00:00Z"),
            feed_row("i1", "clone-1", "g2", "Trip", "2026-08-01T00:00:00Z"),
        ];
        let out = collapse_by_clone(&rows);

        assert_eq!(out.len(), 3);
        let ids: Vec<&str> = out.iter().map(|c| c.rep.id.as_str()).collect();
        assert_eq!(ids, vec!["i3", "i2", "i1"], "order and count untouched");
    }

    async fn gallery_of(pool: &SqlitePool, item_id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT gallery_id FROM encrypted_gallery_items WHERE id = ?")
            .bind(item_id)
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn migration_032_adds_crop_metadata_column() {
        // If the column didn't exist this query would error, failing the test.
        let pool = mem_pool().await;
        insert_gallery(&pool, "g1", "u1").await;
        insert_item(&pool, "i1", "g1").await;
        let crop: Option<String> =
            sqlx::query_scalar("SELECT crop_metadata FROM encrypted_gallery_items WHERE id = 'i1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(crop.is_none(), "new items start with no crop metadata");
    }

    #[tokio::test]
    async fn move_reassigns_gallery_id() {
        let pool = mem_pool().await;
        insert_gallery(&pool, "src", "u1").await;
        insert_gallery(&pool, "dst", "u1").await;
        insert_item(&pool, "i1", "src").await;

        // The exact statement move_gallery_item runs.
        let res = sqlx::query(
            "UPDATE encrypted_gallery_items SET gallery_id = ? WHERE id = ? AND gallery_id = ?",
        )
        .bind("dst")
        .bind("i1")
        .bind("src")
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(res.rows_affected(), 1);
        assert_eq!(gallery_of(&pool, "i1").await.as_deref(), Some("dst"));
    }

    #[tokio::test]
    async fn move_is_scoped_to_source_gallery() {
        // A move claiming the wrong source gallery must NOT touch the item — this
        // is what stops a guessed item id in another gallery being moved.
        let pool = mem_pool().await;
        insert_gallery(&pool, "src", "u1").await;
        insert_gallery(&pool, "dst", "u1").await;
        insert_item(&pool, "i1", "src").await;

        let res = sqlx::query(
            "UPDATE encrypted_gallery_items SET gallery_id = ? WHERE id = ? AND gallery_id = ?",
        )
        .bind("dst")
        .bind("i1")
        .bind("wrong-source")
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(res.rows_affected(), 0, "wrong source must be a no-op");
        assert_eq!(gallery_of(&pool, "i1").await.as_deref(), Some("src"));
    }

    #[tokio::test]
    async fn set_and_clear_crop_metadata() {
        let pool = mem_pool().await;
        insert_gallery(&pool, "g1", "u1").await;
        insert_item(&pool, "i1", "g1").await;

        let json = r#"{"x":0.1,"y":0.1,"width":0.8,"height":0.8,"rotate":90,"brightness":0}"#;
        let set = sqlx::query(
            "UPDATE encrypted_gallery_items SET crop_metadata = ? WHERE id = ? AND gallery_id = ?",
        )
        .bind(Some(json))
        .bind("i1")
        .bind("g1")
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(set.rows_affected(), 1);
        let stored: Option<String> =
            sqlx::query_scalar("SELECT crop_metadata FROM encrypted_gallery_items WHERE id = 'i1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored.as_deref(), Some(json));

        // Clearing (null) wipes the edit.
        sqlx::query(
            "UPDATE encrypted_gallery_items SET crop_metadata = ? WHERE id = ? AND gallery_id = ?",
        )
        .bind(Option::<String>::None)
        .bind("i1")
        .bind("g1")
        .execute(&pool)
        .await
        .unwrap();
        let cleared: Option<String> =
            sqlx::query_scalar("SELECT crop_metadata FROM encrypted_gallery_items WHERE id = 'i1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(cleared.is_none());
    }
}
