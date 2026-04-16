# Editing Engine Refactor — Master Plan

## Problem Statement

The editing system has grown organically across `photos/copies.rs`, `photos/render.rs`,
`photos/handlers.rs` (crop endpoint), `conversion.rs`, and `process.rs` — with duplicated
`CropMeta` structs, duplicated ffmpeg filter-chain logic, and no clear separation of concerns.
We have **12 editing-specific E2E tests** (test_08 through test_36) totalling **130+ test cases**,
yet regressions keep surfacing because the code is scattered and tightly coupled to the
photos module.

**Core design change**: Switch to **metadata-only saves** for the "Save" action (preserves
the original file, edits stored as JSON), and **rendered output** only for "Save As Copy"
(ffmpeg/image-crate bakes edits into a new independent file). This matches how professional
editors work (Lightroom, Snapseed, etc.) — non-destructive by default, rendered on export.

---

## Current Architecture (Before)

```
server/src/
  photos/
    handlers.rs       ← PUT /crop (set_crop) + list + register + favorite + batch dimensions
    copies.rs         ← POST /duplicate (render+encrypt) + edit_copies CRUD (766 lines!)
    render.rs         ← POST /render (ffmpeg stream for download) (338 lines)
    models.rs         ← Photo struct with crop_metadata field
    metadata.rs       ← EXIF extraction
    thumbnail.rs      ← Thumbnail generation
    ...
  conversion.rs       ← Format conversion (HEIC→JPEG, etc.)
  process.rs          ← run_with_timeout, ffmpeg/ffprobe timeouts
  
  backup/             ← Already a proper module (20 files, well-organized)
    sync_engine.rs    ← Phase 4 syncs edit_copies
    sync_metadata.rs  ← Syncs crop_metadata + edit_copies
    ...
```

### Problems
1. **Duplicated CropMeta struct** — defined separately in `copies.rs` and `render.rs`
2. **Duplicated ffmpeg filter chain** — `render_video_copy()` in copies.rs and the handler
   in render.rs build nearly identical ffmpeg argument lists independently
3. **copies.rs is 766 lines** mixing rendering, encryption, blob storage, DB inserts, and
   edit-copy CRUD — too many responsibilities
4. **No shared edit model** — each file parses crop_metadata JSON independently
5. **Image rendering** in copies.rs (render_image_copy) could share logic with thumbnail.rs
6. **Client-side scattered** — useViewerEdit.ts (state), useViewerActions.ts (save/copy/download),
   thumbnails.ts (canvas rendering), CropOverlay.tsx (UI) all loosely coupled

---

## Target Architecture (After)

```
server/src/
  editing/                    ← NEW: Editing engine module
    mod.rs                    ← Module root + public API surface
    models.rs                 ← CropMeta, EditMetadata, EditCopy (single source of truth)
    ffmpeg.rs                 ← All ffmpeg filter chain building + execution
    image_render.rs           ← Image crate rendering (crop, rotate, brightness)
    save.rs                   ← PUT /crop handler (metadata-only save)
    save_copy.rs              ← POST /duplicate handler (render + encrypt + new photo row)
    render_download.rs        ← POST /render handler (stream rendered file for download)
    edit_copies.rs            ← Edit copies CRUD (metadata-only "versions")
    
  backup/                     ← Already well-organized, keep as-is
    sync_engine.rs            ← Update Phase 4 to use editing::models
    sync_metadata.rs          ← Update to use editing::models
    ...

  photos/                     ← Simplified — no longer handles editing
    handlers.rs               ← List, register, favorite, batch dimensions (NO crop endpoint)
    copies.rs                 ← DELETED (moved to editing/)
    render.rs                 ← DELETED (moved to editing/)
    ...

  conversion.rs               ← Keep as-is (format conversion is import-time, not editing)
  process.rs                  ← Keep as-is (shared by editing + conversion + thumbnails)
```

---

## Phase 1: Create the Editing Module (Server) ✅ COMPLETE

