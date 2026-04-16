//! Editing engine — non-destructive photo and video editing.
//!
//! This module centralises all editing functionality that was previously
//! scattered across `photos/copies.rs`, `photos/render.rs`, and the crop
//! endpoint in `photos/handlers.rs`.
//!
//! ## Design
//!
//! - **Save** ([`save::set_crop`]) — Metadata-only.  Stores edit parameters
//!   as JSON in `photos.crop_metadata`.  The original file is never modified.
//!   Clients apply edits visually via CSS transforms (web) or Compose
//!   transforms (Android).
//!
//! - **Save As Copy** ([`save_copy::duplicate_photo`]) — Rendered output.
//!   Creates a fully independent file with edits baked in via **ffmpeg**
//!   (video/audio) or the **image crate** (photos).  The new `photos` row
//!   has `crop_metadata = NULL` and its own encrypted blob.
//!
//! - **Download Rendered** ([`render_download::render_photo`]) — On-demand
//!   server-side render for video/audio.  Streams the result back without
//!   creating a permanent copy.  Cached in `.renders/` for repeat downloads.
//!
//! - **Edit Copies** ([`edit_copies`]) — Lightweight metadata-only "versions"
//!   stored in the `edit_copies` table.  No file duplication.
//!
//! ## Shared Components
//!
//! - [`models::CropMeta`] — Single source of truth for edit metadata parsing.
//! - [`ffmpeg`] — All ffmpeg filter chain construction and execution.
//! - [`image_render`] — Image crate rendering (crop, rotate, brightness).
//!
//! ## Client ↔ Server Parity Reference
//!
//! The `crop_metadata` JSON has three consumers that must produce visually
//! consistent results:
//!
//! | Field | Web/Android Preview | Server (ffmpeg — video) | Server (image crate — photo) |
//! |---|---|---|---|
//! | `x,y,width,height` | CSS `translate + scale` / Compose `graphicsLayer` | `crop=iw*W:ih*H:iw*X:ih*Y` | `crop_imm(X*iw, Y*ih, W*iw, H*ih)` |
//! | `rotate` (0/90/180/270) | CSS `rotate(Ndeg)` / Compose `rotationZ` | `transpose=1` (90), `vflip,hflip` (180), `transpose=2` (270) | `rotate90()` / `rotate180()` / `rotate270()` |
//! | `brightness` (-100…+100) | **Multiplicative**: CSS `brightness(1+b/100)` | **Additive**: `eq=brightness=b/100` | **Additive**: `brighten(b*2.55)` |
//! | `trimStart/trimEnd` (secs) | ExoPlayer seek / `<video>` currentTime | `-ss T -to T` | N/A (images) |
//!
//! ### Brightness Discrepancy
//!
//! CSS `brightness(1.5)` **multiplies** each pixel by 1.5 — a dark pixel (50)
//! becomes 75.  FFmpeg `eq=brightness=0.5` and `image::brighten(128)` **add**
//! a fixed offset — a dark pixel (50) becomes 178.  The two diverge most for
//! dark content at high brightness; for typical edits (±30) the difference is
//! usually imperceptible.  Closing this gap would require a per-pixel multiply
//! pass on the server, which is a potential future enhancement.

pub mod edit_copies;
pub mod ffmpeg;
pub mod image_render;
pub mod models;
pub mod render_download;
pub mod save;
pub mod save_copy;
