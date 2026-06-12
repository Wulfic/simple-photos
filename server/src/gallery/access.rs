//! Serve-path access control for secure-gallery items.
//!
//! Secure-gallery clones are ordinary `photos`/`blobs` rows owned by the user,
//! so the generic media endpoints (`/api/photos/{id}/file`, `/api/blobs/{id}`,
//! …) would otherwise serve them to any authenticated session — bypassing the
//! password re-prompt that the secure gallery is supposed to enforce.
//!
//! This module gates those endpoints: when the requested id belongs to a
//! secure gallery, the caller must additionally present a valid unlock token
//! (see [`crate::gallery::secure_token`]). Non-secure items are unaffected.
//!
//! The token may arrive either as the `X-Gallery-Token` header (used by JSON
//! API calls) or as a `gallery_token` query parameter — the latter is required
//! because `<img>` / `<video>` elements cannot set custom headers.

use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::error::AppError;
use crate::state::AppState;

/// Extracted secure-gallery unlock token, if the request carried one.
///
/// Resolution order: `X-Gallery-Token` header, then `?gallery_token=` query
/// parameter. Absence is **not** an extractor error — enforcement happens in
/// [`require_secure_access`], which only rejects when the item is actually
/// secure.
pub struct GalleryToken(pub Option<String>);

impl FromRequestParts<AppState> for GalleryToken {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // 1. Header (standard for fetch()-based API calls).
        if let Some(v) = parts
            .headers
            .get("x-gallery-token")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
        {
            return Ok(GalleryToken(Some(v.to_string())));
        }

        // 2. Query parameter (for <img>/<video> src URLs that can't set headers).
        if let Some(query) = parts.uri.query() {
            for pair in query.split('&') {
                if let Some(value) = pair.strip_prefix("gallery_token=") {
                    if !value.is_empty() {
                        // Tokens are `sec_<digits>_<hex>` — URL-safe, so no
                        // percent-decoding is required.
                        return Ok(GalleryToken(Some(value.to_string())));
                    }
                }
            }
        }

        Ok(GalleryToken(None))
    }
}

/// Returns `true` if `item_id` (a photo id, clone blob id, or encrypted blob
/// id) belongs to one of `user_id`'s secure galleries.
///
/// Mirrors the set hidden from the main gallery by
/// `secure::list_secure_blob_ids`, so anything hidden there is also gated here.
pub async fn is_secure_item(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    item_id: &str,
) -> Result<bool, AppError> {
    // `?1` = user_id, `?2` = item_id (each referenced several times).
    let found: bool = sqlx::query_scalar(
        "SELECT (\
           EXISTS(\
             SELECT 1 FROM encrypted_gallery_items gi \
             JOIN encrypted_galleries g ON g.id = gi.gallery_id \
             WHERE g.user_id = ?1 AND ( \
                  gi.blob_id = ?2 \
               OR gi.original_blob_id = ?2 \
               OR gi.encrypted_blob_id = ?2 \
               OR gi.encrypted_thumb_blob_id = ?2 \
             ) \
           ) \
           OR EXISTS(\
             SELECT 1 FROM photos p \
             JOIN encrypted_gallery_items gi2 \
               ON (p.id = gi2.blob_id OR p.id = gi2.original_blob_id) \
             JOIN encrypted_galleries g2 ON g2.id = gi2.gallery_id \
             WHERE g2.user_id = ?1 AND p.user_id = ?1 \
               AND (p.encrypted_blob_id = ?2 OR p.encrypted_thumb_blob_id = ?2) \
           ) \
         )",
    )
    .bind(user_id)
    .bind(item_id)
    .fetch_one(pool)
    .await?;

    Ok(found)
}

/// Enforce secure-gallery access for `item_id`.
///
/// If the item is not in a secure gallery this is a no-op. If it is, a valid,
/// unexpired unlock token for `user_id` must be present, otherwise `401` is
/// returned. Call this *after* the handler's own ownership-scoped lookup so a
/// genuine 404 still takes precedence (no existence oracle).
pub async fn require_secure_access(
    state: &AppState,
    user_id: &str,
    item_id: &str,
    token: &GalleryToken,
) -> Result<(), AppError> {
    if !is_secure_item(&state.read_pool, user_id, item_id).await? {
        return Ok(());
    }

    let provided = token.0.as_deref().ok_or_else(|| {
        AppError::Unauthorized(
            "This item is in a secure album. Unlock the album to view it.".into(),
        )
    })?;

    if !crate::gallery::secure_token::verify(provided, user_id, &state.config.auth.jwt_secret) {
        return Err(AppError::Unauthorized(
            "Invalid or expired gallery token. Unlock the secure album again.".into(),
        ));
    }

    Ok(())
}
