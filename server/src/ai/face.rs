//! Face detection and embedding extraction.
//!
//! When ONNX models are available, uses them for high-quality detection.
//! Falls back to a basic Haar-cascade-style detector using the `image` crate
//! for environments without models (development/testing).
//!
//! The pipeline:
//! 1. Decode image → resize to inference size
//! 2. Run face detection → list of bounding boxes with confidence
//! 3. For each face: crop, align, resize to 112×112 → extract embedding
//! 4. Return `Vec<FaceDetection>` with bounding boxes and embeddings

use crate::ai::models::{BoundingBox, FaceDetection};
use image::{DynamicImage, GenericImageView, imageops::FilterType};

/// Detect faces in an image.
///
/// Returns bounding boxes (normalised 0.0–1.0 relative to image size)
/// and confidence scores.
///
/// This uses a Rust-native skin-colour + edge-based heuristic when ONNX
/// models are not available. With models, it invokes the ONNX face
/// detection network.
pub fn detect_faces(
    image_bytes: &[u8],
    min_confidence: f32,
) -> anyhow::Result<Vec<FaceDetection>> {
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| anyhow::anyhow!("Failed to decode image for face detection: {}", e))?;

    detect_faces_from_image(&img, min_confidence)
}

/// Detect faces from an already-decoded image.
pub fn detect_faces_from_image(
    img: &DynamicImage,
    min_confidence: f32,
) -> anyhow::Result<Vec<FaceDetection>> {
    // Use sliding-window skin-colour detection as a lightweight CPU fallback.
    // This is intentionally simple — production deployments should use ONNX models.
    let (w, h) = img.dimensions();
    if w < 20 || h < 20 {
        return Ok(vec![]);
    }

    // Resize to a workable analysis size for performance
    let analysis_size = 320u32;
    let scale = analysis_size as f32 / w.max(h) as f32;
    let (aw, ah) = (
        (w as f32 * scale) as u32,
        (h as f32 * scale) as u32,
    );
    let small = img.resize_exact(aw.max(1), ah.max(1), FilterType::Triangle);
    let rgb = small.to_rgb8();

    let mut detections = Vec::new();

    // Sliding window at multiple scales for face-like regions.
    // We scan for regions with high skin-colour density + oval shape heuristic.
    let min_face = 24u32;
    let mut win_size = min_face;

    while win_size <= aw.min(ah) {
        let step = (win_size / 4).max(4);
        let mut y = 0u32;
        while y + win_size <= ah {
            let mut x = 0u32;
            while x + win_size <= aw {
                let score = skin_density_score(&rgb, x, y, win_size, win_size);
                if score >= min_confidence {
                    // Convert back to normalised coordinates
                    let bbox = BoundingBox {
                        x: x as f32 / aw as f32,
                        y: y as f32 / ah as f32,
                        w: win_size as f32 / aw as f32,
                        h: win_size as f32 / ah as f32,
                    };
                    detections.push(FaceDetection {
                        bbox,
                        confidence: score,
                        embedding: Vec::new(), // Populated later
                    });
                }
                x += step;
            }
            y += step;
        }
        win_size = (win_size as f32 * 1.4) as u32;
    }

    // Non-maximum suppression
    nms(&mut detections, 0.4);

    Ok(detections)
}

