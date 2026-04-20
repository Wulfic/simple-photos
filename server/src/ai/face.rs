//! Face detection and embedding extraction.
//!
//! Uses SCRFD (InsightFace) for face detection and ArcFace for face recognition,
//! matching the approach used by Immich and other production photo management apps.
//! Falls back to UltraFace-RFB-320 → heuristic detector if SCRFD unavailable.
//!
//! The pipeline:
//! 1. Decode image → letterbox-resize to 640×640 (SCRFD input)
//! 2. Run SCRFD detection → bounding boxes + 5 facial landmarks + confidence
//! 3. For each face: align via landmarks (norm_crop) → 112×112 → ArcFace embedding
//! 4. Return `Vec<FaceDetection>` with bounding boxes and 512-dim embeddings

use crate::ai::models::{BoundingBox, FaceDetection};
use image::{DynamicImage, GenericImageView, RgbImage, imageops::FilterType};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use tracing;
use ort::session::Session;

/// Convert ort errors (which aren't Send+Sync) to anyhow errors.
fn ort_err<R>(r: Result<R, impl std::fmt::Display>) -> anyhow::Result<R> {
    r.map_err(|e| anyhow::anyhow!("{e}"))
}

// ── Model constants ─────────────────────────────────────────────────

/// SCRFD face detection model (from InsightFace buffalo_l, same as Immich).
/// Input: [1, 3, 640, 640], output: 9 tensors (3 scales × {scores, bboxes, landmarks}).
const DET_WIDTH: usize = 640;
const DET_HEIGHT: usize = 640;
const DET_MODEL_URL: &str =
    "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx";
const DET_MODEL_FILENAME: &str = "det_10g.onnx";

/// ArcFace recognition model (w600k_r50) for 512-dim face embeddings.
/// Input: [1, 3, 112, 112] aligned face, output: [1, 512].
const REC_SIZE: usize = 112;
const REC_MODEL_URL: &str =
    "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx";
const REC_MODEL_FILENAME: &str = "w600k_r50.onnx";

/// Legacy UltraFace model (kept as secondary fallback).
const LEGACY_WIDTH: usize = 320;
const LEGACY_HEIGHT: usize = 240;
const LEGACY_MODEL_URL: &str = "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx";
const LEGACY_MODEL_FILENAME: &str = "ultraface-RFB-320.onnx";

/// SCRFD anchor configuration: feature pyramid strides.
const FEAT_STRIDES: [usize; 3] = [8, 16, 32];
/// Number of anchors per spatial position.
const NUM_ANCHORS: usize = 2;
/// SCRFD input normalisation: mean=127.5, std=128.0.
const DET_INPUT_MEAN: f32 = 127.5;
const DET_INPUT_STD: f32 = 128.0;
/// ArcFace input normalisation: mean=127.5, std=127.5 → maps [0,255]→[-1,1].
const REC_INPUT_MEAN: f32 = 127.5;
const REC_INPUT_STD: f32 = 127.5;
/// NMS IoU threshold.
const NMS_THRESH: f32 = 0.4;

/// ArcFace alignment template: 5-point landmarks for a 112×112 output.
const ARCFACE_DST: [[f32; 2]; 5] = [
    [38.2946, 51.6963], // left eye
    [73.5318, 51.5014], // right eye
    [56.0252, 71.7366], // nose tip
    [41.5493, 92.3655], // left mouth corner
    [70.7299, 92.2041], // right mouth corner
];

// ── Model singletons ────────────────────────────────────────────────

static DET_MODEL: OnceLock<Option<Arc<Mutex<Session>>>> = OnceLock::new();
static REC_MODEL: OnceLock<Option<Arc<Mutex<Session>>>> = OnceLock::new();
static LEGACY_MODEL: OnceLock<Option<Arc<Mutex<Session>>>> = OnceLock::new();

