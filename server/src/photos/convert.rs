//! Background media conversion task.
//!
//! Periodically scans for photos/videos/audio that need web-compatible previews
//! but don't have them yet, and converts them in the background using low CPU
//! priority (`nice -n 19`).
//!
//! Video transcoding (MKV/AVI/WMV → MP4) is intentionally deferred to this
//! background task rather than running during scan, since it can take minutes
//! per file.
//!
//! The task can also be woken immediately via a `Notify` handle (e.g. after a
//! scan or upload completes).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sqlx::SqlitePool;
use tokio::sync::Notify;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::scan::{needs_web_preview, generate_web_preview_bg, generate_thumbnail_file};

/// Run a single conversion + thumbnail pass. Returns (converted, thumbnails_generated).
async fn run_conversion_pass(pool: &SqlitePool, storage_root: &PathBuf) -> (u32, u32) {
    // If an encryption migration is in progress, skip this pass entirely.
    // Conversion creates disk-based web previews for plain-mode photos, but
    // photos mid-encryption will get encrypted_blob_id set imminently — any
    // previews we generate now would be wasted work. The migration-done
    // handler triggers a new conversion pass once encryption finishes.
    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "idle".to_string());

    if mig_status != "idle" {
        tracing::info!(
            "Background convert: encryption migration is '{}', deferring conversion",
            mig_status
        );
        return (0, 0);
    }

    // Check if FFmpeg is available
    let has_ffmpeg = super::scan::ffmpeg_available_pub().await;
    if !has_ffmpeg {
        tracing::debug!("Background convert: FFmpeg not available, skipping");
        return (0, 0);
    }

    // Find all photos that need a web preview but don't have one yet.
    // Only consider plain-mode photos (encrypted photos are served through
    // encrypted blobs and don't need disk-based web previews).
    let photos: Vec<(String, String, String, Option<String>, String)> = match sqlx::query_as(
        "SELECT id, file_path, filename, thumb_path, mime_type FROM photos WHERE encrypted_blob_id IS NULL",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Background convert: failed to query photos: {}", e);
            return (0, 0);
        }
    };

    let mut converted = 0u32;
    let mut thumbnails_generated = 0u32;
    for (photo_id, file_path, filename, thumb_path, mime_type) in &photos {
        // Source file must exist for any operation
        let source_path = storage_root.join(file_path);
        if !tokio::fs::try_exists(&source_path).await.unwrap_or(false) {
            continue;
        }

        // Generate web preview if this format needs one and it doesn't exist yet
        if let Some(ext) = needs_web_preview(filename) {
            let preview_path = storage_root.join(format!(
                ".web_previews/{}.web.{}",
                photo_id, ext
            ));
            if !tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
                tracing::info!(
                    photo_id = %photo_id,
                    filename = %filename,
                    target_ext = ext,
                    "Background convert: starting conversion"
                );

                if generate_web_preview_bg(&source_path, &preview_path, ext).await {
                    converted += 1;
                    tracing::info!(
                        photo_id = %photo_id,
                        filename = %filename,
                        "Background convert: conversion complete"
                    );
                } else {
                    tracing::warn!(
                        photo_id = %photo_id,
                        filename = %filename,
                        "Background convert: conversion failed"
                    );
                }

                // Yield to other tasks between conversions
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }

        // Generate thumbnail if missing — thumbnails may not have been
        // created during scan when FFmpeg was unavailable or the file format
        // wasn't supported at that point.
        if let Some(tp) = thumb_path {
            let thumb_abs = storage_root.join(tp);
            if !tokio::fs::try_exists(&thumb_abs).await.unwrap_or(false) {
                if generate_thumbnail_file(&source_path, &thumb_abs, mime_type).await {
                    thumbnails_generated += 1;
                    tracing::debug!(
                        photo_id = %photo_id,
                        "Background convert: generated missing thumbnail"
                    );
                }
            }
        }
    }

    if converted > 0 || thumbnails_generated > 0 {
        tracing::info!(
            "Background convert: converted {} files, generated {} thumbnails this cycle",
            converted, thumbnails_generated
        );
    }

    (converted, thumbnails_generated)
}

/// Run the background conversion loop.
/// Checks for unconverted files every `interval_secs` seconds, or immediately
/// when notified via the `notify` handle.
///
/// The `active` flag is set while the converter has pending work, allowing the
/// conversion-status endpoint to keep the client banner alive even if
/// encryption changes the DB state mid-pass.
pub async fn background_convert_task(
    pool: SqlitePool,
    storage_root: PathBuf,
    interval_secs: u64,
    notify: Arc<Notify>,
    active: Arc<AtomicBool>,
) {
    // Brief startup delay to let the server initialize before background work
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    loop {
        // Wait for either the timer or an explicit trigger
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(interval_secs)) => {},
            _ = notify.notified() => {
                tracing::info!("Background convert: triggered immediately");
                // Small delay so multiple rapid triggers coalesce
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            },
        }

        let (converted, thumbs) = run_conversion_pass(&pool, &storage_root).await;

        if converted > 0 || thumbs > 0 {
            // Real work was done — mark active so the status endpoint
            // reflects ongoing processing.
            active.store(true, Ordering::Release);

            // If we converted any files, run a second pass to generate
            // thumbnails for the newly converted items (the web preview
            // may now be usable as a thumbnail source).
            if converted > 0 {
                tracing::info!("Background convert: running second pass for thumbnails of converted files");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                run_conversion_pass(&pool, &storage_root).await;
            }
        }

        // Always clear the active flag after a pass completes.
        active.store(false, Ordering::Release);
    }
}

/// Admin endpoint: trigger an immediate background conversion pass.
///
/// POST /admin/photos/convert
pub async fn trigger_convert(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Verify admin role
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&_auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    state.convert_notify.notify_one();

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "message": "Conversion triggered" })),
    ))
}

/// Check how many files still need conversion or thumbnails for the current user.
///
/// GET /photos/conversion-status
pub async fn conversion_status(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let storage_root = state.storage_root.read().await.clone();
    let converting = state.conversion_active.load(Ordering::Acquire);

    // Fetch plain-mode photos belonging to this user
    let photos: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, filename, thumb_path FROM photos WHERE encrypted_blob_id IS NULL AND user_id = ?",
    )
    .bind(&_auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let mut pending_conversions = 0u32;
    let mut missing_thumbnails = 0u32;

    for (id, filename, thumb_path) in &photos {
        // Check if format needs a web preview that hasn't been generated
        if let Some(ext) = super::scan::needs_web_preview(filename) {
            let preview_path =
                storage_root.join(format!(".web_previews/{}.web.{}", id, ext));
            if !tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
                pending_conversions += 1;
            }
        }

        // Check if thumbnail file is missing on disk
        if let Some(tp) = thumb_path {
            let thumb_abs = storage_root.join(tp);
            if !tokio::fs::try_exists(&thumb_abs).await.unwrap_or(false) {
                missing_thumbnails += 1;
            }
        }
    }

    Ok(Json(serde_json::json!({
        "pending_conversions": pending_conversions,
        "missing_thumbnails": missing_thumbnails,
        "converting": converting,
    })))
}
