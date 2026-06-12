//! Secure-gallery unlock tokens.
//!
//! When a user unlocks their secure galleries by re-entering their account
//! password, the server issues a short-lived token. This module is the single
//! source of truth for **generating and verifying** that token.
//!
//! ## Why this exists
//!
//! The original implementation generated a token but the read path
//! (`list_gallery_items`) accepted *any* non-empty `X-Gallery-Token` string —
//! the signature and expiry were never checked. That made the password
//! re-prompt cosmetic. This module restores a real, verifiable gate:
//!
//! * The token is bound to the user ID and an issue timestamp.
//! * It is authenticated with a keyed SHA-256 tag over the JWT secret, so it
//!   cannot be forged without server-side secrets.
//! * It expires after [`TOKEN_TTL_SECS`].
//! * Verification uses a constant-time comparison to avoid leaking the
//!   expected tag through timing.
//!
//! Wire format (unchanged, so existing clients keep working):
//! `sec_<issued_at_unix>_<hex(sha256("secure:<user_id>:<issued_at>" || jwt_secret))>`
//!
//! ## Scope / known limitation
//!
//! This gate currently protects the gallery **listing** endpoint. The raw
//! bytes of secure items are still served by the generic
//! `/api/photos/{id}/file` and `/api/blobs/{id}` endpoints, which authenticate
//! the account session but do not yet require this token. Closing that path
//! needs coordinated web + Android changes (every secure-item media URL must
//! carry the token) and is tracked as a follow-up. Keeping generation and
//! verification here means that follow-up only has to call [`verify`].

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// How long an unlock token stays valid, in seconds (1 hour).
pub const TOKEN_TTL_SECS: u64 = 3600;

/// Compute the keyed authentication tag for `(user_id, issued_at)`.
///
/// Secret-suffix construction: `sha256(message || key)`. SHA-256 with the
/// secret as a suffix is not affected by the length-extension weakness that
/// makes the secret-*prefix* construction unsafe, which is sufficient here.
fn tag(user_id: &str, issued_at: i64, jwt_secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("secure:{user_id}:{issued_at}").as_bytes());
    hasher.update(jwt_secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate a fresh unlock token for `user_id`.
///
/// Returns `(token, expires_in_secs)`.
pub fn generate(user_id: &str, jwt_secret: &str) -> (String, u64) {
    let issued_at = chrono::Utc::now().timestamp();
    let token = format!("sec_{}_{}", issued_at, tag(user_id, issued_at, jwt_secret));
    (token, TOKEN_TTL_SECS)
}

/// Verify an unlock `token` for `user_id`.
///
/// Returns `true` only when the token is well-formed, unexpired, and carries a
/// valid signature for this user. All comparisons are constant-time.
pub fn verify(token: &str, user_id: &str, jwt_secret: &str) -> bool {
    // Expected shape: sec_<issued_at>_<hex tag>. The tag is hex (no '_') and
    // issued_at is decimal, so a 3-way split on '_' is unambiguous.
    let mut parts = token.splitn(3, '_');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("sec"), Some(issued_str), Some(provided_tag)) => {
            let issued_at: i64 = match issued_str.parse() {
                Ok(v) => v,
                Err(_) => return false,
            };

            // Reject expired tokens (and tokens issued absurdly in the future,
            // allowing a small clock-skew grace window).
            let now = chrono::Utc::now().timestamp();
            if issued_at > now + 60 {
                return false;
            }
            if now.saturating_sub(issued_at) > TOKEN_TTL_SECS as i64 {
                return false;
            }

            let expected = tag(user_id, issued_at, jwt_secret);
            // Constant-time comparison — avoids leaking the expected tag via
            // early-exit timing on `==`.
            expected.as_bytes().ct_eq(provided_tag.as_bytes()).into()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-jwt-secret-at-least-32-chars-long-xx";

    #[test]
    fn generate_then_verify_roundtrips() {
        let (token, ttl) = generate("user-123", SECRET);
        assert_eq!(ttl, TOKEN_TTL_SECS);
        assert!(verify(&token, "user-123", SECRET));
    }

    #[test]
    fn wrong_user_is_rejected() {
        let (token, _) = generate("user-123", SECRET);
        assert!(!verify(&token, "user-456", SECRET));
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let (token, _) = generate("user-123", SECRET);
        assert!(!verify(
            &token,
            "user-123",
            "a-different-secret-value-32chars-xx"
        ));
    }

    #[test]
    fn empty_or_garbage_is_rejected() {
        // This is the exact bypass the old code allowed: any non-empty string.
        assert!(!verify("", "user-123", SECRET));
        assert!(!verify("x", "user-123", SECRET));
        assert!(!verify("sec_not_a_number_tag", "user-123", SECRET));
        assert!(!verify("totally-made-up", "user-123", SECRET));
    }

    #[test]
    fn expired_token_is_rejected() {
        // Forge a correctly-signed token with an issue time well past the TTL.
        let issued_at = chrono::Utc::now().timestamp() - (TOKEN_TTL_SECS as i64) - 120;
        let token = format!("sec_{}_{}", issued_at, tag("user-123", issued_at, SECRET));
        assert!(!verify(&token, "user-123", SECRET));
    }

    #[test]
    fn future_dated_token_is_rejected() {
        let issued_at = chrono::Utc::now().timestamp() + 3600;
        let token = format!("sec_{}_{}", issued_at, tag("user-123", issued_at, SECRET));
        assert!(!verify(&token, "user-123", SECRET));
    }

    #[test]
    fn tampered_tag_is_rejected() {
        let (token, _) = generate("user-123", SECRET);
        let mut chars: Vec<char> = token.chars().collect();
        // Flip the final hex character of the tag.
        let last = chars.len() - 1;
        chars[last] = if chars[last] == '0' { '1' } else { '0' };
        let tampered: String = chars.into_iter().collect();
        assert!(!verify(&tampered, "user-123", SECRET));
    }
}
