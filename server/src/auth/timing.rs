//! Constant-time bcrypt helper for login paths.
//!
//! When a username is not found, we still need to run a bcrypt verification
//! so the failure path takes roughly the same wall-clock time as the success
//! path (defends against username-enumeration timing attacks).
//!
//! Historically this module hard-coded a published dummy bcrypt digest, which
//! caused secret-scanning tools (Gitleaks, Semgrep) to flag the literal even
//! though it was not a real credential. We now generate the dummy hash at
//! runtime, cached in a `OnceLock`, so no bcrypt-shaped string appears in
//! the source tree.

use std::sync::OnceLock;

static DUMMY_HASH: OnceLock<String> = OnceLock::new();

/// Lazily produce a bcrypt hash of a fixed random byte string. Cached for the
/// lifetime of the process. Cost matches the application default so the
/// timing of `verify` against this hash mirrors the timing of `verify`
/// against a real user's stored hash.
fn dummy_hash() -> &'static str {
    DUMMY_HASH
        .get_or_init(|| {
            // Hash a fixed, non-secret throwaway value. The resulting digest
            // is generated at startup and only used to balance login timing.
            bcrypt::hash("dummy-bcrypt-target", bcrypt::DEFAULT_COST)
                .expect("bcrypt hash of constant input must succeed")
        })
        .as_str()
}

/// Run `bcrypt::verify` against a dummy hash to equalize timing on the
/// failure path (e.g. when the username is unknown). The boolean result is
/// intentionally discarded.
pub fn equalize_login_timing(password: &str) {
    let _ = bcrypt::verify(password, dummy_hash());
}
