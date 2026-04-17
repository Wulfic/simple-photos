//! Image-crate rendering for the editing engine.
//!
//! Handles crop, rotation, and brightness edits for static images (JPEG, PNG,
//! BMP, GIF, WebP).  Video/audio rendering is handled by [`super::ffmpeg`].
//!
//! **EXIF orientation** is applied before any user edits.  The `image` crate's
//! `open()` loads raw pixel data without consulting EXIF orientation, so we
//! must rotate/flip the image first to match what the user sees on screen,
//! then apply their crop/rotation/brightness on top.

use std::path::Path;

use crate::error::AppError;
use crate::photos::thumbnail::apply_exif_orientation;

use super::models::CropMeta;

/// Render a static image with crop, rotation, and brightness edits.
///
/// The operation runs on a blocking thread (`spawn_blocking`) to avoid
/// starving the async runtime.  The output format is inferred from the
/// destination file extension.
///
/// **Important:** EXIF orientation is applied first so the pixel layout
/// matches the displayed orientation.  The user's edits (crop, rotation,
/// brightness) are then layered on top.
pub async fn render_image(
    source: &Path,
    dest: &Path,
    meta: &CropMeta,
) -> Result<(), AppError> {
    let src = source.to_path_buf();
    let dst = dest.to_path_buf();
    let x = meta.x.unwrap_or(0.0);
    let y = meta.y.unwrap_or(0.0);
    let w = meta.width.unwrap_or(1.0);
    let h = meta.height.unwrap_or(1.0);
    let rot = meta.rotation_degrees();
    let brightness = meta.brightness.unwrap_or(0.0);

    tracing::info!(
        "[editing/image_render] Starting render: src={}, dst={}, \
         crop=({:.4},{:.4},{:.4},{:.4}), rotate={}°, brightness={:.1}",
        src.display(),
        dst.display(),
        x, y, w, h,
        rot,
        brightness,
    );

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let mut img = image::open(&src)
            .map_err(|e| AppError::Internal(format!("Failed to open image for copy: {e}")))?;

        let raw_w = img.width();
        let raw_h = img.height();

        // ── Apply EXIF orientation FIRST ──────────────────────────────────
        // image::open() returns raw pixel data; the displayed orientation
        // may differ due to the EXIF orientation tag.  We must rotate/flip
        // so that the pixel layout matches what the user sees before we
        // apply their edits (crop coords, rotation, etc.).
        img = apply_exif_orientation(&src, img);

        let exif_w = img.width();
        let exif_h = img.height();
        let exif_changed = raw_w != exif_w || raw_h != exif_h;

        tracing::info!(
            "[editing/image_render] Source: raw={}×{}, after_exif={}×{} (changed={})",
            raw_w, raw_h, exif_w, exif_h, exif_changed,
        );

        let iw = img.width() as f64;
        let ih = img.height() as f64;

        // Crop (fractional coordinates, clamped to image bounds)
        if w < 0.999 || h < 0.999 || x > 0.001 || y > 0.001 {
            let cx = ((x * iw).round() as u32).min(img.width().saturating_sub(1));
            let cy = ((y * ih).round() as u32).min(img.height().saturating_sub(1));
            let max_w = img.width().saturating_sub(cx);
            let max_h = img.height().saturating_sub(cy);
            let cw = ((w * iw).round().max(1.0) as u32).min(max_w).max(1);
            let ch = ((h * ih).round().max(1.0) as u32).min(max_h).max(1);

            tracing::info!(
                "[editing/image_render] Crop: frac=({:.4},{:.4},{:.4},{:.4}) → \
                 px=({},{},{},{}) on {}×{} canvas",
                x, y, w, h, cx, cy, cw, ch, img.width(), img.height(),
            );

            img = img.crop_imm(cx, cy, cw, ch);

            tracing::info!(
                "[editing/image_render] After crop: {}×{}",
                img.width(), img.height(),
            );
        }

        // Rotation
        if rot > 0 {
            let pre_rot_w = img.width();
            let pre_rot_h = img.height();
            img = match rot {
                90 => img.rotate90(),
                180 => img.rotate180(),
                270 => img.rotate270(),
                _ => img,
            };
            tracing::info!(
                "[editing/image_render] Rotation {}°: {}×{} → {}×{}",
                rot, pre_rot_w, pre_rot_h, img.width(), img.height(),
            );
        }

        // Brightness — additive offset on pixel channels (0–255).
        //
        // The client JSON `brightness` field ranges from -100 (black) to +100
        // (white).  `image::imageops::brighten` adds an i32 to each pixel
        // channel, so we map the -100…+100 range to roughly -255…+255.
        //
        // NOTE: This is an *additive* operation, whereas the CSS/Android live
        // preview uses a *multiplicative* `brightness()` filter.  The visual
        // results diverge for extreme values; see `editing/mod.rs` for the
        // full parity table.
        if brightness.abs() > 0.5 {
            tracing::info!(
                "[editing/image_render] Brightness: {:.1} → brighten({})",
                brightness, (brightness * 2.55) as i32,
            );
            img = image::DynamicImage::ImageRgba8(
                image::imageops::brighten(&img, (brightness * 2.55) as i32),
            );
        }

        // Determine output format from extension
        let ext = dst.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
        let format = match ext.to_ascii_lowercase().as_str() {
            "png" => image::ImageFormat::Png,
            "gif" => image::ImageFormat::Gif,
            "webp" => image::ImageFormat::WebP,
            "bmp" => image::ImageFormat::Bmp,
            _ => image::ImageFormat::Jpeg,
        };

        tracing::info!(
            "[editing/image_render] Saving: {}×{} as {:?} → {}",
            img.width(), img.height(), format, dst.display(),
        );

        img.save_with_format(&dst, format)
            .map_err(|e| AppError::Internal(format!("Failed to save rendered image copy: {e}")))?;

        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("Image render task panicked: {e}")))?
}
