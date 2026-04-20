//! Object detection and scene classification pipeline.
//!
//! Primary: MobileNetV2 ONNX model for 1000-class ImageNet classification.
//! Fallback: Multi-pass colour histogram, texture, and edge-density analysis
//! to classify content into ~25 categories.
//!
//! The MobileNetV2 model is automatically downloaded on first use (~14 MB).
//! Results from both the model and heuristics are combined, with the model
//! classes taking priority.

use crate::ai::imagenet_labels::{self, IMAGENET_LABELS};
use crate::ai::models::{BoundingBox, ObjectDetection};
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use tracing;
use ort::session::Session;

/// Convert ort errors (which aren't Send+Sync) to anyhow errors.
fn ort_err<R>(r: Result<R, impl std::fmt::Display>) -> anyhow::Result<R> {
    r.map_err(|e| anyhow::anyhow!("{e}"))
}

// ── MobileNetV2 model constants ─────────────────────────────────────

/// MobileNetV2-12 for 1000-class ImageNet classification.
/// Input: [1, 3, 224, 224] RGB normalised with ImageNet mean/std.
/// Output: [1, 1000] logits (pre-softmax).
const CLS_WIDTH: usize = 224;
const CLS_HEIGHT: usize = 224;
const CLS_MODEL_URL: &str =
    "https://github.com/onnx/models/raw/refs/heads/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx";
const CLS_MODEL_FILENAME: &str = "mobilenetv2-12.onnx";

/// ImageNet normalisation constants.
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Number of top predictions to return from the model.
const TOP_K: usize = 5;

// ── Model singleton ─────────────────────────────────────────────────

static CLS_MODEL: OnceLock<Option<Arc<Mutex<Session>>>> = OnceLock::new();

// ── Initialisation ──────────────────────────────────────────────────

/// Download and load the MobileNetV2 classification model.
pub fn init_classification_model(model_dir: &str) {
    let dir = Path::new(model_dir);

    CLS_MODEL.get_or_init(|| {
        let p = dir.join(CLS_MODEL_FILENAME);
        if !p.exists() {
            if !dir.is_dir() {
                tracing::info!(
                    "Model directory {:?} does not exist — heuristic-only object detection",
                    dir
                );
                return None;
            }
            tracing::info!("MobileNetV2 model not found at {:?}, downloading…", p);
            if let Err(e) = download_model(CLS_MODEL_URL, &p, 5_000_000) {
                tracing::warn!("MobileNetV2 download failed: {}. Using heuristic fallback.", e);
                return None;
            }
        }
        match load_onnx_cls(&p) {
            Ok(session) => {
                tracing::info!("MobileNetV2 classification model loaded from {:?}", p);
                Some(Arc::new(Mutex::new(session)))
            }
            Err(e) => {
                tracing::warn!("Failed to load MobileNetV2: {}. Using heuristic fallback.", e);
                None
            }
        }
    });
}

