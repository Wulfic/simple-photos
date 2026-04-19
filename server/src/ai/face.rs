//! Face detection and embedding extraction.
//!
//! Uses the UltraFace-RFB-320 ONNX model via tract for accurate face detection.
//! Falls back to a conservative heuristic detector if models are unavailable.
//!
//! The pipeline:
//! 1. Decode image → resize to 320×240 (model input size)
//! 2. Run face detection → bounding boxes with confidence
//! 3. For each face: crop, resize to 64×64 → extract embedding
//! 4. Return `Vec<FaceDetection>` with bounding boxes and embeddings

use crate::ai::models::{BoundingBox, FaceDetection};
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use tracing;

// ── Model loading ───────────────────────────────────────────────────

/// UltraFace ONNX model input dimensions.
const MODEL_WIDTH: usize = 320;
const MODEL_HEIGHT: usize = 240;

/// URL for downloading the UltraFace-RFB-320 model.
const MODEL_URL: &str = "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx";
const MODEL_FILENAME: &str = "ultraface-RFB-320.onnx";

/// Thread-safe cached model. Loaded once on first use.
static FACE_MODEL: OnceLock<Option<Arc<FaceModel>>> = OnceLock::new();

struct FaceModel {
    model: tract_onnx::prelude::TypedRunnableModel<tract_onnx::prelude::TypedModel>,
}

// SAFETY: tract models are internally thread-safe for inference
unsafe impl Send for FaceModel {}
unsafe impl Sync for FaceModel {}

/// Initialise face model from the given model directory.
/// Downloads the model if it doesn't exist.
/// Call once during startup; subsequent calls are no-ops.
pub fn init_face_model(model_dir: &str) {
    FACE_MODEL.get_or_init(|| {
        let dir = Path::new(model_dir);
        let model_path = dir.join(MODEL_FILENAME);

        if !model_path.exists() {
            tracing::info!("Face model not found at {:?}, attempting download...", model_path);
            if let Err(e) = download_model(&model_path) {
                tracing::warn!("Failed to download face model: {}. Using heuristic fallback.", e);
                return None;
            }
        }

        match load_onnx_model(&model_path) {
            Ok(m) => {
                tracing::info!("UltraFace ONNX model loaded from {:?}", model_path);
                Some(Arc::new(m))
            }
            Err(e) => {
                tracing::warn!("Failed to load face model: {}. Using heuristic fallback.", e);
                None
            }
        }
    });
}

