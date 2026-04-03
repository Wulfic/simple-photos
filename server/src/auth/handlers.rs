//! Axum handlers for authentication endpoints.
//!
//! Covers the full auth lifecycle: registration, login (with optional TOTP),
//! token refresh (with rotation + theft detection), logout, 2FA management,
//! and password changes.
//!
//! Handler implementations are split across focused sub-modules:
//! - [`handlers_login`]    — register, login, login_totp, refresh, logout
//! - [`handlers_2fa`]      — get_2fa_status, setup_2fa, confirm_2fa, disable_2fa
//! - [`handlers_password`] — change_password, verify_password

// Re-export everything so callers see `auth::handlers::login` etc.
pub use super::handlers_login::*;
pub use super::handlers_2fa::*;
pub use super::handlers_password::*;
