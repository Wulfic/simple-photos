//! Legacy face-detection paths: UltraFace ONNX model and the
//! pure-pixel heuristic fallback.
//!
//! These exist as graceful degradation when SCRFD/ArcFace cannot be
//! loaded.  They are intentionally low-quality compared to the primary
//! pipeline and gated by the `allow_heuristic_fallback` flag at the
//! call site.  The histogram embedding helper is also re-used by the
//! main SCRFD/ArcFace path when the recognition model is unavailable.

use super::ort_err;
use crate::ai::models::{BoundingBox, FaceDetection};
use image::{imageops::FilterType, DynamicImage, GenericImageView, RgbImage};
use ort::session::Session;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

// ── Model constants ─────────────────────────────────────────────────

const LEGACY_WIDTH: usize = 320;
const LEGACY_HEIGHT: usize = 240;
const LEGACY_MODEL_URL: &str = "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx";
const LEGACY_MODEL_FILENAME: &str = "ultraface-RFB-320.onnx";

static LEGACY_MODEL: OnceLock<Option<Arc<Mutex<Session>>>> = OnceLock::new();

// ── Initialisation ──────────────────────────────────────────────────

/// Initialise the legacy UltraFace model, but only if the primary
/// SCRFD detector failed to load.  `dir_exists` mirrors the same flag
/// in [`super::init_face_model`] so we don't attempt downloads when
/// the model directory is missing.
pub(super) fn init_legacy_model(model_dir: &Path, dir_exists: bool, scrfd_loaded: bool) {
    LEGACY_MODEL.get_or_init(|| {
        if scrfd_loaded {
            return None; // SCRFD loaded, no need for legacy
        }
        let p = model_dir.join(LEGACY_MODEL_FILENAME);
        if !p.exists() {
            if !dir_exists {
                return None;
            }
            tracing::info!("Legacy UltraFace not found at {:?}, downloading…", p);
            if let Err(e) = super::download_model(LEGACY_MODEL_URL, &p, 100_000) {
                tracing::warn!("Legacy download failed: {}. Heuristic-only mode.", e);
                return None;
            }
        }
        match load_onnx_legacy(&p) {
            Ok(session) => {
                tracing::info!("Legacy UltraFace model loaded from {:?}", p);
                Some(Arc::new(Mutex::new(session)))
            }
            Err(e) => {
                tracing::warn!("Failed to load legacy model: {}. Heuristic-only mode.", e);
                None
            }
        }
    });
}

fn load_onnx_legacy(path: &Path) -> anyhow::Result<Session> {
    crate::ai::session::build_session(path)
}

/// If a legacy UltraFace model is loaded, run detection and return its
/// result.  Returns `None` when no legacy model is available, leaving
/// the caller free to fall through to the heuristic path.
pub(super) fn detect_faces_legacy_if_loaded(
    img: &DynamicImage,
    min_confidence: f32,
) -> Option<anyhow::Result<Vec<FaceDetection>>> {
    let legacy = LEGACY_MODEL.get().and_then(|m| m.as_ref())?;
    let mut session = legacy.lock().unwrap_or_else(|p| p.into_inner());
    Some(detect_faces_legacy(img, min_confidence, &mut session))
}

// ── UltraFace detection ─────────────────────────────────────────────

fn detect_faces_legacy(
    img: &DynamicImage,
    min_confidence: f32,
    model: &mut Session,
) -> anyhow::Result<Vec<FaceDetection>> {
    let (w, h) = img.dimensions();
    if w < 20 || h < 20 {
        return Ok(vec![]);
    }

    let resized = img.resize_exact(
        LEGACY_WIDTH as u32,
        LEGACY_HEIGHT as u32,
        FilterType::Triangle,
    );
    let rgb = resized.to_rgb8();

    let mut input = ndarray::Array4::<f32>::zeros([1, 3, LEGACY_HEIGHT, LEGACY_WIDTH]);
    for y in 0..LEGACY_HEIGHT {
        for x in 0..LEGACY_WIDTH {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            input[[0, 0, y, x]] = (pixel[0] as f32 - 127.0) / 128.0;
            input[[0, 1, y, x]] = (pixel[1] as f32 - 127.0) / 128.0;
            input[[0, 2, y, x]] = (pixel[2] as f32 - 127.0) / 128.0;
        }
    }

    let input_tensor = ort_err(ort::value::Tensor::from_array(input))?;
    let outputs = ort_err(model.run(ort::inputs![input_tensor]))?;

    let (score_shape, scores_data) = ort_err(outputs[0].try_extract_tensor::<f32>())?;
    let (_box_shape, boxes_data) = ort_err(outputs[1].try_extract_tensor::<f32>())?;

    // scores shape is [1, N, 2], boxes shape is [1, N, 4]
    let num_candidates = score_shape[1] as usize;
    let mut detections = Vec::new();

    for i in 0..num_candidates {
        let face_conf = scores_data[i * 2 + 1]; // [0, i, 1] in flat layout, skip batch dim
        if face_conf < min_confidence {
            continue;
        }

        let x_min = boxes_data[i * 4].clamp(0.0, 1.0);
        let y_min = boxes_data[i * 4 + 1].clamp(0.0, 1.0);
        let x_max = boxes_data[i * 4 + 2].clamp(0.0, 1.0);
        let y_max = boxes_data[i * 4 + 3].clamp(0.0, 1.0);

        let bw = x_max - x_min;
        let bh = y_max - y_min;

        if bw < 0.01 || bh < 0.01 {
            continue;
        }

        detections.push(FaceDetection {
            bbox: BoundingBox {
                x: x_min,
                y: y_min,
                w: bw,
                h: bh,
            },
            confidence: face_conf,
            embedding: Vec::new(),
        });
    }

    nms(&mut detections, 0.3);

    // Extract embeddings using histogram (legacy has no landmarks)
    for det in &mut detections {
        det.embedding = extract_histogram_embedding(img, &det.bbox);
    }

    tracing::debug!(
        detections = detections.len(),
        "Face detection (UltraFace legacy): complete"
    );

    Ok(detections)
}

