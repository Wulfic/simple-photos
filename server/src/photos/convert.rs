//! Background media conversion task.
//!
//! Periodically scans for photos/videos/audio that need web-compatible previews
//! but don't have them yet, and converts them in the background using low CPU
//! priority (`nice -n 19`).
//!
//! Video transcoding (MKV/AVI/WMV → MP4) is intentionally deferred to this
//! background task rather than running during scan, since it can take minutes
//! per file.

use std::path::PathBuf;
use sqlx::SqlitePool;

use super::scan::{needs_web_preview, generate_web_preview_bg, generate_thumbnail_file};

/// Run the background conversion loop.
/// Checks for unconverted files every `interval_secs` seconds.
pub async fn background_convert_task(
    pool: SqlitePool,
    storage_root: PathBuf,
    interval_secs: u64,
) {
    // Brief startup delay to let the server initialize before background work
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));

    loop {
        interval.tick().await;

        // Check if FFmpeg is available
        let has_ffmpeg = super::scan::ffmpeg_available_pub().await;
        if !has_ffmpeg {
            tracing::debug!("Background convert: FFmpeg not available, skipping");
            continue;
        }

        // Find all photos that need a web preview but don't have one yet
        let photos: Vec<(String, String, String, Option<String>, String)> = match sqlx::query_as(
            "SELECT id, file_path, filename, thumb_path, mime_type FROM photos WHERE encrypted_blob_id IS NULL",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!("Background convert: failed to query photos: {}", e);
                continue;
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
    }
}
