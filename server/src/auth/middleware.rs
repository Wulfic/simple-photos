//! Axum extractor that validates a Bearer JWT and yields the authenticated
//! user's ID. Use `AuthUser` as a handler parameter on any protected route.
//!
//! Supports two token sources (checked in order):
//! 1. `Authorization: Bearer <token>` header (standard API usage)
//! 2. `?token=<jwt>` query parameter (used by `<video>`/`<audio>` elements
//!    that cannot set custom headers but need Range-request support)

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

use crate::error::AppError;
use crate::state::AppState;

use super::models::Claims;

/// Authenticated user extracted from a valid Bearer JWT.
///
/// Usage: add `auth: AuthUser` to any handler signature to require
/// authentication. The extractor rejects TOTP session tokens (half-
/// authenticated) — only fully-authenticated JWTs are accepted.
///
/// Token resolution order:
/// 1. `Authorization: Bearer <token>` header
/// 2. `?token=<jwt>` query parameter (for media elements that cannot set headers)
pub struct AuthUser {
    pub user_id: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // 1. Try Authorization header first (standard path)
        let token = if let Some(auth_header) = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
        {
            auth_header
                .strip_prefix("Bearer ")
                .ok_or_else(|| {
                    AppError::Unauthorized("Invalid authorization header format".into())
                })?
                .to_string()
        }
        // 2. Fall back to ?token= query parameter (for <video>/<audio> src URLs)
        else if let Some(query) = parts.uri.query() {
            extract_token_param(query).ok_or_else(|| {
                AppError::Unauthorized("Missing authorization header or token parameter".into())
            })?
        } else {
            return Err(AppError::Unauthorized(
                "Missing authorization header".into(),
            ));
        };

        let key = DecodingKey::from_secret(state.config.auth.jwt_secret.as_bytes());
        // Strictly require HS256 — prevent algorithm confusion attacks
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_required_spec_claims(&["exp", "sub"]);

        let token_data = decode::<Claims>(&token, &key, &validation)
            .map_err(|e| AppError::Unauthorized(format!("Invalid token: {}", e)))?;

        // Reject TOTP session tokens — they represent a half-authenticated
        // state (password OK, 2FA pending) and must not access protected routes.
        if token_data.claims.totp_required {
            return Err(AppError::Unauthorized("TOTP verification required".into()));
        }

        Ok(AuthUser {
            user_id: token_data.claims.sub,
        })
    }
}

/// Extract the `token` value from a query string without pulling in a full
/// URL-parsing crate.  Handles `token=<jwt>` appearing anywhere in the query.
fn extract_token_param(query: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("token=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
