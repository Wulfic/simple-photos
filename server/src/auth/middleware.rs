use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

use crate::error::AppError;
use crate::state::AppState;

use super::models::Claims;

pub struct AuthUser {
    pub user_id: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing authorization header".into()))?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::Unauthorized("Invalid authorization header format".into()))?;

        let key = DecodingKey::from_secret(state.config.auth.jwt_secret.as_bytes());
        // Strictly require HS256 — prevent algorithm confusion attacks
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_required_spec_claims(&["exp", "sub"]);

        let token_data = decode::<Claims>(token, &key, &validation)
            .map_err(|e| AppError::Unauthorized(format!("Invalid token: {}", e)))?;

        if token_data.claims.totp_required {
            return Err(AppError::Unauthorized(
                "TOTP verification required".into(),
            ));
        }

        Ok(AuthUser {
            user_id: token_data.claims.sub,
        })
    }
}
