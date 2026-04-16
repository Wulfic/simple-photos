//! Image-crate rendering for the editing engine.
//!
//! Handles crop, rotation, and brightness edits for static images (JPEG, PNG,
//! BMP, GIF, WebP).  Video/audio rendering is handled by [`super::ffmpeg`].

use std::path::Path;

use crate::error::AppError;

use super::models::CropMeta;

/// Render a static image with crop, rotation, and brightness edits.
///
/// The operation runs on a blocking thread (`spawn_blocking`) to avoid
/// starving the async runtime.  The output format is inferred from the
/// destination file extension.
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

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let mut img = image::open(&src)
            .map_err(|e| AppError::Internal(format!("Failed to open image for copy: {e}")))?;

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
            img = img.crop_imm(cx, cy, cw, ch);
        }

        // Rotation
        img = match rot {
            90 => img.rotate90(),
            180 => img.rotate180(),
            270 => img.rotate270(),
            _ => img,
        };

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

        img.save_with_format(&dst, format)
            .map_err(|e| AppError::Internal(format!("Failed to save rendered image copy: {e}")))?;

        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("Image render task panicked: {e}")))?
}
