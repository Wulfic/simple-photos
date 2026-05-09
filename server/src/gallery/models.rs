//! Re-exports of model types used by the gallery sub-modules.
//!
//! The canonical definitions live in `crate::photos::models` and
//! `crate::sharing::models`; this module simply surfaces the gallery-relevant
//! subset so callers can `use crate::gallery::models::*` when convenient.

// ── Secure gallery models (from photos) ──────────────────────────────────
#[allow(unused_imports)]
pub use crate::photos::models::{
    CreateSecureGalleryRequest, SecureGalleryListResponse, SecureGalleryRecord,
    SecureGalleryUnlockResponse, UnlockSecureGalleryRequest,
};

// ── Shared album models (from sharing) ───────────────────────────────────
#[allow(unused_imports)]
pub use crate::sharing::models::{
    AddMemberRequest, AddPhotoRequest, CreateSharedAlbumRequest, SharedAlbumInfo,
    SharedAlbumMember, SharedAlbumPhoto,
};