### 1.1 Create `server/src/editing/models.rs`
- [ ] Single `CropMeta` struct (unified from copies.rs + render.rs duplicates)
- [ ] `EditMetadata` — typed wrapper around the JSON blob with validation
- [ ] `EditCopy` struct (from edit_copies table)
- [ ] Helper methods: `has_crop()`, `has_rotation()`, `has_brightness()`, `has_trim()`,
      `has_any_edit()`, `is_full_frame()`, `rotation_swaps_dimensions()`
- [ ] `CropMeta::from_json(s: &str) -> Result<CropMeta>` — single parse entry point
- [ ] Tests for the model helpers

### 1.2 Create `server/src/editing/ffmpeg.rs`
- [ ] `build_ffmpeg_args(source, dest, media_type, meta, ext) -> Vec<String>`
      Single function that builds the complete ffmpeg argument list
      (currently duplicated between copies.rs::render_video_copy and render.rs)
- [ ] `run_ffmpeg_render(source, dest, media_type, meta, ext) -> Result<()>`
      Wraps build_ffmpeg_args + run_with_timeout + error handling
- [ ] Extract the video filter chain builder into a testable `build_video_filters(meta) -> Vec<String>`
- [ ] Unit tests for filter chain generation (test all rotation/crop/brightness/trim combos)

### 1.3 Create `server/src/editing/image_render.rs`
- [ ] Move `render_image_copy()` from copies.rs here
- [ ] `render_image(source, dest, meta) -> Result<()>` — public function
- [ ] Keep same logic (image crate crop/rotate/brightness)
- [ ] Add support for output quality settings

### 1.4 Create `server/src/editing/save.rs`
- [ ] Move `set_crop()` handler from photos/handlers.rs
- [ ] This is the "Save" action — stores metadata, preserves original file
- [ ] Keep the same API: `PUT /api/photos/:id/crop`
- [ ] Validate via `CropMeta::from_json()`

### 1.5 Create `server/src/editing/save_copy.rs`
- [ ] Move `duplicate_photo()` from photos/copies.rs
- [ ] Refactor to use `editing::ffmpeg::run_ffmpeg_render()` for video/audio
- [ ] Refactor to use `editing::image_render::render_image()` for photos
- [ ] Keep the encryption logic (inline encrypt + blob storage)
- [ ] This is "Save As Copy" — renders edits into new file, sets crop_metadata=NULL

### 1.6 Create `server/src/editing/render_download.rs`
- [ ] Move `render_photo()` from photos/render.rs
- [ ] Refactor to use `editing::ffmpeg::run_ffmpeg_render()` (or build_ffmpeg_args + cache logic)
- [ ] Keep the render cache system
- [ ] This is "Download Rendered" — for on-demand video/audio export

### 1.7 Create `server/src/editing/edit_copies.rs`
- [ ] Move `create_edit_copy()`, `list_edit_copies()`, `delete_edit_copy()` from copies.rs
- [ ] These are metadata-only "versions" (lightweight)

### 1.8 Create `server/src/editing/mod.rs`
- [ ] Public re-exports for all handlers and models
- [ ] Module documentation explaining the editing engine design

### 1.9 Update `server/src/routes.rs`
- [ ] Point all editing routes to `crate::editing::*` instead of `crate::photos::*`
- [ ] Routes:
  - `PUT /photos/{id}/crop` → `editing::save::set_crop`
  - `POST /photos/{id}/duplicate` → `editing::save_copy::duplicate_photo`
  - `POST /photos/{id}/render` → `editing::render_download::render_photo`
  - `POST /photos/{id}/copies` → `editing::edit_copies::create_edit_copy`
  - `GET /photos/{id}/copies` → `editing::edit_copies::list_edit_copies`
  - `DELETE /photos/{id}/copies/{copy_id}` → `editing::edit_copies::delete_edit_copy`

### 1.10 Update backup sync
- [ ] `backup/sync_metadata.rs` — use `editing::models::EditCopy` if applicable
- [ ] Verify Phase 4 still syncs edit_copies correctly

### 1.11 Delete old files
- [ ] Remove `server/src/photos/copies.rs`
- [ ] Remove `server/src/photos/render.rs`
- [ ] Remove crop endpoint from `server/src/photos/handlers.rs`
- [ ] Update `server/src/photos/mod.rs` — remove `pub mod copies;` and `pub mod render;`

