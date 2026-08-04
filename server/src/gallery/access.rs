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
use crate::http_utils::Confidentiality;
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

/// Returns `true` if `item_id` (a photo id, clone blob id, encrypted blob id,
/// or **video-rendition blob id**) belongs to one of `user_id`'s secure
/// galleries.
///
/// Mirrors the set hidden from the main gallery by
/// `secure::list_secure_blob_ids`, so anything hidden there is also gated here.
///
/// # Derived content is not named by any secure-gallery row
///
/// The first two arms both work by *matching an id a secure-gallery row already
/// records*. A video rendition (#49) breaks that assumption: it is bytes derived
/// from a photo, stored in its own blob, and `encrypted_gallery_items` has no
/// column that will ever mention it. So the third arm resolves the other way —
/// blob → owning photo → is that photo secure.
///
/// Without it, securing a 4K video that already had a 1080p rung left the rung's
/// blob fetchable with nothing but an account session: the ladder only generates
/// for *eligible* (non-secure) photos, so the exposure is created by securing a
/// video **after** its rung was produced, which is ordinary use rather than an
/// edge case. That is a full-quality copy of the video the secure album exists
/// to hide.
///
/// That third arm's correlation is
/// [`SECURE_ITEM_RENDITION_MATCH`](crate::transcode::renditions::SECURE_ITEM_RENDITION_MATCH),
/// shared verbatim with the secure listing that *offers* those rungs to a
/// picker. Gate and offer must be derived from one expression: a listing that
/// matched more broadly than the gate would publish an ungated blob id.
pub async fn is_secure_item(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    item_id: &str,
) -> Result<bool, AppError> {
    use crate::transcode::renditions::SECURE_ITEM_RENDITION_MATCH;

    // `?1` = user_id, `?2` = item_id (each referenced several times).
    let found: bool = sqlx::query_scalar(&format!(
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
           OR EXISTS(\
             SELECT 1 FROM video_renditions r \
             JOIN encrypted_gallery_items gi ON {SECURE_ITEM_RENDITION_MATCH} \
             JOIN encrypted_galleries g3 ON g3.id = gi.gallery_id \
             WHERE g3.user_id = ?1 AND r.blob_id = ?2 \
           ) \
         )"
    ))
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
///
/// # The return value is not incidental
///
/// Returns **which kind of content the caller is about to serve**, so a handler
/// that has already paid for the [`is_secure_item`] query can also use the
/// answer to pick its `Cache-Control` ([`Confidentiality`]). Secure media must
/// never be written to a client-side cache: a browser cache entry is a plaintext
/// copy on disk that outlives both the unlock token and the session, so caching
/// a decrypted secure photo defeats the album as thoroughly as never encrypting
/// it.
///
/// This is deliberately *returned* rather than re-derived at header-building
/// time. Two derivations of "is this item secure" is the exact failure shape
/// `todo.md` tracks eight instances of — and here the drift would be a
/// confidentiality bug, not a counting one.
pub async fn require_secure_access(
    state: &AppState,
    user_id: &str,
    item_id: &str,
    token: &GalleryToken,
) -> Result<Confidentiality, AppError> {
    if !is_secure_item(&state.read_pool, user_id, item_id).await? {
        return Ok(Confidentiality::Cacheable);
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

    Ok(Confidentiality::Secure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// Foreign keys ON: `video_renditions.photo_id` cascades from `photos`, and
    /// these tests are about the relationship between the two tables.
    async fn test_pool() -> sqlx::SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at) \
             VALUES ('u1', 'u1', 'x', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn insert_blob(pool: &sqlx::SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
             VALUES (?, 'u1', 'video', 1024, '2026-01-01T00:00:00Z', ?)",
        )
        .bind(id)
        .bind(format!("blobs/u1/{id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    /// A 4K video with its own encrypted blob.
    async fn insert_video(pool: &sqlx::SqlitePool, id: &str, enc_blob: &str) {
        insert_blob(pool, enc_blob).await;
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, encrypted_blob_id, created_at) \
             VALUES (?, 'u1', ?, '', 'video/mp4', 'video', 0, 3840, 2160, ?, \
                     '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("{id}.mp4"))
        .bind(enc_blob)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Record a produced 1080p rung owning its own blob.
    async fn insert_rendition(pool: &sqlx::SqlitePool, photo_id: &str, blob_id: &str) {
        insert_blob(pool, blob_id).await;
        sqlx::query(
            "INSERT INTO video_renditions (photo_id, short_edge, width, height, is_source, \
             blob_id, size_bytes, created_at) \
             VALUES (?, 1080, 1920, 1080, 0, ?, 2048, '2026-01-01T00:00:00Z')",
        )
        .bind(photo_id)
        .bind(blob_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Hide a photo in a secure gallery, exactly as `gallery::secure` does: it
    /// inserts an `encrypted_gallery_items` row naming the photo id and never
    /// touches `photos`.
    async fn secure_hide(pool: &sqlx::SqlitePool, photo_id: &str) {
        sqlx::query(
            "INSERT OR IGNORE INTO encrypted_galleries (id, user_id, name, password_hash, \
             created_at) VALUES ('g1', 'u1', 'vault', 'x', '2026-01-01T00:00:00Z')",
        )
        .execute(pool)
        .await
        .unwrap();
        // The first eligibility arm matches a photo id against `blob_id`, which
        // carries an FK to `blobs` — so a secured photo's id is also a blob id.
        insert_blob(pool, photo_id).await;
        sqlx::query(
            "INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at) \
             VALUES ('i1', 'g1', ?, '2026-01-01T00:00:00Z')",
        )
        .bind(photo_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// **The exposure this arm exists to close.**
    ///
    /// The ladder only generates rungs for photos that are *eligible*, i.e. not
    /// already secure. So the dangerous ordering is the ordinary one: a 4K video
    /// gets its 1080p rung, and the user secures it afterwards. Every id the
    /// secure gallery records is gated — but the rung lives in a blob nothing
    /// records, and the picker hands that blob id straight to clients.
    ///
    /// Verified RED against the two-arm predicate: `is_secure_item` returns
    /// false and `GET /api/blobs/{rung}` streams a full 1080p copy of the video
    /// to any authenticated session, with no unlock token.
    #[tokio::test]
    async fn a_rendition_blob_of_a_secured_video_requires_the_unlock_token() {
        let pool = test_pool().await;
        insert_video(&pool, "p1", "pb1").await;
        insert_rendition(&pool, "p1", "rb1").await;
        secure_hide(&pool, "p1").await;

        // Precondition: the ids the gallery *does* record are already gated, so
        // a failure below is specifically about derived content.
        assert!(is_secure_item(&pool, "u1", "p1").await.unwrap());
        assert!(is_secure_item(&pool, "u1", "pb1").await.unwrap());

        assert!(
            is_secure_item(&pool, "u1", "rb1").await.unwrap(),
            "the 1080p rung of a secured video is a full-quality copy of it; \
             leaving it ungated defeats the secure album entirely"
        );
    }

    /// The other half — and the one that would break every video in the library
    /// if the arm were written too broadly. An ordinary video's rung must stay
    /// freely fetchable, or the picker 401s on content that was never secure.
    #[tokio::test]
    async fn a_rendition_blob_of_an_ordinary_video_is_not_gated() {
        let pool = test_pool().await;
        insert_video(&pool, "p_open", "pb_open").await;
        insert_rendition(&pool, "p_open", "rb_open").await;
        // A *different* photo is secured, so the gallery tables are non-empty —
        // an arm that forgot to correlate on photo_id would pass a test where
        // nothing at all was secure.
        insert_video(&pool, "p_secret", "pb_secret").await;
        secure_hide(&pool, "p_secret").await;

        assert!(!is_secure_item(&pool, "u1", "rb_open").await.unwrap());
        assert!(!is_secure_item(&pool, "u1", "pb_open").await.unwrap());
    }

    /// A source rung points at the photo's *own* blob rather than owning bytes
    /// (`037` guards the orphan trigger on exactly this). Gating it must
    /// therefore agree with gating the photo's encrypted blob directly — the two
    /// name the same bytes, and disagreeing would mean the same content is
    /// reachable through one id and not the other.
    #[tokio::test]
    async fn a_source_rung_agrees_with_the_photos_own_blob() {
        let pool = test_pool().await;
        insert_video(&pool, "p1", "pb1").await;
        sqlx::query(
            "INSERT INTO video_renditions (photo_id, short_edge, width, height, is_source, \
             blob_id, size_bytes, created_at) \
             VALUES ('p1', 2160, 3840, 2160, 1, 'pb1', 4096, '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(!is_secure_item(&pool, "u1", "pb1").await.unwrap());
        secure_hide(&pool, "p1").await;
        assert!(is_secure_item(&pool, "u1", "pb1").await.unwrap());
    }

    /// Another user's secure gallery must not gate my blobs, and — more
    /// importantly — must not be consultable at all. Ownership is checked by the
    /// caller; this pins that the predicate itself is user-scoped.
    #[tokio::test]
    async fn the_rendition_arm_is_scoped_to_the_requesting_user() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at) \
             VALUES ('u2', 'u2', 'x', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        insert_video(&pool, "p1", "pb1").await;
        insert_rendition(&pool, "p1", "rb1").await;
        secure_hide(&pool, "p1").await;

        assert!(is_secure_item(&pool, "u1", "rb1").await.unwrap());
        assert!(
            !is_secure_item(&pool, "u2", "rb1").await.unwrap(),
            "u1's secure gallery must not gate a lookup made as u2"
        );
    }
}
