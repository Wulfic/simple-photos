//! Animal face / individual re-identification pipeline.
//!
//! Detects pet species from existing object-detection results and extracts
//! a per-individual embedding so the same pet can be grouped across photos.
//!
//! # Model strategy
//!
//! **Phase 1 (current)** — reuses the already-loaded MobileNetV2 ONNX model
//! via [`crate::ai::object::extract_raw_logits`].  The 1000-dim logit vector
//! is a surprisingly strong per-individual fingerprint: the same cat or dog
//! photographed under varied lighting and poses produces cosine-similar
//! activations because the network has learned species-specific texture and
//! shape features.  No extra model download is needed.
//!
//! **Phase 2 (upgrade path)** — dropping a `pet_embedding.onnx` (e.g.
//! EfficientNet-Lite4, 1280-dim features) into the model directory will be
//! automatically picked up by `init_pet_embedding_model` on next server
//! start.  The schema and API are identical; only the embedding dimension
//! changes, so similarity thresholds may need retuning.
//!
//! # GPU / CPU
//!
//! All ONNX inference honours the global [`crate::ai::session::SessionConfig`]
//! set at startup by [`crate::ai::engine::AiEngine::new`]: CUDA when
//! available (requires `--features cuda` build), CPU otherwise.

use image::DynamicImage;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use ort::session::Session;
use tracing;

use crate::ai::models::BoundingBox;

// ── Phase-2 dedicated model (optional) ──────────────────────────────

const PET_EMB_FILENAME: &str = "pet_embedding.onnx";

static PET_EMB_MODEL: OnceLock<Option<Arc<Mutex<Session>>>> = OnceLock::new();

/// Initialise the optional dedicated pet-embedding model.
///
/// If `pet_embedding.onnx` is present in `model_dir` it is loaded; otherwise
/// we silently fall back to Phase-1 (MobileNetV2 logits from the object
/// classifier).  Does NOT download automatically — operators opt in by
/// placing / symlinking the file in the model directory.
pub fn init_pet_embedding_model(model_dir: &str) {
    PET_EMB_MODEL.get_or_init(|| {
        let p = Path::new(model_dir).join(PET_EMB_FILENAME);
        if !p.exists() {
            tracing::debug!(
                "Pet embedding model not found at {:?} — using MobileNetV2 logits (Phase 1)",
                p
            );
            return None;
        }
        match crate::ai::session::build_session(&p) {
            Ok(sess) => {
                tracing::info!(
                    "Pet embedding model (Phase 2) loaded from {:?}",
                    p
                );
                Some(Arc::new(Mutex::new(sess)))
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to load pet_embedding.onnx: {}. Falling back to Phase 1.",
                    e
                );
                None
            }
        }
    });
}

// ── Species classification ───────────────────────────────────────────

/// Object-detection class names we consider "pets" (triggers re-ID).
const PET_SPECIES: &[(&str, &str)] = &[
    ("dog",      "dog"),
    ("cat",      "cat"),
    ("big cat",  "cat"),   // lions, tigers in zoo photos — treat as cat family
    ("bird",     "bird"),
    ("horse",    "horse"),
    ("rabbit",   "rabbit"),
    ("hamster",  "hamster"),
    ("fish",     "fish"),
];

/// Map an object-detection `class_name` to a pet species string.
///
/// Returns `None` for non-pet classes (landscapes, vehicles, food, …).
pub fn map_to_species(class_name: &str) -> Option<&'static str> {
    PET_SPECIES
        .iter()
        .find_map(|(key, species)| {
            if *key == class_name { Some(*species) } else { None }
        })
}

// ── Embedding extraction ─────────────────────────────────────────────

/// Crop the image to the bbox (normalised 0..1) with `pad` extra context
/// on each side, clamped to image bounds.  Returns the original image if
/// cropping would produce a zero-sized region.
fn crop_with_padding(img: &DynamicImage, bbox: &BoundingBox, pad: f32) -> DynamicImage {
    let iw = img.width() as f32;
    let ih = img.height() as f32;

    let x = (bbox.x - bbox.w * pad).clamp(0.0, 1.0);
    let y = (bbox.y - bbox.h * pad).clamp(0.0, 1.0);
    let w = (bbox.w * (1.0 + 2.0 * pad)).min(1.0 - x);
    let h = (bbox.h * (1.0 + 2.0 * pad)).min(1.0 - y);

    let px = (x * iw) as u32;
    let py = (y * ih) as u32;
    let pw = (w * iw).max(1.0) as u32;
    let ph = (h * ih).max(1.0) as u32;

    if pw < 8 || ph < 8 {
        return img.clone();
    }
    img.crop_imm(px, py, pw, ph)
}

/// L2-normalise a vector in place; returns the same vector for chaining.
/// Does nothing for the zero vector.
fn l2_normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Extract a per-individual pet embedding from `img`.
///
/// When `bbox` is supplied the image is cropped (with 15% padding) to the
/// detected animal before embedding — this dramatically reduces the
/// influence of background scenery on the resulting vector.  The output is
/// always L2-normalised so cosine similarity is well behaved.
///
/// Priority:
/// 1. Phase-2 dedicated `pet_embedding.onnx` (if loaded) — runs
///    EfficientNet-Lite4 inference and returns the 1280-dim penultimate
///    feature vector.
/// 2. Phase-1 fallback — calls
///    [`crate::ai::object::extract_raw_logits`] to get the 1000-dim
///    MobileNetV2 logit vector from the already-loaded classification model.
///
/// Returns `None` when no model is available (degraded mode).
pub fn extract_pet_embedding(
    img: &DynamicImage,
    bbox: Option<&BoundingBox>,
) -> Option<Vec<f32>> {
    // Crop to the animal first when a bbox is provided.  15% padding gives
    // the network a little context (fur boundary, ears) without letting the
    // background dominate the activations.
    let crop: DynamicImage = match bbox {
        Some(b) => crop_with_padding(img, b, 0.15),
        None => img.clone(),
    };

    // Phase 2: dedicated model
    if let Some(emb) = try_phase2_embedding(&crop) {
        return Some(l2_normalize(emb));
    }
    // Phase 1: MobileNetV2 logits
    crate::ai::object::extract_raw_logits(&crop).map(l2_normalize)
}

fn try_phase2_embedding(img: &DynamicImage) -> Option<Vec<f32>> {
    use image::imageops::FilterType;

    let model_arc = PET_EMB_MODEL.get()?.as_ref()?;
    let mut session = model_arc.lock().unwrap_or_else(|p| p.into_inner());

    // EfficientNet-Lite4: 320×320 input, ImageNet normalisation
    const W: usize = 320;
    const H: usize = 320;
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD:  [f32; 3] = [0.229, 0.224, 0.225];

    let resized = img.resize_exact(W as u32, H as u32, FilterType::Triangle);
    let rgb = resized.to_rgb8();

    let mut input = ndarray::Array4::<f32>::zeros((1, 3, H, W));
    for y in 0..H {
        for x in 0..W {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            for c in 0..3 {
                input[[0, c, y, x]] =
                    (pixel[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
    }

    let tensor = ort::value::Tensor::from_array(input).ok()?;
    let outputs = session.run(ort::inputs![tensor]).ok()?;
    let (_shape, data) = outputs[0].try_extract_tensor::<f32>().ok()?;
    Some(data.to_vec())
}