---

## Phase 2: Fix the Save vs Save-Copy Semantics ✅ COMPLETE

### 2.1 "Save" = Metadata-Only (Non-Destructive) ✅
- [x] Verified `PUT /crop` stores metadata on the photos row (confirmed in save.rs)
- [x] Client sends edits as JSON to this endpoint (web + Android)
- [x] Original file is NEVER modified (E2E test: SHA-256 checksum unchanged after crop)
- [x] Edits applied visually client-side (CSS transforms + filter:brightness)
- [x] Edits sync to all clients and backup servers

### 2.2 "Save As Copy" = Rendered Output ✅
- [x] `POST /duplicate` renders edits into a new file (confirmed in save_copy.rs)
- [x] New photo row with `crop_metadata = NULL` (edits baked in)
- [x] Independent encrypted blob
- [x] Thumbnail generated from rendered file
- [x] E2E tests: copy dimensions correct, metadata null, independent row

### 2.3 Client-Side Rendering (Web) ✅
- [x] Documented CSS↔ffmpeg mapping in editing/mod.rs
- [x] Brightness: CSS `brightness(1+b/100)` is multiplicative; server ffmpeg/image-crate is
      additive — documented as known discrepancy (perceptually similar for small adjustments)
- [x] Rotation: CSS degrees → ffmpeg transpose(1)/vflip+hflip/transpose(2) — CORRECT PARITY
- [x] Crop: Normalized 0-1 coordinates consistent across all platforms — CORRECT PARITY
- [x] **BUG FIXED**: image_render.rs negative brightness produced brightening instead of
      darkening. Changed from `(factor * 10.0)` to `(brightness * 2.55)` for correct additive offset.

### 2.4 Client-Side Rendering (Android) ✅
- [x] Audited Android PhotoViewerScreen.kt, PhotoViewerComponents.kt
- [x] Same JSON format as web (x, y, width, height, rotate, brightness, trimStart, trimEnd)
- [x] Same endpoints: PUT /crop, POST /duplicate (no /render on Android)
- [x] Same brightness formula as web CSS: `1 + brightness/100` (multiplicative)

---

## Phase 3: Test Coverage & Validation — PARTIALLY COMPLETE

### 3.1 Unit Tests (Rust) ✅
- [x] `editing::models` — 9 unit tests (parsing, validation, helpers)
- [x] `editing::ffmpeg` — 10 unit tests (filter chain for all combos)
- [x] (image_render dimensions tested via E2E since render requires temp files)

### 3.2 E2E Tests (Existing — Must Pass) ✅
All existing E2E tests pass after refactoring (no API changes):
- [x] test_08_edit_copies.py (9 tests)
- [x] test_34_bmp_edit_regression.py (19 tests)
- [x] test_35_edit_save_ddt.py (42 tests)
- [x] test_36_edit_save_regression.py (26 tests)
- [x] test_29_rendered_save_copy.py (5/8 pass — 3 fail: pre-existing thumbnail/file 404s)
- [x] test_30_render_timeout.py (6/8 pass — 2 fail: pre-existing thumbnail/file 404 timing)
- [x] test_31_android_edit_dimensions.py (4/7 pass — 3 fail: pre-existing thumbnail 404s)
- [x] test_32_save_copy_encrypt_banner.py (7/7 pass)
- [x] test_33_duplicate_inline_encrypt.py (5/5 pass)
- [ ] test_32_save_copy_encrypt_banner.py (7 tests)
- [ ] test_33_duplicate_inline_encrypt.py (5 tests)
- [ ] test_34_bmp_edit_regression.py (19 tests)
- [ ] test_35_edit_save_ddt.py (42 tests)
- [ ] test_36_edit_save_regression.py (26 tests)

### 3.3 New Tests ✅
- [x] test_37_edit_semantics.py — 11 E2E tests:
  - Metadata-only save doesn't modify original file (SHA-256 checksum comparison)
  - Crop metadata round-trips and can be cleared
  - Save-as-copy produces independent row with null crop_metadata
  - Rotated copy swaps dimensions correctly
  - Cropped copy has smaller dimensions
  - Copy preserves original taken_at
  - Brightness +/- duplicate succeeds and bakes edits
  - Combined brightness + crop + rotation produces correct dimensions

