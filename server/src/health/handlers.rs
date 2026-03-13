//! Health check handler.

use axum::Json;
use serde_json::{json, Value};

/// GET /health — lightweight health check for load balancers and uptime monitors.
pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "simple-photos",
        "version": "0.6.9"
    }))
}
