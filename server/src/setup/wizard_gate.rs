//! Middleware that blocks every non-setup API endpoint until the first-run
//! wizard has been finalized.
//!
//! While `server_settings.wizard_completed != "true"` the server returns a
//! `403 Forbidden` JSON body with `error_code: "wizard_incomplete"` for any
//! gated route. The web frontend reads that code and redirects the user to
//! `/welcome` (the wizard) instead of showing an error.
//!
//! The middleware is applied to the *gated* router only — see
//! [`crate::routes`]. Setup, auth, health, and the LAN discovery info
//! endpoint are intentionally outside the gate so the wizard itself can
//! still function.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::setup::handlers::is_wizard_completed;
use crate::state::AppState;

/// Reject every gated request with 403 while the first-run wizard is
/// incomplete. The body is structured so the frontend can dispatch on
/// `error_code` and redirect rather than render an error page.
pub async fn require_wizard_completed(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    match is_wizard_completed(&state).await {
        Ok(true) => next.run(request).await,
        Ok(false) => (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "First-run setup wizard has not been completed.",
                "error_code": "wizard_incomplete",
            })),
        )
            .into_response(),
        Err(err) => {
            // Fail closed: if the DB lookup itself fails, do not leak the
            // rest of the API. Surface the error so the operator sees it.
            tracing::error!("wizard_gate: failed to read wizard_completed flag: {err}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to verify setup state.",
                    "error_code": "wizard_check_failed",
                })),
            )
                .into_response()
        }
    }
}
