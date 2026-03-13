//! Unified error type for all API handlers.
//!
//! Each `AppError` variant maps to an HTTP status code with a JSON body.
//! Internal errors (database, anyhow) are logged server-side with full detail
//! but return only a generic "Internal server error" to the client to avoid
//! leaking implementation details.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Application error type returned by all handlers via `Result<T, AppError>`.
/// Implements `IntoResponse` to convert each variant into the appropriate HTTP
/// status code and JSON error body.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found")]
    NotFound,

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Payload too large")]
    PayloadTooLarge,

    #[error("Too many requests")]
    TooManyRequests,

    #[error("Internal error: {0}")]
    Internal(String),

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

/// Convert `AppError` into an Axum HTTP response.
///
/// Internal errors (`Sqlx`, `Anyhow`, `Internal`) are logged with full detail
/// but return a generic 500 response — never expose internal error messages to
/// the client.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::PayloadTooLarge => {
                (StatusCode::PAYLOAD_TOO_LARGE, "Payload too large".into())
            }
            AppError::TooManyRequests => {
                (StatusCode::TOO_MANY_REQUESTS, "Too many requests".into())
            }
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            AppError::Sqlx(e) => {
                tracing::error!("Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            AppError::Anyhow(e) => {
                tracing::error!("Internal error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
        };

        let body = axum::Json(json!({ "error": message }));
        (status, body).into_response()
    }
}

/// Allow `axum::response::Response` to convert into `AppError` so rate limiters
/// can return a pre-built 429 response through the `?` operator.
impl From<Response> for AppError {
    fn from(_response: Response) -> Self {
        AppError::TooManyRequests
    }
}
