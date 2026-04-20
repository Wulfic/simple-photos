//! AI face and object recognition module.
//!
//! Provides GPU-accelerated (CUDA) or CPU-fallback face detection,
//! face embedding extraction, face clustering, object detection,
//! and automatic tag application.
//!
//! The module is entirely behind the `ai.enabled` config toggle.
//! When disabled, no background tasks are spawned and all endpoints
//! return 404 or indicate disabled status.

pub mod clustering;
pub mod engine;
pub mod face;
pub mod handlers;
pub mod imagenet_labels;
pub mod models;
pub mod object;
pub mod processor;
pub mod tagging;