/// Raw detection before final NMS and coordinate mapping.
struct RawDetection {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    score: f32,
    /// 5 facial landmarks (x, y) in pixel coords of the det_img.
    landmarks: [[f32; 2]; 5],
}

// ── Initialisation ──────────────────────────────────────────────────

/// Download and load face models from the given model directory.
/// Tries SCRFD first, falls back to UltraFace legacy, then heuristic.
pub fn init_face_model(model_dir: &str) {
    let dir = Path::new(model_dir);
    let dir_exists = dir.is_dir();

    if !dir_exists {
        tracing::info!("Model directory {:?} does not exist — heuristic-only mode", dir);
    }

    // SCRFD detection model (primary)
    DET_MODEL.get_or_init(|| {
        let p = dir.join(DET_MODEL_FILENAME);
        if !p.exists() {
            if !dir_exists {
                return None; // Don't try to download if dir doesn't exist
            }
            tracing::info!("SCRFD det model not found at {:?}, downloading…", p);
            if let Err(e) = download_model(DET_MODEL_URL, &p, 1_000_000) {
                tracing::warn!("SCRFD download failed: {}. Will try legacy model.", e);
                return None;
            }
        }
        match load_onnx_det(&p) {
            Ok(session) => {
                tracing::info!("SCRFD detection model loaded from {:?}", p);
                Some(Arc::new(Mutex::new(session)))
            }
            Err(e) => {
                tracing::warn!("Failed to load SCRFD model: {}. Will try legacy model.", e);
                None
            }
        }
    });

    // ArcFace recognition model
    REC_MODEL.get_or_init(|| {
        let p = dir.join(REC_MODEL_FILENAME);
        if !p.exists() {
            if !dir_exists {
                return None;
            }
            tracing::info!("ArcFace model not found at {:?}, downloading…", p);
            if let Err(e) = download_model(REC_MODEL_URL, &p, 10_000_000) {
                tracing::warn!("ArcFace download failed: {}. Using histogram embeddings.", e);
                return None;
            }
        }
        match load_onnx_rec(&p) {
            Ok(session) => {
                tracing::info!("ArcFace recognition model loaded from {:?}", p);
                Some(Arc::new(Mutex::new(session)))
            }
            Err(e) => {
                tracing::warn!("Failed to load ArcFace model: {}. Using histogram embeddings.", e);
                None
            }
        }
    });

    // Legacy UltraFace (secondary fallback for detection)
    LEGACY_MODEL.get_or_init(|| {
        if DET_MODEL.get().and_then(|m| m.as_ref()).is_some() {
            return None; // SCRFD loaded, no need for legacy
        }
        let p = dir.join(LEGACY_MODEL_FILENAME);
        if !p.exists() {
            if !dir_exists {
                return None;
            }
            tracing::info!("Legacy UltraFace not found at {:?}, downloading…", p);
            if let Err(e) = download_model(LEGACY_MODEL_URL, &p, 100_000) {
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

fn download_model(url: &str, dest: &Path, min_size: usize) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    tracing::info!("Downloading model from {url}…");
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
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
        return Err(format!("File too small ({} bytes), expected ≥{min_size}", bytes.len()));
    }
    std::fs::write(dest, &bytes).map_err(|e| format!("write: {e}"))?;
    tracing::info!("Model downloaded ({} bytes) → {:?}", bytes.len(), dest);
    Ok(())
}

fn load_onnx_det(path: &Path) -> anyhow::Result<Session> {
    let session = ort_err(ort_err(Session::builder())?
        .with_intra_threads(1))?
        .commit_from_file(path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(session)
}

fn load_onnx_rec(path: &Path) -> anyhow::Result<Session> {
    let session = ort_err(ort_err(Session::builder())?
        .with_intra_threads(1))?
        .commit_from_file(path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(session)
}

fn load_onnx_legacy(path: &Path) -> anyhow::Result<Session> {
    let session = ort_err(ort_err(Session::builder())?
        .with_intra_threads(1))?
        .commit_from_file(path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(session)
}

// ── Detection entry point ───────────────────────────────────────────

/// Detect faces from an already-decoded image.
/// Tries SCRFD → UltraFace legacy → heuristic fallback.
pub fn detect_faces_from_image(
    img: &DynamicImage,
    min_confidence: f32,
) -> anyhow::Result<Vec<FaceDetection>> {
    let (w, h) = img.dimensions();
    if w < 20 || h < 20 {
        return Ok(vec![]);
    }

    // Try SCRFD (best)
    if let Some(det) = DET_MODEL.get().and_then(|m| m.as_ref()) {
        let mut session = det.lock().unwrap();
        return detect_faces_scrfd(img, min_confidence, &mut session);
    }

    // Try legacy UltraFace (acceptable)
    if let Some(legacy) = LEGACY_MODEL.get().and_then(|m| m.as_ref()) {
        let mut session = legacy.lock().unwrap();
        return detect_faces_legacy(img, min_confidence, &mut session);
    }

    // Last resort: heuristic
    detect_faces_heuristic(img, min_confidence)
}

// ── SCRFD detection (InsightFace / Immich approach) ─────────────────

fn detect_faces_scrfd(
    img: &DynamicImage,
    min_confidence: f32,
    model: &mut Session,
) -> anyhow::Result<Vec<FaceDetection>> {
    let (orig_w, orig_h) = img.dimensions();

    // Letterbox resize: maintain aspect ratio, pad with black
    let im_ratio = orig_h as f32 / orig_w as f32;
    let model_ratio = DET_HEIGHT as f32 / DET_WIDTH as f32;
    let (new_w, new_h) = if im_ratio > model_ratio {
        let nh = DET_HEIGHT as u32;
        let nw = (nh as f32 / im_ratio) as u32;
        (nw.max(1), nh)
    } else {
        let nw = DET_WIDTH as u32;
        let nh = (nw as f32 * im_ratio) as u32;
        (nw, nh.max(1))
    };
    let det_scale = new_h as f32 / orig_h as f32;
    let resized = img.resize_exact(new_w, new_h, FilterType::Triangle);
    let rgb = resized.to_rgb8();

    // Build input tensor [1, 3, 640, 640] — letterbox-padded, normalised.
    let pad_val = -DET_INPUT_MEAN / DET_INPUT_STD; // (0 - 127.5) / 128.0 ≈ -0.996
    let mut input =
        ndarray::Array4::<f32>::from_elem([1, 3, DET_HEIGHT, DET_WIDTH], pad_val);
    for y in 0..new_h as usize {
        for x in 0..new_w as usize {
            let p = rgb.get_pixel(x as u32, y as u32);
            input[[0, 0, y, x]] = (p[0] as f32 - DET_INPUT_MEAN) / DET_INPUT_STD;
            input[[0, 1, y, x]] = (p[1] as f32 - DET_INPUT_MEAN) / DET_INPUT_STD;
            input[[0, 2, y, x]] = (p[2] as f32 - DET_INPUT_MEAN) / DET_INPUT_STD;
        }
    }

    let input_tensor = ort_err(ort::value::Tensor::from_array(input))?;
    let outputs = ort_err(model.run(ort::inputs![input_tensor]))?;

    let num_outputs = outputs.len();
    let _use_kps = num_outputs == 9;

    // ── Shape-based output re-mapping ──────────────────────────────
    struct StrideOutputs {
        scores: Vec<f32>,
        bboxes: Vec<f32>,
        kps: Vec<f32>,
    }

    // Expected anchor counts for each stride level.
    let expected_n: Vec<usize> = FEAT_STRIDES
        .iter()
        .map(|&s| (DET_HEIGHT / s) * (DET_WIDTH / s) * NUM_ANCHORS)
        .collect();

    let mut stride_map: std::collections::HashMap<usize, StrideOutputs> =
        std::collections::HashMap::new();

    for (i, (name, value)) in outputs.iter().enumerate() {
        let (shape_ref, flat_data) = ort_err(value.try_extract_tensor::<f32>())?;
        let shape: Vec<usize> = shape_ref.iter().map(|&d| d as usize).collect();
        let (n, c) = match shape.len() {
            1 => (shape[0], 1usize),
            2 => (shape[0], shape[1]),
            3 => (shape[1], shape[2]),
            _ => continue,
        };

        tracing::debug!(
            output_idx = i,
            name = %name,
            shape = ?shape,
            n = n,
            c = c,
            "SCRFD output tensor"
        );

        // Find which stride this N belongs to.
        let stride_idx = match expected_n.iter().position(|&en| en == n) {
            Some(idx) => idx,
            None => {
                tracing::debug!(output_idx = i, n = n, "SCRFD: no matching stride for N");
                continue;
            }
        };

        let entry = stride_map
            .entry(stride_idx)
            .or_insert_with(|| StrideOutputs {
                scores: vec![],
                bboxes: vec![],
                kps: vec![],
            });

        let flat: Vec<f32> = flat_data.to_vec();
        // Log first few values for debugging
        let sample: Vec<f32> = flat.iter().copied().take(5).collect();
        tracing::debug!(
            output_idx = i,
            stride_idx = stride_idx,
            c = c,
            sample_values = format!("{:?}", sample),
            "SCRFD tensor values"
        );
        match c {
            1 => entry.scores = flat,
            4 => entry.bboxes = flat,
            10 => entry.kps = flat,
            _ => {}
        }
    }

    let mut all_dets: Vec<RawDetection> = Vec::new();

    for (idx, &stride) in FEAT_STRIDES.iter().enumerate() {
        let so = match stride_map.get(&idx) {
            Some(s) => s,
            None => continue,
        };

        // Log score statistics for debugging
        if !so.scores.is_empty() {
            let max_score = so.scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let min_score = so.scores.iter().copied().fold(f32::INFINITY, f32::min);
            let above_thresh = so.scores.iter().filter(|&&s| s >= min_confidence).count();
            // Sample first 5 scores for debugging
            let sample: Vec<f32> = so.scores.iter().copied().take(5).collect();
            tracing::debug!(
                stride = stride,
                score_count = so.scores.len(),
                bbox_count = so.bboxes.len(),
                kps_count = so.kps.len(),
                max_score = format!("{:.6}", max_score),
                min_score = format!("{:.6}", min_score),
                above_threshold = above_thresh,
                threshold = format!("{:.2}", min_confidence),
                sample_scores = format!("{:?}", sample),
                "SCRFD stride scores"
            );
        }

        let height = DET_HEIGHT / stride;
        let width = DET_WIDTH / stride;

        // Generate anchor centres for this stride
        let mut anchors = Vec::with_capacity(height * width * NUM_ANCHORS);
        for ay in 0..height {
            for ax in 0..width {
                let cx = ax as f32 * stride as f32;
                let cy = ay as f32 * stride as f32;
                for _ in 0..NUM_ANCHORS {
                    anchors.push((cx, cy));
                }
            }
        }

        // Determine the number of candidates in this feature map
        let n = anchors.len();

        for i in 0..n {
            let score = if i < so.scores.len() {
                so.scores[i]
            } else {
                break;
            };

            if score < min_confidence {
                continue;
            }

            let bi = i * 4;
            if bi + 3 >= so.bboxes.len() {
                break;
            }

            let (cx, cy) = anchors[i];
            // distance2bbox: centre ± distance*stride
            let s = stride as f32;
            let x1 = cx - so.bboxes[bi] * s;
            let y1 = cy - so.bboxes[bi + 1] * s;
            let x2 = cx + so.bboxes[bi + 2] * s;
            let y2 = cy + so.bboxes[bi + 3] * s;

            // distance2kps
            let mut lmk = [[0.0f32; 2]; 5];
            if !so.kps.is_empty() {
                let ki = i * 10;
                if ki + 9 < so.kps.len() {
                    for j in 0..5 {
                        lmk[j][0] = cx + so.kps[ki + j * 2] * s;
                        lmk[j][1] = cy + so.kps[ki + j * 2 + 1] * s;
                    }
                }
            }

            all_dets.push(RawDetection {
                x1,
                y1,
                x2,
                y2,
                score,
                landmarks: lmk,
            });
        }
    }

    // Map to original image coordinates
    for d in &mut all_dets {
        d.x1 /= det_scale;
        d.y1 /= det_scale;
        d.x2 /= det_scale;
        d.y2 /= det_scale;
        for lm in &mut d.landmarks {
            lm[0] /= det_scale;
            lm[1] /= det_scale;
        }
    }

    // Sort by score descending for NMS
    all_dets.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // NMS
    let keep = nms_raw(&all_dets, NMS_THRESH);

    let mut results = Vec::new();
    for i in keep {
        let d = &all_dets[i];
        let bw = (d.x2 - d.x1).max(0.0);
        let bh = (d.y2 - d.y1).max(0.0);
        // Skip tiny detections (likely false positives) — face must be at least
        // 2% of image width/height to be meaningful
        let min_face_px = (orig_w.min(orig_h) as f32 * 0.02).max(20.0);
        if bw < min_face_px || bh < min_face_px {
            continue;
        }
        // Normalise to 0..1
        let bbox = BoundingBox {
            x: (d.x1 / orig_w as f32).clamp(0.0, 1.0),
            y: (d.y1 / orig_h as f32).clamp(0.0, 1.0),
            w: (bw / orig_w as f32).clamp(0.0, 1.0),
            h: (bh / orig_h as f32).clamp(0.0, 1.0),
        };

        // Extract embedding (ArcFace if available, else histogram)
        let embedding = extract_face_embedding_with_landmarks(img, &bbox, &d.landmarks);

        results.push(FaceDetection {
            bbox,
            confidence: d.score,
            embedding,
        });
    }

    tracing::debug!(
        detections = results.len(),
        "Face detection (SCRFD): complete"
    );

    Ok(results)
}

/// NMS operating on RawDetection — returns indices to keep.
fn nms_raw(dets: &[RawDetection], thresh: f32) -> Vec<usize> {
    let mut keep = Vec::new();
    let mut suppressed = vec![false; dets.len()];

    for i in 0..dets.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(i);
        let a = &dets[i];
        for j in (i + 1)..dets.len() {
            if suppressed[j] {
                continue;
            }
            let b = &dets[j];
            let ix1 = a.x1.max(b.x1);
            let iy1 = a.y1.max(b.y1);
            let ix2 = a.x2.min(b.x2);
            let iy2 = a.y2.min(b.y2);
            let iw = (ix2 - ix1).max(0.0);
            let ih = (iy2 - iy1).max(0.0);
            let inter = iw * ih;
            let area_a = (a.x2 - a.x1) * (a.y2 - a.y1);
            let area_b = (b.x2 - b.x1) * (b.y2 - b.y1);
            let union = area_a + area_b - inter;
            if union > 0.0 && inter / union > thresh {
                suppressed[j] = true;
            }
        }
    }
    keep
}

// ── ArcFace embedding extraction ────────────────────────────────────

/// Extract a face embedding using ArcFace (512-dim) if available,
/// falling back to handcrafted histogram features (128-dim).
fn extract_face_embedding_with_landmarks(
    img: &DynamicImage,
    bbox: &BoundingBox,
    landmarks: &[[f32; 2]; 5],
) -> Vec<f32> {
    // Check if we have valid landmarks (non-zero)
    let has_landmarks = landmarks.iter().any(|lm| lm[0] > 0.0 || lm[1] > 0.0);

    if let Some(rec) = REC_MODEL.get().and_then(|m| m.as_ref()) {
        if has_landmarks {
            let mut session = rec.lock().unwrap();
            match extract_arcface_embedding(img, landmarks, &mut session) {
                Ok(emb) => return emb,
                Err(e) => tracing::warn!("ArcFace embedding failed: {e}, using histogram"),
            }
        }
    }

    // Fallback: histogram-based embedding
    extract_histogram_embedding(img, bbox)
}

/// ArcFace embedding: align face using 5 landmarks, run recognition model.
fn extract_arcface_embedding(
    img: &DynamicImage,
    landmarks: &[[f32; 2]; 5],
    rec_model: &mut Session,
) -> anyhow::Result<Vec<f32>> {
    // Align face: compute similarity transform from detected landmarks to template
    let aligned = norm_crop(img, landmarks, REC_SIZE as u32);

    // Build input tensor [1, 3, 112, 112]
    let rgb = aligned.to_rgb8();
    let mut input = ndarray::Array4::<f32>::zeros([1, 3, REC_SIZE, REC_SIZE]);
    for y in 0..REC_SIZE {
        for x in 0..REC_SIZE {
            let p = rgb.get_pixel(x as u32, y as u32);
            input[[0, 0, y, x]] = (p[0] as f32 - REC_INPUT_MEAN) / REC_INPUT_STD;
            input[[0, 1, y, x]] = (p[1] as f32 - REC_INPUT_MEAN) / REC_INPUT_STD;
            input[[0, 2, y, x]] = (p[2] as f32 - REC_INPUT_MEAN) / REC_INPUT_STD;
        }
    }

    let input_tensor = ort_err(ort::value::Tensor::from_array(input))?;
    let outputs = ort_err(rec_model.run(ort::inputs![input_tensor]))?;
    let (_shape, emb_data) = ort_err(outputs[0].try_extract_tensor::<f32>())?;
    let mut embedding: Vec<f32> = emb_data.to_vec();

    // L2 normalise
    let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for v in &mut embedding {
            *v /= norm;
        }
    }

    Ok(embedding)
}

/// Align a face via similarity transform from 5 detected landmarks to
/// the ArcFace template (same as InsightFace `norm_crop`).
fn norm_crop(img: &DynamicImage, landmarks: &[[f32; 2]; 5], size: u32) -> DynamicImage {
    // Estimate 2D similarity transform: dst = M * src
    // M = [[a, -b, tx], [b, a, ty]]
    // Solve least-squares: for each (src_i, dst_i):
    //   dst_x = a*src_x - b*src_y + tx
    //   dst_y = b*src_x + a*src_y + ty
    let src = landmarks;
    let dst = &ARCFACE_DST;

    // Build 10×4 system: A @ [a, b, tx, ty]^T = rhs
    let mut ata = [[0.0f64; 4]; 4];
    let mut atb = [0.0f64; 4];

    for i in 0..5 {
        let sx = src[i][0] as f64;
        let sy = src[i][1] as f64;
        let dx = dst[i][0] as f64;
        let dy = dst[i][1] as f64;

        // Row 1: [sx, -sy, 1, 0] → dx
        let r1 = [sx, -sy, 1.0, 0.0];
        // Row 2: [sy. sx, 0, 1] → dy
        let r2 = [sy, sx, 0.0, 1.0];

        for j in 0..4 {
            for k in 0..4 {
                ata[j][k] += r1[j] * r1[k] + r2[j] * r2[k];
            }
            atb[j] += r1[j] * dx + r2[j] * dy;
        }
    }

    // Solve 4×4 system via Gaussian elimination
    let params = solve_4x4(ata, atb);
    let a = params[0] as f32;
    let b = params[1] as f32;
    let tx = params[2] as f32;
    let ty = params[3] as f32;

    // Warp: for each (dst_x, dst_y) in output, find (src_x, src_y) in input
    // Forward: dst = M * src where M = [[a, -b, tx], [b, a, ty]]
    // Inverse: src = M_inv * dst
    let det = a * a + b * b;
    if det < 1e-10 {
        // Degenerate transform; fall back to simple crop
        return crop_face_simple(img, landmarks, size);
    }
    let inv_det = 1.0 / det;
    // M_inv = [[a, b, -(a*tx + b*ty)], [-b, a, (b*tx - a*ty)]] / det
    let ia = a * inv_det;
    let ib = b * inv_det;
    let itx = -(a * tx + b * ty) * inv_det;
    let ity = (b * tx - a * ty) * inv_det;

    let (iw, ih) = img.dimensions();
    let rgb_in = img.to_rgb8();
    let mut out = RgbImage::new(size, size);

    for dy in 0..size {
        for dx in 0..size {
            let dxf = dx as f32;
            let dyf = dy as f32;
            let src_x = ia * dxf + ib * dyf + itx;
            let src_y = -ib * dxf + ia * dyf + ity;

            // Bilinear interpolation
            let sx = src_x.floor() as i32;
            let sy = src_y.floor() as i32;
            let fx = src_x - sx as f32;
            let fy = src_y - sy as f32;

            let sample = |x: i32, y: i32| -> [f32; 3] {
                let cx = x.clamp(0, iw as i32 - 1) as u32;
                let cy = y.clamp(0, ih as i32 - 1) as u32;
                let p = rgb_in.get_pixel(cx, cy);
                [p[0] as f32, p[1] as f32, p[2] as f32]
            };

            if sx >= 0 && sx + 1 < iw as i32 && sy >= 0 && sy + 1 < ih as i32 {
                let tl = sample(sx, sy);
                let tr = sample(sx + 1, sy);
                let bl = sample(sx, sy + 1);
                let br = sample(sx + 1, sy + 1);

                let mut pixel = [0u8; 3];
                for c in 0..3 {
                    let v = tl[c] * (1.0 - fx) * (1.0 - fy)
                        + tr[c] * fx * (1.0 - fy)
                        + bl[c] * (1.0 - fx) * fy
                        + br[c] * fx * fy;
                    pixel[c] = v.clamp(0.0, 255.0) as u8;
                }
                out.put_pixel(dx, dy, image::Rgb(pixel));
            }
            // Out-of-bounds pixels stay black (border value 0)
        }
    }

    DynamicImage::ImageRgb8(out)
}

/// Simple centre-crop fallback when landmarks are degenerate.
fn crop_face_simple(img: &DynamicImage, landmarks: &[[f32; 2]; 5], size: u32) -> DynamicImage {
    let (iw, ih) = img.dimensions();
    let cx: f32 = landmarks.iter().map(|l| l[0]).sum::<f32>() / 5.0;
    let cy: f32 = landmarks.iter().map(|l| l[1]).sum::<f32>() / 5.0;
    let half = size as f32 * 0.8;
    let x = (cx - half).max(0.0) as u32;
    let y = (cy - half).max(0.0) as u32;
    let w = (half * 2.0).min((iw - x) as f32) as u32;
    let h = (half * 2.0).min((ih - y) as f32) as u32;
    img.crop_imm(x.min(iw.saturating_sub(1)), y.min(ih.saturating_sub(1)), w.max(1), h.max(1))
        .resize_exact(size, size, FilterType::Triangle)
}

/// Solve 4×4 linear system via Gaussian elimination with partial pivoting.
fn solve_4x4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> [f64; 4] {
    for col in 0..4 {
        // Pivot
        let mut max_row = col;
        let mut max_val = a[col][col].abs();
        for row in (col + 1)..4 {
            if a[row][col].abs() > max_val {
                max_val = a[row][col].abs();
                max_row = row;
            }
        }
        a.swap(col, max_row);
        b.swap(col, max_row);

        let pivot = a[col][col];
        if pivot.abs() < 1e-12 {
            return [0.0; 4];
        }
        for j in col..4 {
            a[col][j] /= pivot;
        }
        b[col] /= pivot;

        for row in 0..4 {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            for j in col..4 {
                a[row][j] -= factor * a[col][j];
            }
            b[row] -= factor * b[col];
        }
    }
    b
}

// ── Legacy UltraFace detection ──────────────────────────────────────

fn detect_faces_legacy(
    img: &DynamicImage,
    min_confidence: f32,
    model: &mut Session,
) -> anyhow::Result<Vec<FaceDetection>> {
    let (w, h) = img.dimensions();
    if w < 20 || h < 20 {
        return Ok(vec![]);
    }

    let resized = img.resize_exact(LEGACY_WIDTH as u32, LEGACY_HEIGHT as u32, FilterType::Triangle);
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
            bbox: BoundingBox { x: x_min, y: y_min, w: bw, h: bh },
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

fn detect_faces_heuristic(
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

// ── Public embedding entry point (backward-compatible) ──────────────

/// Extract a face embedding.
///
/// If the ArcFace model is loaded, this produces a 512-dim embedding from a
/// simple centre crop (no landmarks). For best results, use SCRFD detection
/// which provides landmarks internally. When no model is available, falls
/// back to a 128-dim histogram/gradient feature vector.
pub fn extract_face_embedding(
    img: &DynamicImage,
    bbox: &BoundingBox,
) -> Vec<f32> {
    if let Some(rec_arc) = REC_MODEL.get().and_then(|m| m.as_ref()) {
        let mut rec = rec_arc.lock().unwrap();
        // Synthesise approximate landmarks from bbox centre for non-SCRFD path
        let (iw, ih) = img.dimensions();
        let cx = (bbox.x + bbox.w / 2.0) * iw as f32;
        let cy = (bbox.y + bbox.h / 2.0) * ih as f32;
        let fw = bbox.w * iw as f32;
        let fh = bbox.h * ih as f32;

        // Approximate landmark positions relative to bbox
        let landmarks = [
            [cx - fw * 0.17, cy - fh * 0.12], // left eye
            [cx + fw * 0.17, cy - fh * 0.12], // right eye
            [cx, cy + fh * 0.05],              // nose
            [cx - fw * 0.14, cy + fh * 0.22],  // left mouth
            [cx + fw * 0.14, cy + fh * 0.22],  // right mouth
        ];

        match extract_arcface_embedding(img, &landmarks, &mut rec) {
            Ok(emb) => return emb,
            Err(e) => tracing::debug!("ArcFace fallback failed: {e}"),
        }
    }

    extract_histogram_embedding(img, bbox)
}

/// Histogram-based 128-dim embedding (fallback when no ArcFace model).
fn extract_histogram_embedding(
    img: &DynamicImage,
    bbox: &BoundingBox,
) -> Vec<f32> {
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
    let grad_norm = if grad_total > 0 { grad_total as f32 } else { 1.0 };
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

/// NMS on FaceDetection vec (used by legacy and heuristic paths).
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

    if union < 1e-6 { 0.0 } else { inter / union }
}

/// Skin-colour density using YCbCr colour space (heuristic fallback).
fn skin_density_score_ycbcr(
    rgb: &RgbImage,
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
            let cb = 128.0 + (-0.169 * r - 0.331 * g + 0.500 * b);
            let cr = 128.0 + (0.500 * r - 0.419 * g - 0.081 * b);
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
    if density < 0.30 || density > 0.80 {
        return 0.0;
    }
    ((density - 0.25) / 0.45).clamp(0.0, 0.95)
}

/// Face-like structure score (heuristic fallback).
fn face_structure_score(
    rgb: &RgbImage,
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
        if edge_ratio >= 0.05 && edge_ratio <= 0.35 {
            score += 0.2;
        }
    }

    score
}