fn download_model(url: &str, dest: &Path, min_size: usize) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    tracing::info!("Downloading model from {url}…");
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("client: {e}"))?
        .get(url)
        .send()
        .map_err(|e| format!("download: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().map_err(|e| format!("read: {e}"))?;
    if bytes.len() < min_size {
        return Err(format!(
            "File too small ({} bytes), expected ≥{min_size}",
            bytes.len()
        ));
    }
    std::fs::write(dest, &bytes).map_err(|e| format!("write: {e}"))?;
    tracing::info!("Model downloaded ({} bytes) → {:?}", bytes.len(), dest);
    Ok(())
}

fn load_onnx_cls(path: &Path) -> anyhow::Result<Session> {
    let session = ort_err(ort_err(Session::builder())?
        .with_intra_threads(1))?
        .commit_from_file(path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(session)
}

// ── MobileNetV2 classification ──────────────────────────────────────

/// Run MobileNetV2 classification on an image.
/// Returns top-K predictions above `min_confidence`, mapped to broad categories.
fn classify_mobilenet(
    img: &DynamicImage,
    min_confidence: f32,
    model: &mut Session,
) -> anyhow::Result<Vec<ObjectDetection>> {
    // Resize to 224×224 using bilinear interpolation
    let resized = img.resize_exact(CLS_WIDTH as u32, CLS_HEIGHT as u32, FilterType::Triangle);
    let rgb = resized.to_rgb8();

    // Build [1, 3, 224, 224] tensor: RGB channels, normalised with ImageNet stats
    let mut input = ndarray::Array4::<f32>::zeros((1, 3, CLS_HEIGHT, CLS_WIDTH));
    for y in 0..CLS_HEIGHT {
        for x in 0..CLS_WIDTH {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            for c in 0..3 {
                input[[0, c, y, x]] =
                    (pixel[c] as f32 / 255.0 - IMAGENET_MEAN[c]) / IMAGENET_STD[c];
            }
        }
    }

    let input_tensor = ort_err(ort::value::Tensor::from_array(input))?;
    let outputs = ort_err(model.run(ort::inputs![input_tensor]))?;

    // Output shape: [1, 1000] logits → apply softmax
    let (_logit_shape, logits_data) = ort_err(outputs[0].try_extract_tensor::<f32>())?;
    let logits_slice: Vec<f32> = logits_data.to_vec();

    // Softmax
    let max_logit = logits_slice.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum: f32 = logits_slice.iter().map(|&x| (x - max_logit).exp()).sum();
    let probs: Vec<f32> = logits_slice
        .iter()
        .map(|&x| (x - max_logit).exp() / exp_sum)
        .collect();

    // Get top-K
    let mut indexed: Vec<(usize, f32)> = probs.into_iter().enumerate().collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    indexed.truncate(TOP_K);

    let full_bbox = BoundingBox {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };

    let mut detections = Vec::new();
    let mut seen_categories = std::collections::HashSet::new();

    for (idx, prob) in &indexed {
        if *prob < min_confidence {
            continue;
        }

        let raw_label = if *idx < IMAGENET_LABELS.len() {
            IMAGENET_LABELS[*idx]
        } else {
            continue;
        };

        tracing::debug!(
            index = idx,
            label = raw_label,
            probability = format!("{:.4}", prob),
            "MobileNetV2 prediction"
        );

        // Map to a broad category for useful photo tags
        if let Some(category) = imagenet_labels::label_category(*idx) {
            if seen_categories.insert(category) {
                detections.push(ObjectDetection {
                    class_name: category.to_string(),
                    confidence: *prob,
                    bbox: full_bbox.clone(),
                });
            }
        }
    }

    Ok(detections)
}

/// Quality preset for detection — higher quality = slower but more accurate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionQuality {
    Fast,     // Quick colour histogram only (~2 classes)
    Balanced, // Multi-pass + spatial analysis (~15 classes)
    High,     // Full analysis at multiple scales (~25+ classes)
}

/// Detect objects/scenes from an already-decoded image.
///
/// Tries MobileNetV2 classification first for real object recognition.
/// Always runs heuristic analysis for scene attributes (night, sunset, panoramic, etc.)
/// that the ImageNet classifier doesn't cover well.
pub fn detect_objects_with_quality(
    img: &DynamicImage,
    min_confidence: f32,
    quality: DetectionQuality,
) -> anyhow::Result<Vec<ObjectDetection>> {
    let (w, h) = img.dimensions();
    if w < 10 || h < 10 {
        return Ok(vec![]);
    }

    let mut detections = Vec::new();

    // ── Phase 1: MobileNetV2 classification (if model available) ─────
    let model_used = if let Some(Some(model)) = CLS_MODEL.get() {
        let mut session = model.lock().unwrap();
        match classify_mobilenet(img, min_confidence, &mut session) {
            Ok(model_dets) => {
                tracing::debug!(
                    count = model_dets.len(),
                    classes = ?model_dets.iter().map(|d| &d.class_name).collect::<Vec<_>>(),
                    "MobileNetV2 classification complete"
                );
                detections.extend(model_dets);
                true
            }
            Err(e) => {
                tracing::warn!("MobileNetV2 inference failed: {e}");
                false
            }
        }
    } else {
        false
    };

    // ── Phase 2: Heuristic scene classification ──────────────────────
    // These scene tags complement model detections (night, sunset, snow, etc.)
    let heuristic_dets = detect_scenes_heuristic(img, min_confidence, quality)?;

    // Merge: model tags take priority; only add heuristic tags not already covered
    let model_classes: std::collections::HashSet<String> =
        detections.iter().map(|d| d.class_name.clone()).collect();
    for det in heuristic_dets {
        if !model_classes.contains(&det.class_name) {
            detections.push(det);
        }
    }

    if !model_used && detections.is_empty() {
        tracing::debug!("No model and no heuristic detections for {}×{} image", w, h);
    }

    tracing::debug!(
        total = detections.len(),
        classes = ?detections.iter().map(|d| &d.class_name).collect::<Vec<_>>(),
        model_used = model_used,
        "Object detection: final results"
    );

    Ok(detections)
}

/// Heuristic scene classification using colour, texture, and spatial analysis.
fn detect_scenes_heuristic(
    img: &DynamicImage,
    min_confidence: f32,
    quality: DetectionQuality,
) -> anyhow::Result<Vec<ObjectDetection>> {
    let (w, h) = img.dimensions();
    if w < 10 || h < 10 {
        return Ok(vec![]);
    }

    let full_bbox = BoundingBox { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let mut detections = Vec::new();

    // ── Pass 1: Global colour & luminance statistics ─────────────────
    let rgb = img.to_rgb8();
    let total_pixels = (w * h) as f32;

    let mut green_count = 0u32;
    let mut blue_count = 0u32;
    let mut brown_count = 0u32;
    let mut bright_count = 0u32;
    let mut dark_count = 0u32;
    let mut red_count = 0u32;
    let mut orange_count = 0u32;
    let mut warm_count = 0u32;
    let mut white_count = 0u32;
    let mut yellow_count = 0u32;
    let mut grey_count = 0u32;
    let mut saturated_count = 0u32;
    let mut desaturated_count = 0u32;

    let mut lum_sum = 0.0f64;
    let mut lum_sq_sum = 0.0f64;
    let mut sat_sum = 0.0f64;

    for pixel in rgb.pixels() {
        let r = pixel[0] as f32;
        let g = pixel[1] as f32;
        let b = pixel[2] as f32;
        let lum = 0.299 * r + 0.587 * g + 0.114 * b;
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let chroma = max_c - min_c;
        let sat = if max_c > 0.0 { chroma / max_c } else { 0.0 };

        lum_sum += lum as f64;
        lum_sq_sum += (lum as f64) * (lum as f64);
        sat_sum += sat as f64;

        if g > r * 1.2 && g > b * 1.2 && g > 60.0 { green_count += 1; }
        if b > r * 1.3 && b > g * 1.1 && b > 80.0 { blue_count += 1; }
        if r > 80.0 && g > 40.0 && g < r && b < g && r - b > 30.0 { brown_count += 1; }
        if lum > 200.0 { bright_count += 1; }
        if lum < 40.0 { dark_count += 1; }
        if r > 180.0 && g < 80.0 && b < 80.0 { red_count += 1; }
        if r > 180.0 && g > 100.0 && g < 180.0 && b < 80.0 { orange_count += 1; }
        if r > 150.0 && g > 100.0 && b < 100.0 { warm_count += 1; }
        if lum > 230.0 && chroma < 20.0 { white_count += 1; }
        if r > 180.0 && g > 180.0 && b < 80.0 { yellow_count += 1; }
        if chroma < 20.0 && lum > 50.0 && lum < 200.0 { grey_count += 1; }
        if sat > 0.6 { saturated_count += 1; }
        if sat < 0.15 { desaturated_count += 1; }
    }

    let green_ratio = green_count as f32 / total_pixels;
    let blue_ratio = blue_count as f32 / total_pixels;
    let brown_ratio = brown_count as f32 / total_pixels;
    let bright_ratio = bright_count as f32 / total_pixels;
    let dark_ratio = dark_count as f32 / total_pixels;
    let red_ratio = red_count as f32 / total_pixels;
    let orange_ratio = orange_count as f32 / total_pixels;
    let warm_ratio = warm_count as f32 / total_pixels;
    let white_ratio = white_count as f32 / total_pixels;
    let yellow_ratio = yellow_count as f32 / total_pixels;
    let grey_ratio = grey_count as f32 / total_pixels;
    let saturated_ratio = saturated_count as f32 / total_pixels;
    let desaturated_ratio = desaturated_count as f32 / total_pixels;

    let mean_lum = (lum_sum / total_pixels as f64) as f32;
    let lum_variance = ((lum_sq_sum / total_pixels as f64) - (mean_lum as f64).powi(2)) as f32;
    let lum_std = lum_variance.max(0.0).sqrt();
    let mean_sat = (sat_sum / total_pixels as f64) as f32;

    tracing::debug!(
        img_w = w, img_h = h,
        green = format!("{:.3}", green_ratio),
        blue = format!("{:.3}", blue_ratio),
        brown = format!("{:.3}", brown_ratio),
        bright = format!("{:.3}", bright_ratio),
        dark = format!("{:.3}", dark_ratio),
        red = format!("{:.3}", red_ratio),
        warm = format!("{:.3}", warm_ratio),
        mean_lum = format!("{:.1}", mean_lum),
        lum_std = format!("{:.1}", lum_std),
        mean_sat = format!("{:.3}", mean_sat),
        "Object detection: colour analysis complete"
    );

    // ── Pass 2: Spatial analysis (top/bottom/center distribution) ────
    let top_third_h = h / 3;
    let bottom_third_start = h * 2 / 3;

    let mut top_blue = 0u32;
    let mut top_bright = 0u32;
    let mut top_total = 0u32;
    let mut bottom_green = 0u32;
    let mut bottom_brown = 0u32;
    let mut bottom_blue = 0u32;
    let mut bottom_total = 0u32;

    // Also compute centre region stats for subject detection
    let cx_start = w / 4;
    let cx_end = w * 3 / 4;
    let cy_start = h / 4;
    let cy_end = h * 3 / 4;
    let mut centre_warm = 0u32;
    let mut centre_sat = 0u32;
    let mut centre_total = 0u32;

    for (x, y, pixel) in rgb.enumerate_pixels() {
        let r = pixel[0] as f32;
        let g = pixel[1] as f32;
        let b = pixel[2] as f32;
        let lum = 0.299 * r + 0.587 * g + 0.114 * b;
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let chroma = max_c - min_c;
        let sat = if max_c > 0.0 { chroma / max_c } else { 0.0 };

        if y < top_third_h {
            top_total += 1;
            if b > r * 1.3 && b > g * 1.1 && b > 80.0 { top_blue += 1; }
            if lum > 200.0 { top_bright += 1; }
        }
        if y >= bottom_third_start {
            bottom_total += 1;
            if g > r * 1.2 && g > b * 1.2 && g > 60.0 { bottom_green += 1; }
            if r > 80.0 && g > 40.0 && g < r && b < g { bottom_brown += 1; }
            if b > r * 1.3 && b > g * 1.1 && b > 80.0 { bottom_blue += 1; }
        }
        if x >= cx_start && x < cx_end && y >= cy_start && y < cy_end {
            centre_total += 1;
            if r > 150.0 && g > 100.0 && b < 100.0 { centre_warm += 1; }
            if sat > 0.5 { centre_sat += 1; }
        }
    }

    let top_blue_ratio = if top_total > 0 { top_blue as f32 / top_total as f32 } else { 0.0 };
    let top_bright_ratio = if top_total > 0 { top_bright as f32 / top_total as f32 } else { 0.0 };
    let bottom_green_ratio = if bottom_total > 0 { bottom_green as f32 / bottom_total as f32 } else { 0.0 };
    let bottom_brown_ratio = if bottom_total > 0 { bottom_brown as f32 / bottom_total as f32 } else { 0.0 };
    let bottom_blue_ratio = if bottom_total > 0 { bottom_blue as f32 / bottom_total as f32 } else { 0.0 };
    let _centre_warm_ratio = if centre_total > 0 { centre_warm as f32 / centre_total as f32 } else { 0.0 };
    let _centre_sat_ratio = if centre_total > 0 { centre_sat as f32 / centre_total as f32 } else { 0.0 };

    // ── Pass 3: Edge density (gradient magnitude) ────────────────────
    let edge_density = if quality != DetectionQuality::Fast {
        compute_edge_density(img, w, h)
    } else {
        0.0
    };

    // ── Pass 4: Aspect ratio analysis ────────────────────────────────
    let aspect_ratio = w as f32 / h as f32;
    let is_panoramic = aspect_ratio > 2.5;
    let is_portrait_aspect = aspect_ratio < 0.7;

    // ─────────────────────────────────────────────────────────────────
    // Scene / content classification
    // ─────────────────────────────────────────────────────────────────

    // Night / low-light scene
    if dark_ratio > 0.65 && mean_lum < 50.0 {
        let conf = ((dark_ratio - 0.5) * 2.0).clamp(0.4, 0.88);
        push_if(&mut detections, "night", conf, min_confidence, &full_bbox);
    }

    // Sunset / sunrise — warm tones in upper portion, gradient
    if warm_ratio > 0.15 && (orange_ratio > 0.05 || red_ratio > 0.03)
        && top_bright_ratio > 0.1 && mean_sat > 0.3
    {
        let conf = ((warm_ratio + orange_ratio) * 1.5).clamp(0.4, 0.85);
        push_if(&mut detections, "sunset", conf, min_confidence, &full_bbox);
    }

    // Sky (blue top third, not uniformly blue overall)
    if top_blue_ratio > 0.25 && blue_ratio < 0.7 {
        let conf = (top_blue_ratio * 1.2).clamp(0.4, 0.85);
        push_if(&mut detections, "sky", conf, min_confidence, &full_bbox);
    }

    // Landscape — blue sky top + green/brown bottom
    if top_blue_ratio > 0.2 && (bottom_green_ratio > 0.15 || bottom_brown_ratio > 0.15) {
        let conf = ((top_blue_ratio + bottom_green_ratio.max(bottom_brown_ratio)) * 0.9).clamp(0.4, 0.85);
        push_if(&mut detections, "landscape", conf, min_confidence, &full_bbox);
    }

    // Nature / forest — lots of green with moderate edge density
    if green_ratio > 0.3 && edge_density > 0.03 {
        let conf = (green_ratio * 1.3).clamp(0.45, 0.88);
        push_if(&mut detections, "nature", conf, min_confidence, &full_bbox);
    }

    // Vegetation / plant (green dominant)
    if green_ratio > 0.25 {
        let conf = (green_ratio * 1.4).clamp(0.4, 0.85);
        push_if(&mut detections, "plant", conf, min_confidence, &full_bbox);
    }

    // Water / ocean / lake — blue bottom or overall with moderate texture
    if blue_ratio > 0.25 && (bottom_blue_ratio > 0.3 || blue_ratio > 0.4)
        && edge_density < 0.15
    {
        let conf = (blue_ratio * 1.2).clamp(0.4, 0.82);
        push_if(&mut detections, "water", conf, min_confidence, &full_bbox);
    }

    // Beach — blue top + brown/warm bottom
    if top_blue_ratio > 0.2 && bottom_brown_ratio > 0.2 && warm_ratio > 0.1 {
        let conf = ((top_blue_ratio + bottom_brown_ratio) * 0.7).clamp(0.4, 0.78);
        push_if(&mut detections, "beach", conf, min_confidence, &full_bbox);
    }

    // Snow / winter — lots of white/bright, low saturation
    if white_ratio > 0.3 && bright_ratio > 0.4 && mean_sat < 0.2 {
        let conf = (white_ratio * 1.3).clamp(0.45, 0.85);
        push_if(&mut detections, "snow", conf, min_confidence, &full_bbox);
    }

    // Mountain — blue sky top + grey/brown bottom + high edge density
    if top_blue_ratio > 0.15 && edge_density > 0.06
        && (grey_ratio > 0.1 || brown_ratio > 0.1)
        && !is_portrait_aspect
    {
        let conf = ((edge_density * 3.0 + top_blue_ratio) * 0.6).clamp(0.4, 0.75);
        push_if(&mut detections, "mountain", conf, min_confidence, &full_bbox);
    }

    // Clouds — bright top, low saturation in upper region
    if top_bright_ratio > 0.5 && top_blue_ratio < 0.3 && desaturated_ratio > 0.3 {
        let conf = (top_bright_ratio * 0.8).clamp(0.4, 0.75);
        push_if(&mut detections, "clouds", conf, min_confidence, &full_bbox);
    }

    // Food — warm centre, moderate saturation, usually close-up
    if warm_ratio > 0.15 && mean_sat > 0.3 && brown_ratio > 0.05
        && !is_panoramic && edge_density > 0.04
        && yellow_ratio + orange_ratio + red_ratio > 0.08
    {
        let conf = ((warm_ratio + mean_sat) * 0.6).clamp(0.4, 0.72);
        push_if(&mut detections, "food", conf, min_confidence, &full_bbox);
    }

    // Flower — high saturation centre, moderate green around it
    if saturated_ratio > 0.2 && green_ratio > 0.1 && mean_sat > 0.35
        && (red_ratio > 0.05 || yellow_ratio > 0.05 || orange_ratio > 0.03)
    {
        let conf = ((saturated_ratio + mean_sat) * 0.7).clamp(0.4, 0.78);
        push_if(&mut detections, "flower", conf, min_confidence, &full_bbox);
    }

    // Architecture / building — high edge density, lots of straight lines, grey/brown
    if edge_density > 0.08 && (grey_ratio > 0.15 || brown_ratio > 0.1)
        && desaturated_ratio > 0.2 && green_ratio < 0.15
    {
        let conf = (edge_density * 2.5 + grey_ratio).clamp(0.4, 0.78);
        push_if(&mut detections, "architecture", conf, min_confidence, &full_bbox);
    }

    // Cityscape — high edge density + grey + various colours
    if edge_density > 0.07 && grey_ratio > 0.1 && lum_std > 50.0
        && !is_portrait_aspect
    {
        let conf = ((edge_density + grey_ratio) * 1.5).clamp(0.4, 0.72);
        push_if(&mut detections, "cityscape", conf, min_confidence, &full_bbox);
    }

    // Indoor — low saturation, moderate/even luminance, warm tones
    if desaturated_ratio > 0.3 && lum_std < 60.0 && mean_lum > 80.0
        && mean_lum < 200.0 && blue_ratio < 0.1 && green_ratio < 0.1
    {
        let conf = (desaturated_ratio * 0.9).clamp(0.4, 0.7);
        push_if(&mut detections, "indoor", conf, min_confidence, &full_bbox);
    }

    // Document / text — very high contrast, mostly white/black, high edges
    if (white_ratio > 0.5 || bright_ratio > 0.6)
        && dark_ratio > 0.05 && edge_density > 0.1
        && desaturated_ratio > 0.6
    {
        let conf = ((white_ratio + edge_density) * 0.8).clamp(0.45, 0.82);
        push_if(&mut detections, "document", conf, min_confidence, &full_bbox);
    }

    // Autumn / fall — orange + brown + yellow mix
    if orange_ratio > 0.05 && brown_ratio > 0.1 && yellow_ratio > 0.03
        && green_ratio < 0.15
    {
        let conf = ((orange_ratio + brown_ratio + yellow_ratio) * 1.2).clamp(0.4, 0.78);
        push_if(&mut detections, "autumn", conf, min_confidence, &full_bbox);
    }

    // Panorama scene tag
    if is_panoramic {
        push_if(&mut detections, "panoramic", 0.9, min_confidence, &full_bbox);
    }

    // Black and white / monochrome
    if desaturated_ratio > 0.85 && mean_sat < 0.05 {
        let conf = (desaturated_ratio * 0.95).clamp(0.6, 0.92);
        push_if(&mut detections, "monochrome", conf, min_confidence, &full_bbox);
    }

    // High contrast / dramatic
    if lum_std > 80.0 && saturated_ratio > 0.15 {
        let conf = ((lum_std / 128.0) * 0.7).clamp(0.4, 0.75);
        push_if(&mut detections, "dramatic", conf, min_confidence, &full_bbox);
    }

    // Bright / high-key lighting
    if bright_ratio > 0.6 && mean_lum > 180.0 {
        push_if(&mut detections, "bright", 0.7, min_confidence, &full_bbox);
    }

    // Dark / moody
    if dark_ratio > 0.5 && mean_lum < 70.0 && dark_ratio < 0.65 {
        push_if(&mut detections, "moody", 0.65, min_confidence, &full_bbox);
    }

    // Portrait-like framing (vertical aspect, moderate variance, warm centre)
    if is_portrait_aspect && lum_std > 30.0 && mean_sat > 0.15 {
        push_if(&mut detections, "portrait", 0.55, min_confidence, &full_bbox);
    }

    // Macro / close-up — very high saturation centre, low depth-of-field hint
    if quality == DetectionQuality::High && mean_sat > 0.4
        && edge_density > 0.05 && !is_panoramic
        && saturated_ratio > 0.25
    {
        // Check if centre is sharper than edges (depth-of-field)
        let centre_edge = compute_region_edge_density(img, w, h,
            w / 4, h / 4, w * 3 / 4, h * 3 / 4);
        let border_edge = edge_density;
        if centre_edge > border_edge * 1.3 {
            push_if(&mut detections, "macro", 0.55, min_confidence, &full_bbox);
        }
    }

    // Fire / flames
    if red_ratio > 0.08 && orange_ratio > 0.06 && yellow_ratio > 0.03
        && warm_ratio > 0.25
    {
        let conf = ((red_ratio + orange_ratio) * 2.0).clamp(0.4, 0.72);
        push_if(&mut detections, "fire", conf, min_confidence, &full_bbox);
    }

    tracing::debug!(
        detections = detections.len(),
        classes = ?detections.iter().map(|d| &d.class_name).collect::<Vec<_>>(),
        "Object detection: classification complete"
    );

    Ok(detections)
}

/// Helper: push a detection if confidence >= threshold.
fn push_if(
    detections: &mut Vec<ObjectDetection>,
    class: &str,
    confidence: f32,
    min_confidence: f32,
    bbox: &BoundingBox,
) {
    if confidence >= min_confidence {
        detections.push(ObjectDetection {
            class_name: class.to_string(),
            confidence,
            bbox: bbox.clone(),
        });
    }
}

/// Compute edge density using Sobel-like gradient approximation.
/// Returns a value 0.0–1.0 where higher = more edges.
fn compute_edge_density(img: &DynamicImage, w: u32, h: u32) -> f32 {
    // Downsample for speed
    let max_dim = 200u32;
    let scale = max_dim as f32 / w.max(h) as f32;
    let (sw, sh) = (
        ((w as f32 * scale) as u32).max(3),
        ((h as f32 * scale) as u32).max(3),
    );
    let small = img.resize_exact(sw, sh, image::imageops::FilterType::Triangle);
    let grey = small.to_luma8();

    let mut edge_sum = 0.0f64;
    let mut count = 0u64;

    for y in 1..(sh - 1) {
        for x in 1..(sw - 1) {
            let gx = grey.get_pixel(x + 1, y).0[0] as f32
                   - grey.get_pixel(x - 1, y).0[0] as f32;
            let gy = grey.get_pixel(x, y + 1).0[0] as f32
                   - grey.get_pixel(x, y - 1).0[0] as f32;
            let mag = (gx * gx + gy * gy).sqrt();
            edge_sum += mag as f64;
            count += 1;
        }
    }

    if count == 0 { return 0.0; }
    // Normalise: max possible gradient magnitude is ~360 (255*sqrt(2))
    (edge_sum / count as f64 / 360.0) as f32
}

/// Compute edge density for a specific region of the image.
fn compute_region_edge_density(
    img: &DynamicImage,
    _w: u32, _h: u32,
    x1: u32, y1: u32, x2: u32, y2: u32,
) -> f32 {
    let cropped = img.crop_imm(x1, y1, x2.saturating_sub(x1).max(3), y2.saturating_sub(y1).max(3));
    let (cw, ch) = cropped.dimensions();
    compute_edge_density(&cropped, cw, ch)
}