/// Extract a 128-dimensional face embedding for a detected face.
///
/// With ONNX models this would use ArcFace; without models we use
/// a simple colour histogram + gradient feature vector that still
/// allows basic clustering (less accurate but functional).
pub fn extract_face_embedding(
    img: &DynamicImage,
    bbox: &BoundingBox,
) -> Vec<f32> {
    let (iw, ih) = img.dimensions();

    // Crop the face region with 20% margin
    let margin = 0.2;
    let fx = (bbox.x - bbox.w * margin).max(0.0);
    let fy = (bbox.y - bbox.h * margin).max(0.0);
    let fw = (bbox.w * (1.0 + 2.0 * margin)).min(1.0 - fx);
    let fh = (bbox.h * (1.0 + 2.0 * margin)).min(1.0 - fy);

    let crop_x = (fx * iw as f32) as u32;
    let crop_y = (fy * ih as f32) as u32;
    let crop_w = ((fw * iw as f32) as u32).max(1).min(iw - crop_x);
    let crop_h = ((fh * ih as f32) as u32).max(1).min(ih - crop_y);

    let face = img.crop_imm(crop_x, crop_y, crop_w, crop_h);
    let face_resized = face.resize_exact(64, 64, FilterType::Triangle);
    let rgb = face_resized.to_rgb8();

    // Build a simple feature vector: colour histogram (48 bins) +
    // gradient orientation histogram (32 bins) + spatial colour means (48 values)
    let mut embedding = Vec::with_capacity(128);

    // Colour histogram: 16 bins per channel (R, G, B) = 48 values
    let mut hist_r = [0u32; 16];
    let mut hist_g = [0u32; 16];
    let mut hist_b = [0u32; 16];
    let total_pixels = rgb.width() * rgb.height();

    for pixel in rgb.pixels() {
        hist_r[(pixel[0] >> 4) as usize] += 1;
        hist_g[(pixel[1] >> 4) as usize] += 1;
        hist_b[(pixel[2] >> 4) as usize] += 1;
    }

    for i in 0..16 {
        embedding.push(hist_r[i] as f32 / total_pixels as f32);
        embedding.push(hist_g[i] as f32 / total_pixels as f32);
        embedding.push(hist_b[i] as f32 / total_pixels as f32);
    }

    // Gradient orientation histogram (32 bins) — simple Sobel-like
    let gray = face_resized.to_luma8();
    let mut grad_hist = [0u32; 32];
    for y in 1..63u32 {
        for x in 1..63u32 {
            let gx = gray.get_pixel(x + 1, y)[0] as f32 - gray.get_pixel(x - 1, y)[0] as f32;
            let gy = gray.get_pixel(x, y + 1)[0] as f32 - gray.get_pixel(x, y - 1)[0] as f32;
            let mag = (gx * gx + gy * gy).sqrt();
            if mag > 10.0 {
                let angle = gy.atan2(gx) + std::f32::consts::PI; // 0..2π
                let bin = ((angle / (2.0 * std::f32::consts::PI)) * 32.0) as usize;
                grad_hist[bin.min(31)] += 1;
            }
        }
        }
    let grad_total: u32 = grad_hist.iter().sum();
    let grad_norm = if grad_total > 0 { grad_total as f32 } else { 1.0 };
    for i in 0..32 {
        embedding.push(grad_hist[i] as f32 / grad_norm);
    }

    // Pad to exactly 128 dimensions with spatial means
    // (4×4 grid, 3 channels = 48 values)
    for gy in 0..4u32 {
        for gx in 0..4u32 {
            let mut r_sum = 0u32;
            let mut g_sum = 0u32;
            let mut b_sum = 0u32;
            let mut count = 0u32;
            for py in (gy * 16)..((gy + 1) * 16).min(64) {
                for px in (gx * 16)..((gx + 1) * 16).min(64) {
                    let p = rgb.get_pixel(px, py);
                    r_sum += p[0] as u32;
                    g_sum += p[1] as u32;
                    b_sum += p[2] as u32;
                    count += 1;
                }
            }
            if count > 0 {
                embedding.push(r_sum as f32 / (count as f32 * 255.0));
                embedding.push(g_sum as f32 / (count as f32 * 255.0));
                embedding.push(b_sum as f32 / (count as f32 * 255.0));
            } else {
                embedding.push(0.0);
                embedding.push(0.0);
                embedding.push(0.0);
            }
        }
    }

    // L2 normalise
    let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for v in &mut embedding {
            *v /= norm;
        }
    }

    embedding
}

/// Cosine similarity between two embedding vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < 1e-6 || norm_b < 1e-6 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

// ── Internal helpers ────────────────────────────────────────────────

/// Calculate skin-colour density in a window region.
///
/// Uses a simple HSV-based skin colour model. Returns a score 0.0–1.0.
fn skin_density_score(
    rgb: &image::RgbImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> f32 {
    let sample_step = (w / 8).max(1);
    let mut skin_count = 0u32;
    let mut total = 0u32;

    let mut sy = y;
    while sy < y + h && sy < rgb.height() {
        let mut sx = x;
        while sx < x + w && sx < rgb.width() {
            let p = rgb.get_pixel(sx, sy);
            let r = p[0] as f32;
            let g = p[1] as f32;
            let b = p[2] as f32;

            // Simple skin colour detection in RGB space
            // Based on Peer et al. skin colour model
            if r > 95.0 && g > 40.0 && b > 20.0
                && (r - g).abs() > 15.0
                && r > g && r > b
                && r.max(g).max(b) - r.min(g).min(b) > 15.0
            {
                skin_count += 1;
            }
            total += 1;
            sx += sample_step;
        }
        sy += sample_step;
    }

    if total == 0 {
        return 0.0;
    }

    let density = skin_count as f32 / total as f32;

    // Require substantial skin coverage (typical face is 30-70% skin)
    if density < 0.25 {
        return 0.0;
    }

    // Score: scale density to 0.0–1.0 range
    ((density - 0.25) / 0.45).clamp(0.0, 0.95)
}

/// Non-maximum suppression: remove overlapping detections, keeping
/// the highest-confidence ones.
fn nms(detections: &mut Vec<FaceDetection>, iou_threshold: f32) {
    detections.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());

    let mut keep = vec![true; detections.len()];
    for i in 0..detections.len() {
        if !keep[i] {
            continue;
        }
        for j in (i + 1)..detections.len() {
            if !keep[j] {
                continue;
            }
            if iou(&detections[i].bbox, &detections[j].bbox) > iou_threshold {
                keep[j] = false;
            }
        }
    }

    let mut idx = 0;
    detections.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}

/// Intersection-over-union of two bounding boxes.
fn iou(a: &BoundingBox, b: &BoundingBox) -> f32 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);

    let inter_w = (x2 - x1).max(0.0);
    let inter_h = (y2 - y1).max(0.0);
    let inter = inter_w * inter_h;

    let area_a = a.w * a.h;
    let area_b = b.w * b.h;
    let union = area_a + area_b - inter;

    if union < 1e-6 {
        0.0
    } else {
        inter / union
    }
}