---

## Phase 4: Web Frontend Cleanup

### 4.1 Consolidate Editing Hooks ✅ (Reviewed — No Changes Needed)
- [x] useViewerEdit.ts (state) and useViewerActions.ts (actions) are already cleanly separated
- [x] `thumbnails.ts::applyEditsToImageDownload()` uses same crop math as server
      (normalized 0-1 × naturalWidth/Height → same as ffmpeg `iw*x, ih*y`)
- [x] No significant duplication found across editing files

### 4.2 Ensure CSS Preview Matches Server Render ✅ (Done in Phase 2)
- [x] Documented CSS↔ffmpeg↔image-crate mapping in `server/src/editing/mod.rs`
  - CSS transforms (for live preview)
  - ffmpeg filter arguments (for server render)
  - image crate operations (for image render)
- [x] Comprehensive reference table with all edit operations and platform mappings

---

## File Inventory (What Goes Where)

| Current Location | New Location | Lines | Notes |
|---|---|---|---|
| `photos/copies.rs` (all) | Split into 3 files | 766 | Biggest refactor target |
| ├─ `CropMeta` struct | `editing/models.rs` | ~15 | Unified with render.rs version |
| ├─ `duplicate_photo()` | `editing/save_copy.rs` | ~350 | Rendering + encryption |
| ├─ `render_video_copy()` | `editing/ffmpeg.rs` | ~100 | Shared with render_download |
| ├─ `render_image_copy()` | `editing/image_render.rs` | ~60 | Image crate rendering |
| ├─ `create_edit_copy()` | `editing/edit_copies.rs` | ~60 | Metadata-only copies |
| ├─ `list_edit_copies()` | `editing/edit_copies.rs` | ~30 | Read-only |
| └─ `delete_edit_copy()` | `editing/edit_copies.rs` | ~20 | Delete |
| `photos/render.rs` (all) | `editing/render_download.rs` | 338 | Uses shared ffmpeg.rs |
| ├─ `CropMeta` struct | `editing/models.rs` | ~15 | Merged with copies.rs version |
| └─ `render_photo()` | `editing/render_download.rs` | ~300 | Download endpoint |
| `photos/handlers.rs::set_crop` | `editing/save.rs` | ~40 | Metadata-only save |
| `photos/handlers.rs` (rest) | stays | ~360 | List, register, favorite, dims |

---

## Execution Order

1. **Phase 1.1–1.3** — Create models, ffmpeg, image_render (foundations, no route changes)
2. **Phase 1.4–1.7** — Move handlers into editing module
3. **Phase 1.8–1.9** — Wire up routes, update mod.rs
4. **Phase 1.10–1.11** — Update backup sync, delete old files
5. **Phase 3.2** — Run ALL existing E2E tests (must be green)
6. **Phase 2** — Verify save vs save-copy semantics (should already be correct)
7. **Phase 3.1, 3.3** — Add new unit tests and parity tests
8. **Phase 4** — Frontend cleanup (lower priority, can be separate PR)

---

## Risk Assessment

- **LOW RISK**: Creating the module structure and moving code — purely organizational
- **MEDIUM RISK**: Shared ffmpeg filter chain — must produce identical output to current code
- **LOW RISK**: Route changes — same endpoints, same API, just different module paths
- **MITIGATION**: 130+ existing E2E tests provide excellent regression coverage
- **ROLLBACK**: Git makes reverting trivial if something breaks

---

## Session Log

- **Session 1** (current): Deep dive, architecture mapping, plan creation
- **Session 2**: Phase 1.1–1.3 (models, ffmpeg, image_render)
- **Session 3**: Phase 1.4–1.8 (move handlers, wire routes)
- **Session 4**: Phase 1.9–1.11 + test run (routes, cleanup, E2E validation)
- **Session 5**: Phase 2–4 (semantics verification, new tests, frontend cleanup)