// ── Heuristic fallback ──────────────────────────────────────────────

pub(super) fn detect_faces_heuristic(
    img: &DynamicImage,
    min_confidence: f32,
) -> anyhow::Result<Vec<FaceDetection>> {
    let (w, h) = img.dimensions();
    if w < 48 || h < 48 {
        return Ok(vec![]);
    }

    let analysis_size = 320u32;
    let scale = analysis_size as f32 / w.max(h) as f32;
    let (aw, ah) = (
        (w as f32 * scale).max(1.0) as u32,
        (h as f32 * scale).max(1.0) as u32,
    );
    let small = img.resize_exact(aw.max(1), ah.max(1), FilterType::Triangle);
    let rgb = small.to_rgb8();

    let mut detections = Vec::new();
    let min_face = 40u32;
    let mut win_size = min_face;
    let max_face = (aw.min(ah) as f32 * 0.5) as u32;

    while win_size <= max_face {
        let step = (win_size / 3).max(6);
        let mut y = 0u32;
        while y + win_size <= ah {
            let mut x = 0u32;
            while x + win_size <= aw {
                let skin_score = skin_density_score_ycbcr(&rgb, x, y, win_size, win_size);
                if skin_score >= min_confidence.max(0.6) {
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

    nms(&mut detections, 0.3);
    detections.truncate(5);

    tracing::debug!(
        detections = detections.len(),
        "Face detection (heuristic fallback): complete"
    );

    Ok(detections)
}

// ── Histogram embedding (shared fallback) ───────────────────────────

/// Histogram-based 128-dim embedding (fallback when no ArcFace model).
pub(super) fn extract_histogram_embedding(img: &DynamicImage, bbox: &BoundingBox) -> Vec<f32> {
    let (iw, ih) = img.dimensions();

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

    let mut embedding = Vec::with_capacity(128);

    // Colour histogram: 16 bins × 3 channels = 48
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

    // Gradient orientation histogram (32 bins)
    let gray = face_resized.to_luma8();
    let mut grad_hist = [0u32; 32];
    for y in 1..63u32 {
        for x in 1..63u32 {
            let gx = gray.get_pixel(x + 1, y)[0] as f32 - gray.get_pixel(x - 1, y)[0] as f32;
            let gy = gray.get_pixel(x, y + 1)[0] as f32 - gray.get_pixel(x, y - 1)[0] as f32;
            let mag = (gx * gx + gy * gy).sqrt();
            if mag > 10.0 {
                let angle = gy.atan2(gx) + std::f32::consts::PI;
                let bin = ((angle / (2.0 * std::f32::consts::PI)) * 32.0) as usize;
                grad_hist[bin.min(31)] += 1;
            }
        }
    }
    let grad_total: u32 = grad_hist.iter().sum();
    let grad_norm = if grad_total > 0 {
        grad_total as f32
    } else {
        1.0
    };
    for i in 0..32 {
        embedding.push(grad_hist[i] as f32 / grad_norm);
    }

    // Spatial means (4×4 grid × 3 channels = 48)
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

// ── Internal helpers ────────────────────────────────────────────────

/// NMS on FaceDetection vec (used by legacy and heuristic paths).
fn nms(detections: &mut Vec<FaceDetection>, iou_threshold: f32) {
    detections.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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

/// Skin-colour density using YCbCr colour space (heuristic fallback).
fn skin_density_score_ycbcr(rgb: &RgbImage, x: u32, y: u32, w: u32, h: u32) -> f32 {
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
            let cb = 128.0 + (-0.169 * r - 0.331 * g + 0.500 * b);
            let cr = 128.0 + (0.500 * r - 0.419 * g - 0.081 * b);
            if (77.0..=127.0).contains(&cb)
                && (133.0..=173.0).contains(&cr)
                && r > 80.0
                && g > 30.0
                && b > 15.0
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
    if !(0.30..=0.80).contains(&density) {
        return 0.0;
    }
    ((density - 0.25) / 0.45).clamp(0.0, 0.95)
}

/// Face-like structure score (heuristic fallback).
fn face_structure_score(rgb: &RgbImage, x: u32, y: u32, w: u32, h: u32) -> f32 {
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

    let eye_y_start = actual_h * 20 / 100;
    let eye_y_end = actual_h * 45 / 100;
    let eye_x_left_start = actual_w * 15 / 100;
    let eye_x_left_end = actual_w * 40 / 100;
    let eye_x_right_start = actual_w * 60 / 100;
    let eye_x_right_end = actual_w * 85 / 100;

    let mean_lum: f32 = gray.iter().sum::<f32>() / gray.len() as f32;

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
        if left_mean < mean_lum * 0.90 && right_mean < mean_lum * 0.90 {
            score += 0.4;
        }
        let eye_diff = (left_mean - right_mean).abs() / mean_lum.max(1.0);
        if eye_diff < 0.2 {
            score += 0.2;
        }
    }

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
        if (0.05..=0.35).contains(&edge_ratio) {
            score += 0.2;
        }
    }

    score
}
