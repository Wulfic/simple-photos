//! ML inference engine — manages ONNX Runtime sessions for face detection,
//! face embedding, and object detection.
//!
//! Falls back to CPU when GPU (CUDA) is unavailable. The engine is
//! initialised once at startup and shared across async tasks via `Arc`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::AiConfig;

/// Execution provider used for inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionProvider {
    Cpu,
    Cuda,
}

impl std::fmt::Display for ExecutionProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cpu => write!(f, "CPU"),
            Self::Cuda => write!(f, "CUDA"),
        }
    }
}

/// The AI inference engine. Thread-safe, cheaply cloneable via `Arc`.
#[derive(Clone)]
pub struct AiEngine {
    inner: Arc<AiEngineInner>,
}

#[allow(dead_code)] // Fields used once ONNX model loading is active
struct AiEngineInner {
    provider: ExecutionProvider,
    config: AiConfig,
    model_dir: PathBuf,
    /// Whether ONNX models are available for face detection.
    has_face_detection_model: bool,
    /// Whether ONNX models are available for face embedding.
    has_face_embedding_model: bool,
    /// Whether ONNX models are available for object detection.
    has_object_detection_model: bool,
    num_threads: usize,
}

impl AiEngine {
    /// Create a new AI engine, detecting available hardware and models.
    ///
    /// This does NOT fail — if models are missing or GPU is unavailable,
    /// it gracefully degrades. Check `has_*` methods before calling inference.
    pub fn new(config: &AiConfig) -> Self {
        let model_dir = PathBuf::from(&config.model_dir);

        // Detect execution provider
        let provider = if config.gpu_preferred {
            if Self::detect_cuda() {
                tracing::info!("AI engine: CUDA GPU detected, using GPU acceleration");
                ExecutionProvider::Cuda
            } else {
                tracing::info!("AI engine: No CUDA GPU detected, falling back to CPU");
                ExecutionProvider::Cpu
            }
        } else {
            tracing::info!("AI engine: GPU disabled by config, using CPU");
            ExecutionProvider::Cpu
        };

        // Initialise face models (downloads if needed, loads via onnxruntime)
        crate::ai::face::init_face_model(&config.model_dir);

        // Initialise object classification model (MobileNetV2, downloads if needed)
        crate::ai::object::init_classification_model(&config.model_dir);

        // Check for model files (SCRFD or legacy UltraFace for detection,
        // ArcFace w600k_r50 for recognition)
        let has_face_det = model_dir.join("det_10g.onnx").exists()
            || model_dir.join("ultraface-RFB-320.onnx").exists()
            || model_dir.join("face_detection.onnx").exists();
        let has_face_emb = model_dir.join("w600k_r50.onnx").exists()
            || model_dir.join("face_embedding.onnx").exists();
        let has_obj_det = model_dir.join("mobilenetv2-12.onnx").exists()
            || model_dir.join("object_detection.onnx").exists();

        if !has_face_det {
            tracing::warn!("AI engine: face_detection.onnx not found in {:?}", model_dir);
        }
        if !has_face_emb {
            tracing::warn!("AI engine: face_embedding.onnx not found in {:?}", model_dir);
        }
        if !has_obj_det {
            tracing::warn!("AI engine: object_detection.onnx not found in {:?}", model_dir);
        }

        let num_threads = if config.threads == 0 {
            num_cpus::get_physical()
        } else {
            config.threads
        };

        tracing::info!(
            "AI engine initialized: provider={}, threads={}, models: face_det={}, face_emb={}, obj_det={}",
            provider, num_threads, has_face_det, has_face_emb, has_obj_det
        );

        Self {
            inner: Arc::new(AiEngineInner {
                provider,
                config: config.clone(),
                model_dir,
                has_face_detection_model: has_face_det,
                has_face_embedding_model: has_face_emb,
                has_object_detection_model: has_obj_det,
                num_threads,
            }),
        }
    }

    /// The active execution provider.
    pub fn provider(&self) -> ExecutionProvider {
        self.inner.provider
    }

    /// Whether GPU is being used.
    #[allow(dead_code)] // Ready for ONNX integration
    pub fn is_gpu(&self) -> bool {
        self.inner.provider == ExecutionProvider::Cuda
    }

    /// Whether face detection is available (model loaded).
    pub fn has_face_detection(&self) -> bool {
        self.inner.has_face_detection_model
    }

    /// Whether face embedding is available (model loaded).
    #[allow(dead_code)] // Ready for ONNX integration
    pub fn has_face_embedding(&self) -> bool {
        self.inner.has_face_embedding_model
    }

    /// Whether object detection is available (model loaded).
    pub fn has_object_detection(&self) -> bool {
        self.inner.has_object_detection_model
    }

    /// Whether any AI capability is available.
    pub fn has_any_capability(&self) -> bool {
        self.has_face_detection() || self.has_object_detection()
    }

    /// The AI config.
    #[allow(dead_code)] // Ready for ONNX integration
    pub fn config(&self) -> &AiConfig {
        &self.inner.config
    }

    /// Number of inference threads.
    #[allow(dead_code)] // Ready for ONNX integration
    pub fn num_threads(&self) -> usize {
        self.inner.num_threads
    }

    /// Path to the model directory.
    #[allow(dead_code)] // Ready for ONNX integration
    pub fn model_dir(&self) -> &Path {
        &self.inner.model_dir
    }

    /// Detect CUDA availability by checking for NVIDIA driver / libraries.
    fn detect_cuda() -> bool {
        // Check for nvidia-smi (Linux)
        if let Ok(output) = std::process::Command::new("nvidia-smi")
            .arg("--query-gpu=name")
            .arg("--format=csv,noheader")
            .output()
        {
            if output.status.success() {
                let gpu_name = String::from_utf8_lossy(&output.stdout);
                tracing::info!("AI engine: found GPU: {}", gpu_name.trim());
                return true;
            }
        }

        // Check for CUDA libraries
        let cuda_paths = [
            "/usr/local/cuda/lib64/libcudart.so",
            "/usr/lib/x86_64-linux-gnu/libcudart.so",
        ];
        for path in &cuda_paths {
            if Path::new(path).exists() {
                return true;
            }
        }

        false
    }
}
