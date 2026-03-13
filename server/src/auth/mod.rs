//! Authentication and authorization subsystem.
//!
//! Provides user registration, login (with optional TOTP 2FA), JWT
//! access/refresh token management, account lockout protection, and
//! password validation.
//!
//! ## Sub-modules
//!
//! - [`handlers`]    — Axum route handlers for all auth endpoints
//! - [`lockout`]     — Brute-force protection via account lockout
//! - [`middleware`]   — `AuthUser` extractor for protected routes
//! - [`models`]      — Request/response DTOs and JWT claims
//! - [`tokens`]      — JWT creation and refresh-token lifecycle
//! - [`totp`]        — TOTP and backup-code verification
//! - [`validation`]  — Username and password input validation

pub mod handlers;
pub mod lockout;
pub mod middleware;
pub mod models;
pub mod tokens;
pub mod totp;
pub mod validation;
