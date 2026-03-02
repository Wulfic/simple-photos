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

use super::scan::{needs_web_preview, generate_web_preview_bg};

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
        let photos: Vec<(String, String, String)> = match sqlx::query_as(
            "SELECT id, file_path, filename FROM photos WHERE encrypted_blob_id IS NULL",
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
        for (photo_id, file_path, filename) in &photos {
            // Check if this file needs a web preview
            let ext = match needs_web_preview(filename) {
                Some(e) => e,
                None => continue,
            };

            // Check if the preview already exists on disk
            let preview_path = storage_root.join(format!(
                ".web_previews/{}.web.{}",
                photo_id, ext
            ));
            if tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
                continue; // Already converted
            }

            // Source file must exist
            let source_path = storage_root.join(file_path);
            if !tokio::fs::try_exists(&source_path).await.unwrap_or(false) {
                continue;
            }

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

            // Yield to other tasks between conversions to keep the server responsive
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        if converted > 0 {
            tracing::info!("Background convert: converted {} files this cycle", converted);
        }
    }
}