fn download_model(dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {}", e))?;
    }

    tracing::info!("Downloading UltraFace model from {}...", MODEL_URL);
    let resp = reqwest::blocking::get(MODEL_URL).map_err(|e| format!("download: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let bytes = resp.bytes().map_err(|e| format!("read: {}", e))?;
    if bytes.len() < 100_000 {
        return Err("Downloaded file too small, likely an error page".into());
    }

    std::fs::write(dest, &bytes).map_err(|e| format!("write: {}", e))?;
    tracing::info!("Face model downloaded ({} bytes)", bytes.len());
    Ok(())
}

fn load_onnx_model(path: &Path) -> anyhow::Result<FaceModel> {
    use tract_onnx::prelude::*;

    let model = tract_onnx::onnx()
        .model_for_path(path)?
        .with_input_fact(0, f32::fact([1, 3, MODEL_HEIGHT, MODEL_WIDTH]).into())?
        .into_optimized()?
        .into_runnable()?;

    Ok(FaceModel { model })
}

// ── Detection entry point ───────────────────────────────────────────

/// Detect faces from an already-decoded image.
/// Uses the ONNX model if loaded, otherwise falls back to a conservative heuristic.
pub fn detect_faces_from_image(
    img: &DynamicImage,
    min_confidence: f32,
) -> anyhow::Result<Vec<FaceDetection>> {
    let model = FACE_MODEL.get().and_then(|m| m.as_ref());

    match model {
        Some(m) => detect_faces_onnx(img, min_confidence, m),
        None => detect_faces_heuristic(img, min_confidence),
    }
}

// ── ONNX model-based detection ──────────────────────────────────────

fn detect_faces_onnx(
    img: &DynamicImage,
    min_confidence: f32,
    model: &FaceModel,
) -> anyhow::Result<Vec<FaceDetection>> {
    use tract_onnx::prelude::*;

    let (w, h) = img.dimensions();
    if w < 20 || h < 20 {
        return Ok(vec![]);
    }

    // Resize to model input size (320x240)
    let resized = img.resize_exact(MODEL_WIDTH as u32, MODEL_HEIGHT as u32, FilterType::Triangle);
    let rgb = resized.to_rgb8();

    // Build input tensor: [1, 3, 240, 320], normalised to (pixel - 127) / 128
    let mut input = tract_ndarray::Array4::<f32>::zeros([1, 3, MODEL_HEIGHT, MODEL_WIDTH]);
    for y in 0..MODEL_HEIGHT {
        for x in 0..MODEL_WIDTH {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            input[[0, 0, y, x]] = (pixel[0] as f32 - 127.0) / 128.0;
            input[[0, 1, y, x]] = (pixel[1] as f32 - 127.0) / 128.0;
            input[[0, 2, y, x]] = (pixel[2] as f32 - 127.0) / 128.0;
        }
    }

    let input_tv: TValue = input.into_tvalue();
    let outputs = model.model.run(tvec![input_tv])?;

    // UltraFace outputs:
    //   [0] scores: [1, N, 2] — [background_conf, face_conf]
    //   [1] boxes:  [1, N, 4] — [x_min, y_min, x_max, y_max] normalised 0..1
    let scores = outputs[0].to_array_view::<f32>()?;
    let boxes = outputs[1].to_array_view::<f32>()?;

    let num_candidates = scores.shape()[1];
    let mut detections = Vec::new();

    for i in 0..num_candidates {
        let face_conf = scores[[0, i, 1]];
        if face_conf < min_confidence {
            continue;
        }

        let x_min = boxes[[0, i, 0]].clamp(0.0, 1.0);
        let y_min = boxes[[0, i, 1]].clamp(0.0, 1.0);
        let x_max = boxes[[0, i, 2]].clamp(0.0, 1.0);
        let y_max = boxes[[0, i, 3]].clamp(0.0, 1.0);

        let bw = x_max - x_min;
        let bh = y_max - y_min;

        if bw < 0.01 || bh < 0.01 {
            continue;
        }

        detections.push(FaceDetection {
            bbox: BoundingBox { x: x_min, y: y_min, w: bw, h: bh },
            confidence: face_conf,
            embedding: Vec::new(),
        });
    }

    // NMS
    nms(&mut detections, 0.3);

    tracing::debug!(
        detections = detections.len(),
        "Face detection (ONNX model): complete"
    );

    Ok(detections)
}

// ── Heuristic fallback ──────────────────────────────────────────────

/// Conservative heuristic face detection. Only detects very clear faces
/// by requiring both skin colour AND face-like structure (eye features).
fn detect_faces_heuristic(
    img: &DynamicImage,
    min_confidence: f32,
) -> anyhow::Result<Vec<FaceDetection>> {
    let (w, h) = img.dimensions();
    if w < 48 || h < 48 {
        return Ok(vec![]);
    }

    // Resize to a workable analysis size
    let analysis_size = 320u32;
    let scale = analysis_size as f32 / w.max(h) as f32;
    let (aw, ah) = (
        (w as f32 * scale).max(1.0) as u32,
        (h as f32 * scale).max(1.0) as u32,
    );
    let small = img.resize_exact(aw.max(1), ah.max(1), FilterType::Triangle);
    let rgb = small.to_rgb8();

    let mut detections = Vec::new();

    // More conservative sliding window: fewer scales, larger minimum
    let min_face = 40u32;
    let mut win_size = min_face;

    // Faces should be between 5% and 50% of image dimension
    let max_face = (aw.min(ah) as f32 * 0.5) as u32;

    while win_size <= max_face {
        let step = (win_size / 3).max(6);
        let mut y = 0u32;
        while y + win_size <= ah {
            let mut x = 0u32;
            while x + win_size <= aw {
                let skin_score = skin_density_score_ycbcr(&rgb, x, y, win_size, win_size);
                if skin_score >= min_confidence.max(0.6) {
                    // Verify face-like structure: check for eye features
                    let struct_score = face_structure_score(&rgb, x, y, win_size, win_size);
                    let combined = skin_score * 0.4 + struct_score * 0.6;

                    if combined >= min_confidence.max(0.55) && struct_score >= 0.3 {
                        let bbox = BoundingBox {
                            x: x as f32 / aw as f32,
                            y: y as f32 / ah as f32,
                            w: win_size as f32 / aw as f32,
                            h: win_size as f32 / ah as f32,
                        };
                        detections.push(FaceDetection {
                            bbox,
                            confidence: combined,
                            embedding: Vec::new(),
                        });
                    }
                }
                x += step;
            }
            y += step;
        }
        win_size = (win_size as f32 * 1.5) as u32;
    }

    // Aggressive NMS
    nms(&mut detections, 0.3);

    // Cap at 5 faces per image for heuristic fallback
    detections.truncate(5);

    tracing::debug!(
        detections = detections.len(),
        "Face detection (heuristic fallback): complete"
    );

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

/// Skin-colour density using YCbCr colour space (much more robust than RGB).
fn skin_density_score_ycbcr(
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

            // Convert to YCbCr
            let cb = 128.0 + (-0.169 * r - 0.331 * g + 0.500 * b);
            let cr = 128.0 + (0.500 * r - 0.419 * g - 0.081 * b);

            // Strict skin range in YCbCr (tighter than standard)
            if cb >= 77.0 && cb <= 127.0 && cr >= 133.0 && cr <= 173.0
                && r > 80.0 && g > 30.0 && b > 15.0
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

    // Require high skin coverage (faces are 35-70% skin)
    if density < 0.30 {
        return 0.0;
    }
    if density > 0.80 {
        return 0.0; // Too uniform, likely not a face
    }

    ((density - 0.25) / 0.45).clamp(0.0, 0.95)
}

/// Check for face-like structure within a candidate region.
/// Looks for dark features (eyes) in the upper half and moderate edge density.
fn face_structure_score(
    rgb: &image::RgbImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> f32 {
    if w < 16 || h < 16 {
        return 0.0;
    }

    let gray: Vec<f32> = {
        let mut g = Vec::with_capacity((w * h) as usize);
        for py in y..(y + h).min(rgb.height()) {
            for px in x..(x + w).min(rgb.width()) {
                let p = rgb.get_pixel(px, py);
                g.push(0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32);
            }
        }
        g
    };

    let actual_w = w.min(rgb.width() - x) as usize;
    let actual_h = h.min(rgb.height() - y) as usize;
    if actual_w < 8 || actual_h < 8 || gray.len() < actual_w * actual_h {
        return 0.0;
    }

    let mut score = 0.0f32;

    // 1. Check for dark spots in eye region (20-45% from top, 20-80% from sides)
    let eye_y_start = actual_h * 20 / 100;
    let eye_y_end = actual_h * 45 / 100;
    let eye_x_left_start = actual_w * 15 / 100;
    let eye_x_left_end = actual_w * 40 / 100;
    let eye_x_right_start = actual_w * 60 / 100;
    let eye_x_right_end = actual_w * 85 / 100;

    // Mean luminance of full region
    let mean_lum: f32 = gray.iter().sum::<f32>() / gray.len() as f32;

    // Mean luminance of left and right eye regions
    let mut left_eye_sum = 0.0f32;
    let mut left_eye_count = 0u32;
    let mut right_eye_sum = 0.0f32;
    let mut right_eye_count = 0u32;

    for py in eye_y_start..eye_y_end {
        for px in eye_x_left_start..eye_x_left_end {
            if py * actual_w + px < gray.len() {
                left_eye_sum += gray[py * actual_w + px];
                left_eye_count += 1;
            }
        }
        for px in eye_x_right_start..eye_x_right_end {
            if py * actual_w + px < gray.len() {
                right_eye_sum += gray[py * actual_w + px];
                right_eye_count += 1;
            }
        }
    }

    if left_eye_count > 0 && right_eye_count > 0 {
        let left_mean = left_eye_sum / left_eye_count as f32;
        let right_mean = right_eye_sum / right_eye_count as f32;

        // Eyes should be darker than face mean
        if left_mean < mean_lum * 0.90 && right_mean < mean_lum * 0.90 {
            score += 0.4;
        }
        // Eyes should have similar luminance (symmetry)
        let eye_diff = (left_mean - right_mean).abs() / mean_lum.max(1.0);
        if eye_diff < 0.2 {
            score += 0.2;
        }
    }

    // 2. Check bilateral symmetry of the full region
    let mut sym_diff = 0.0f32;
    let mut sym_count = 0u32;
    for py in 0..actual_h {
        for px in 0..(actual_w / 2) {
            let mirror_px = actual_w - 1 - px;
            let idx1 = py * actual_w + px;
            let idx2 = py * actual_w + mirror_px;
            if idx1 < gray.len() && idx2 < gray.len() {
                sym_diff += (gray[idx1] - gray[idx2]).abs();
                sym_count += 1;
            }
        }
    }
    if sym_count > 0 {
        let avg_sym = sym_diff / sym_count as f32;
        if avg_sym < 25.0 {
            score += 0.2;
        } else if avg_sym < 40.0 {
            score += 0.1;
        }
    }

    // 3. Edge density check: faces have moderate edges
    let mut edge_count = 0u32;
    let mut edge_total = 0u32;
    for py in 1..(actual_h - 1) {
        for px in 1..(actual_w - 1) {
            let idx = py * actual_w + px;
            if idx + actual_w < gray.len() && idx >= actual_w {
                let gx = gray[idx + 1] - gray[idx - 1];
                let gy = gray[idx + actual_w] - gray[idx - actual_w];
                if (gx * gx + gy * gy).sqrt() > 20.0 {
                    edge_count += 1;
                }
                edge_total += 1;
            }
        }
    }
    if edge_total > 0 {
        let edge_ratio = edge_count as f32 / edge_total as f32;
        // Faces have moderate edge density (0.05-0.35)
        if edge_ratio >= 0.05 && edge_ratio <= 0.35 {
            score += 0.2;
        }
    }

    score
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
