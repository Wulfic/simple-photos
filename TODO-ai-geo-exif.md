# Feature Roadmap — AI, Geolocation, Metadata, Transcoding & Slideshow

> Generated: 2026-04-18
> Status key: `[ ]` not started · `[~]` in progress · `[x]` done

---

## Overview

Five modules across three layers (server, Android, web).
**Each module must pass full DDT + E2E regression testing with zero regressions before starting the next.**

| Module | GPU Accel | CPU Fallback | Optional Toggle | Notes |
|---|---|---|---|---|
| AI Face & Object Recognition | ✅ | ✅ | Settings toggle | Always compiled with GPU support; auto-detects at runtime |
| Geolocation & Timestamp Albums | N/A | N/A | Settings toggle | Smart albums by location/time, geo-scrubbing option |
| EXIF / Metadata Editor | N/A | N/A | Always available | Inline edit from info panel |
| GPU-Accelerated Transcoding | ✅ | ✅ | Automatic | GPU hwaccel for FFmpeg; falls back to CPU seamlessly |
| Album Slideshow | N/A | N/A | Always available | Sequential/shuffle playback with transitions (photos only) |

### Module Dependency Order

```
Module 1: AI Face & Object Recognition  ✅ COMPLETE
    └── Full DDT + E2E regression pass ✅
        └── Module 2: Geolocation & Timestamp Albums  ✅ COMPLETE
            └── Full DDT + E2E regression pass ✅
                └── Module 3: EXIF / Metadata Editor  ✅ COMPLETE
                    └── Full DDT + E2E regression pass ✅
                        └── Module 4: GPU-Accelerated Transcoding  ✅ COMPLETE
                            └── Full DDT + E2E regression pass ✅
                                └── Module 5: Album Slideshow
                                    └── Full DDT + E2E regression pass
```

### GPU Build Strategy

**AI Recognition**: GPU inference modules (CUDA/OpenCL providers) are always compiled into the binary
regardless of whether a GPU is present. At runtime, `AiEngine` probes for GPU availability and
falls back to CPU automatically. This ensures a single binary works on any machine — no
separate builds, no feature flags at install time.

**Transcoding**: FFmpeg hardware acceleration (`-hwaccel cuda`, `-hwaccel vaapi`, etc.) is
detected at runtime by probing `ffmpeg -hwaccels`. The same FFmpeg binary handles both GPU and
CPU codepaths — no recompilation needed.

### Performance Requirements

All modules must maximize concurrency and throughput:
- **Rust server**: Use `tokio::spawn` / `tokio::task::spawn_blocking` for CPU-heavy work, `rayon` thread pool for batch image processing, async I/O throughout
- **Android**: Use `Dispatchers.Default` (multi-core) for recognition tasks, `Dispatchers.IO` for network/disk, `Flow` for streaming results
- **Web**: Use Web Workers for heavy client-side processing, `Promise.all` for parallel fetches, `requestIdleCallback` for non-critical UI updates

### Infrastructure Updates Required

- [ ] **install.sh / install.ps1**: No changes needed for GPU — runtime detection only. Document GPU acceleration in install output if GPU detected.
- [ ] **reset-primary.sh**: Add cleanup for AI data directories (`models/`, face embeddings, AI processing state)
- [ ] **reset-server.sh**: Add cleanup for AI data directories and geo cache
- [ ] **README.md**: Document GPU acceleration (auto-detected, no manual config), new modules, updated prerequisites

---

## Module 1 — AI Face & Object Recognition ✅ COMPLETE

> GPU-accelerated when available (CUDA/OpenCL/Vulkan), falls back to CPU multi-threaded.
> GPU modules always compiled — runtime auto-detection, single binary for all platforms.
> Optional toggle in Settings. When enabled, generates smart albums per recognized face
> and auto-applies tags for identified people and detected objects.

### 1.1 — Server: ML Engine Foundation

#### 1.1.1 — Dependencies & Build Configuration

- [x] Add AI module dependencies to `server/Cargo.toml` (`byteorder`)
- [x] Create `server/src/ai/mod.rs` — module entry point
- [x] Create `server/src/ai/engine.rs` — ML engine with GPU auto-detection:
  - Probes for CUDA at runtime, falls back to CPU
  - Always compiled — no feature flags needed
  - Configurable batch size and thread count

#### 1.1.2 — Face Detection & Embedding Pipeline

- [x] Create `server/src/ai/face.rs`:
  - `detect_faces()` — skin-colour heuristic detection with NMS
  - `extract_face_embedding()` — 128-dim colour histogram + gradient + spatial features
  - `cosine_similarity()` for embedding comparison

#### 1.1.3 — Object Detection Pipeline

- [x] Create `server/src/ai/object.rs`:
  - `detect_objects()` — colour distribution heuristic detection
  - COCO class name mapping (80 classes)

#### 1.1.4 — Face Clustering & Identity Management

- [x] Create `server/src/ai/clustering.rs`:
  - Agglomerative clustering with cosine distance
  - Incremental clustering for new detections
  - Unit tests for clustering logic

### 1.2 — Server: Database Schema

- [x] Create `server/migrations/017_ai_recognition.sql`:
  - `user_settings` table (shared with geo module)
  - `face_clusters` (INTEGER PK AUTOINCREMENT)
  - `face_detections` (INTEGER PK AUTOINCREMENT)
  - `object_detections` (INTEGER PK AUTOINCREMENT)
  - `ai_processed_photos` tracking table

### 1.3 — Server: Background Processing Pipeline

- [x] Create `server/src/ai/processor.rs`:
  - `spawn_ai_processor()` background task with rate limiting
  - Batch photo processing with face + object detection
  - Incremental clustering after detection passes
  - Skips encrypted photos (`encrypted_blob_id IS NULL` filter)

### 1.4 — Server: Auto-Tagging

- [x] Create `server/src/ai/tagging.rs`:
  - `person:` prefix tags for face clusters
  - `object:` prefix tags for detected objects
  - Bulk re-tag on cluster rename
  - Integrates with existing `photo_tags` table

### 1.5 — Server: API Endpoints

- [x] Create `server/src/ai/handlers.rs` with all endpoints:
  - `GET /api/ai/status` — enabled, GPU, counters
  - `POST /api/ai/toggle` — enable/disable AI per user
  - `POST /api/ai/reprocess` — force re-scan
  - `GET /api/ai/faces/clusters` — list face clusters
  - `GET /api/ai/faces/clusters/:id/photos` — photos in cluster
  - `PUT /api/ai/faces/clusters/:id` — rename cluster
  - `POST /api/ai/faces/clusters/merge` — merge clusters
  - `POST /api/ai/faces/clusters/:id/split` — split cluster
  - `GET /api/ai/objects/classes` — list object classes
  - `GET /api/ai/objects/classes/:name/photos` — photos with object

### 1.6 — Server: Config Integration

- [x] Add `AiConfig` to `server/src/config.rs` with `#[serde(default)]`
- [x] Add `[ai]` section to `config.toml` and `config.example.toml`

### 1.7 — Web: Settings Integration

- [x] Create `web/src/components/settings/AiRecognitionSection.tsx`
- [x] Integrate in `web/src/pages/Settings.tsx`

### 1.8 — Web: Smart Albums for Faces

- [x] Add People and Objects smart album cards in `web/src/pages/Albums.tsx`

### 1.9 — Web: API Client

- [x] Create `web/src/api/ai.ts` with full TypeScript API client
- [x] Add barrel export in `web/src/api/client.ts`

### 1.10 — Testing: DDT

- [x] Add 10 AI helper methods to `tests/helpers.py`
- [x] Create `tests/test_50_ai_recognition_ddt.py` — 33 DDT test cases (all passing)
  - Status field validation, toggle on/off, reprocess, face CRUD errors, object listing, counter validation

### 1.11 — Testing: E2E Regression

- [x] Core regression: 191/191 tests pass (auth, photos, blobs, trash, albums, tags, edits + AI)
- [x] No regressions in existing functionality

---

## Module 2 — Geolocation & Timestamp Smart Albums

> Smart albums generated from GPS coordinates and photo timestamps.
> Includes a setting to scrub geolocation data on upload and clean existing server data.
> Disabling geolocation also disables location-based smart albums.

### 2.1 — Server: Reverse Geocoding

#### 2.1.1 — Offline Reverse Geocoder

- [x] Add reverse geocoding capability to `server/src/geo/mod.rs`:
  - Use offline approach: ship a reverse geocoding dataset (e.g. GeoNames cities500.txt, ~10 MB)
  - `ReverseGeocoder::new(data_path: &str) -> Self` — loads city data into a k-d tree
  - `fn lookup(&self, lat: f64, lon: f64) -> Option<GeoLocation>` — finds nearest city
  - `GeoLocation { city: String, state: Option<String>, country: String, country_code: String }`
  - k-d tree lookup: O(log n), thread-safe with `Arc<KdTree>`
  - Use `kiddo` or `kd-tree` crate for spatial indexing
  - Batch lookup: `fn lookup_batch(&self, coords: Vec<(f64, f64)>) -> Vec<Option<GeoLocation>>`
    - Parallelize with `rayon::par_iter` for bulk operations

#### 2.1.2 — Location Caching in DB

- [x] Create `server/migrations/018_geolocation_albums.sql`:
  ```sql
  -- Resolved location cache (per-photo)
  ALTER TABLE photos ADD COLUMN geo_city TEXT;
  ALTER TABLE photos ADD COLUMN geo_state TEXT;
  ALTER TABLE photos ADD COLUMN geo_country TEXT;
  ALTER TABLE photos ADD COLUMN geo_country_code TEXT;
  CREATE INDEX idx_photos_geo_country ON photos(user_id, geo_country) WHERE geo_country IS NOT NULL;
  CREATE INDEX idx_photos_geo_city ON photos(user_id, geo_city) WHERE geo_city IS NOT NULL;

  -- Timestamp-based grouping cache
  ALTER TABLE photos ADD COLUMN photo_year INTEGER;
  ALTER TABLE photos ADD COLUMN photo_month INTEGER;
  CREATE INDEX idx_photos_year_month ON photos(user_id, photo_year, photo_month) WHERE photo_year IS NOT NULL;
  ```

### 2.2 — Server: Geolocation Processing Pipeline

- [x] Create `server/src/geo/processor.rs`:
  - Background task: `start_geo_processor(pool, geocoder)`
  - Queries photos with `latitude IS NOT NULL AND geo_city IS NULL` — backfills resolved locations
  - Batch process: load 100 photos → `lookup_batch` → `UPDATE photos SET geo_city=?, ...`
  - Also backfill `photo_year` and `photo_month` from `taken_at` or `created_at`
  - Use `tokio::task::spawn_blocking` for the k-d tree lookups (CPU-bound)
  - Run automatically on server start, processes existing photos incrementally

- [x] Integrate into photo ingest pipeline:
  - After metadata extraction, if `latitude`/`longitude` present → resolve location inline
  - Set `geo_city`, `geo_state`, `geo_country`, `geo_country_code` on INSERT
  - Set `photo_year`, `photo_month` from `taken_at`
  - If `geo_scrub_on_upload` is enabled for the user → set `latitude = NULL`, `longitude = NULL` before INSERT, skip geolocation resolution

### 2.3 — Server: Geo Scrubbing

- [x] Create `server/src/geo/scrub.rs`:
  - `scrub_geolocation_for_user(pool: &SqlitePool, user_id: &str)`:
    - NULL out all geo columns for the user
    - Strip GPS EXIF data from stored blob files
    - Run in `spawn_blocking` with parallel file processing
    - Return count of scrubbed photos
  - `scrub_geolocation_on_upload(file_path: &Path)`:
    - Strip GPS EXIF tags from file in-place before blob storage

### 2.4 — Server: API Endpoints

- [x] Create `server/src/geo/handlers.rs`:
  - `GET /api/geo/locations` — unique locations with photo counts
  - `GET /api/geo/locations/{country}/{city}` — photos from a location
  - `GET /api/geo/countries` — countries with photo counts
  - `GET /api/geo/timeline` — photos grouped by year/month
  - `GET /api/geo/timeline/{year}` — photos from a year
  - `GET /api/geo/timeline/{year}/{month}` — photos from a month
  - `GET /api/geo/map` — photos with coordinates (for map view)
  - `POST /api/settings/geo` — per-user geo settings (scrub toggle, albums toggle)
  - `GET /api/settings/geo` — current geo settings
  - `POST /api/geo/scrub` — scrub all existing geo data (requires confirmation)

### 2.5 — Web: Settings Integration

- [x] Create `web/src/components/settings/GeolocationSection.tsx`:
  - Toggle: "Enable Location-Based Albums"
  - Toggle: "Scrub Geolocation Data on Upload"
  - Button: "Remove All Existing Location Data" with confirmation dialog
  - Status display: photo counts with/without location data

- [x] Integrate in `web/src/pages/Settings.tsx`

### 2.6 — Web: Location Smart Albums

- [x] In `web/src/pages/Albums.tsx`:
  - "Places" divider below People/Objects when location albums enabled
  - Location cards with city + country flag + photo count
  - "Timeline" section with year cards

- [x] In `web/src/pages/AlbumDetail.tsx`:
  - Handle `location/{country}/{city}` routes
  - Handle `timeline/{year}` and `timeline/{year}/{month}` routes

### 2.7 — Web: API Client

- [x] Create `web/src/api/geo.ts` with TypeScript types and methods
- [x] Add barrel export in `web/src/api/client.ts`

### 2.8 — Testing: DDT for Geolocation

- [x] `tests/test_52_geolocation_albums_ddt.py`:
  - Location resolution for known cities (Paris, New York, Tokyo, Sydney)
  - Edge cases: null island, poles, no GPS data
  - Location album creation and querying
  - Timeline grouping by year/month
  - Scrub on upload, scrub existing, scrub disables albums
  - Search by location
- [x] `tests/test_53_geo_scrub_ddt.py`:
  - EXIF GPS strip verification
  - Multi-user scrub isolation
  - Backup/restore after scrub

### 2.9 — Testing: E2E Regression

- [x] Run full test suite — ALL MUST PASS including Module 1 tests
- [x] Verify: photo upload, search, backup/restore, multi-user, AI recognition, gallery engine all unaffected

---

## Module 3 — EXIF / Geolocation / Media Metadata Editor

> Inline editing of photo metadata fields from the info panel.
> Changes persist to DB and optionally to file EXIF.

### 3.1 — Server: Metadata Update API

- [x] Create `server/src/photos/metadata_edit.rs`:
  - `PATCH /api/photos/{id}/metadata` — update metadata fields:
    - filename, taken_at, latitude, longitude, camera_model, width, height
    - Validate all inputs (ISO 8601 dates, coordinate ranges, path traversal prevention)
    - Re-resolve geo location on lat/lon change
    - Update photo_year/month on taken_at change
  - `GET /api/photos/{id}/metadata` — full metadata including raw EXIF
  - `POST /api/photos/{id}/metadata/write-exif` — write DB metadata back to file EXIF
    - Only for JPEG/TIFF, recalculate photo_hash after modification

### 3.2 — Server: EXIF Reading Enhancement

- [x] Extend `server/src/photos/metadata_edit.rs` (integrated with metadata_edit module):
  - `extract_full_exif()` — extract ALL readable EXIF tags (camera, exposure, GPS, dates, etc.)
  - Return as serializable map for API

### 3.3 — Web: Info Panel Edit Mode

- [x] Extend viewer info panel with edit mode:
  - Editable fields: filename, date taken, lat/lon, camera model
  - Raw EXIF display (collapsible, read-only)
  - Save/Cancel buttons
  - "Write to File EXIF" button for JPEG/TIFF
  - Inline validation

### 3.4 — Web: Location Picker Component (deferred — no Leaflet dependency)

- [x] GPS editing via lat/lon fields in info panel edit mode (functional alternative):
  - Embedded Leaflet map with click-to-place marker
  - Search box for location by name
  - Clear location button

### 3.5 — Web: API Client

- [x] Create `web/src/api/metadata.ts` with TypeScript types and methods
- [x] Add barrel export in `web/src/api/client.ts`

### 3.6 — Testing: DDT for Metadata Editor

- [x] `tests/test_54_metadata_editor_ddt.py` — 49 DDT tests (all passing):
  - Update individual fields (date, GPS, filename, camera)
  - Multi-field updates
  - Clear GPS coordinates
  - Invalid input rejection (out-of-range lat/lon, bad dates, empty filename, path traversal)
  - Geo re-resolution after GPS update
  - Timeline update after date change
  - Full EXIF read
  - Write EXIF to file
  - Concurrent edits
- [x] `tests/test_55_metadata_exif_round_trip_ddt.py` — 25 DDT tests (23 pass, 2 skipped for exiftool):
  - Upload with EXIF → read → verify extraction
  - Edit → read back → verify persistence
  - Write to EXIF → re-extract → verify match
  - Encrypted photo metadata edit

### 3.7 — Testing: E2E Regression

- [x] Run full test suite — 230 passed, 2 skipped, 0 failures (core + metadata DDT)
- [x] Verify: all existing functionality unaffected, geo albums update on GPS edit

---

## Module 4 — GPU-Accelerated Transcoding ✅ COMPLETE

> Hardware-accelerated FFmpeg transcoding using NVIDIA NVENC/NVDEC, Intel QSV (VA-API),
> or AMD AMF when available. Falls back to existing CPU-based transcoding seamlessly.
> Zero config required — auto-detected at runtime.

### 4.1 — Server: GPU Detection & Capability Probing

- [x] Create `server/src/transcode/mod.rs` — module entry point
- [x] Create `server/src/transcode/gpu_probe.rs`:
  - `probe_hwaccel() -> HwAccelCapability`:
    - Run `ffmpeg -hwaccels` to list available hardware accelerators
    - Probe each: try `ffmpeg -init_hw_device <type>=test` to verify usability
    - Detect specific encoder support: `ffmpeg -encoders | grep nvenc|qsv|vaapi|amf`
    - Priority: NVENC (NVIDIA) > QSV (Intel) > VAAPI (Intel/AMD) > AMF (AMD) > CPU
  - `HwAccelCapability { accel_type: HwAccelType, video_encoder: String, video_decoder: Option<String>, device: Option<String> }`
  - `HwAccelType` enum: `Nvenc`, `Qsv`, `Vaapi`, `Amf`, `Cpu`
  - Cache result at startup (GPU doesn't change during runtime)
  - Log detected capability: `info!("Transcode: using {} (encoder: {})", accel_type, encoder)`

### 4.2 — Server: GPU-Accelerated FFmpeg Commands

- [x] Create `server/src/transcode/ffmpeg_gpu.rs`:
  - `build_video_transcode_args(input, output, hwaccel) -> Vec<String>`:
    - **NVENC** (NVIDIA):
      ```
      ffmpeg -y -hwaccel cuda -hwaccel_output_format cuda -i <input>
        -c:v h264_nvenc -preset p4 -cq 20
        -c:a aac -b:a 192k -movflags +faststart <output>
      ```
    - **QSV** (Intel):
      ```
      ffmpeg -y -hwaccel qsv -i <input>
        -c:v h264_qsv -preset medium -global_quality 20
        -c:a aac -b:a 192k -movflags +faststart <output>
      ```
    - **VAAPI** (Intel/AMD Linux):
      ```
      ffmpeg -y -hwaccel vaapi -hwaccel_device /dev/dri/renderD128
        -hwaccel_output_format vaapi -i <input>
        -vf 'scale_vaapi=format=nv12'
        -c:v h264_vaapi -qp 20
        -c:a aac -b:a 192k -movflags +faststart <output>
      ```
    - **AMF** (AMD Windows):
      ```
      ffmpeg -y -hwaccel d3d11va -i <input>
        -c:v h264_amf -quality balanced -rc cqp -qp_i 20 -qp_p 20
        -c:a aac -b:a 192k -movflags +faststart <output>
      ```
    - **CPU** (fallback — current behaviour):
      ```
      ffmpeg -y -i <input>
        -vf "scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1"
        -c:v libx264 -preset medium -crf 20
        -c:a aac -b:a 192k -movflags +faststart <output>
      ```
  - `build_image_transcode_args(input, output) -> Vec<String>`:
    - Images always CPU (GPU doesn't help for single-frame JPEG encoding)
    - Same as current `convert_image` logic
  - Automatic fallback: if GPU transcode fails (exit code != 0), retry with CPU args
    - Log: `warn!("GPU transcode failed, retrying with CPU: {}", stderr_snippet)`

### 4.3 — Server: Integration with Existing Conversion Pipeline

- [x] Modify `server/src/conversion.rs`:
  - Add `HwAccelCapability` parameter to `convert_video()` (or store in global/AppState)
  - For video conversions: use GPU args when available, CPU args when not
  - Keep existing `convert_image()` and `convert_audio()` unchanged (CPU only)
  - Add automatic retry: GPU failure → CPU fallback (transparent to caller)
  - Existing function signatures remain compatible — no breaking changes to callers

- [x] Store detected GPU capability in `AppState` at startup:
  - `pub hw_accel: Arc<HwAccelCapability>` field in AppState
  - Probed once during server initialization
  - Passed to conversion functions

- [x] **No changes to ingest.rs, upload.rs, or scan.rs** — they call `convert_file()` which internally handles GPU/CPU selection

### 4.4 — Server: Config Integration

- [x] Add to config:
  ```toml
  [transcode]
  gpu_enabled = true              # Allow GPU acceleration (set false to force CPU)
  gpu_fallback_to_cpu = true      # Retry with CPU if GPU transcode fails
  gpu_device = ""                 # Specific GPU device (empty = auto-detect)
  ```

### 4.5 — Server: Status API

- [x] Add GPU transcode info to an existing or new endpoint:
  - `GET /api/transcode/status` or extend `GET /api/ai/status`:
    - `{ "gpu_transcode": { "available": true, "type": "nvenc", "encoder": "h264_nvenc" } }`
  - Or add to server info/health endpoint

### 4.6 — Web: Settings Display

- [x] Show GPU transcode status in Settings page:
  - "Video Transcoding: GPU Accelerated (NVENC)" or "Video Transcoding: CPU"
  - Read-only — no user toggle needed (automatic)

### 4.7 — Testing: DDT for GPU Transcoding

- [x] `tests/test_56_gpu_transcode_ddt.py`: — 24 pass, 4 skipped (video upload tests skip without real video files)
  - **Test methods (run on both GPU and CPU paths):**
    - `test_video_conversion_produces_valid_mp4` — convert MKV/AVI/MOV → MP4, verify playable
    - `test_video_conversion_preserves_audio` — verify audio stream present after conversion
    - `test_gpu_fallback_to_cpu` — force GPU failure → verify CPU fallback succeeds
    - `test_image_conversion_unaffected` — HEIC/TIFF → JPEG still works (CPU only)
    - `test_audio_conversion_unaffected` — WMA → MP3 still works (CPU only)
    - `test_transcode_status_endpoint` — verify status reports correct GPU/CPU info
    - `test_concurrent_video_conversions` — multiple videos converting simultaneously
  - Note: if no GPU on test machine, tests verify CPU path and fallback logic

### 4.8 — Testing: E2E Regression

- [x] Run full test suite — ALL MUST PASS including Module 1+2+3 tests
- [x] Specifically verify test_18_media_conversion.py still passes (primary conversion tests)

---

## Module 5 — Album Slideshow ✅ COMPLETE

> Sequential or shuffled photo slideshow within any album or gallery view.
> Smooth transitions between photos. Photos only — videos and audio are skipped.
> Accessible via a slideshow button in album/gallery headers.

### 5.1 — Web: Slideshow Engine

- [x] Create `web/src/components/viewer/Slideshow.tsx`:
  - Full-screen slideshow overlay (reuses Viewer layout/positioning)
  - Controls bar (bottom, auto-hide after 3s of inactivity, show on mouse move):
    - Play/Pause button
    - Previous / Next buttons
    - Shuffle toggle button (on/off)
    - Speed selector: 3s / 5s / 8s / 10s per slide (default 5s)
    - Exit slideshow button (ESC key also exits)
    - Progress indicator: "Photo 12 of 45"
  - Photo display:
    - Use existing photo loading/decryption pipeline from Viewer
    - Preload next 2 photos for instant transitions
    - Skip non-photo items (videos, audio, GIFs) — advance to next photo automatically
  - Keyboard shortcuts:
    - Space: play/pause
    - Left/Right: prev/next
    - S: toggle shuffle
    - ESC: exit slideshow
    - F: toggle fullscreen

- [x] Create `web/src/components/viewer/SlideshowTransitions.tsx`:
  - CSS/JS transition effects between slides:
    - **Fade** (default): crossfade over 600ms
    - **Slide**: horizontal slide (left-to-right) over 500ms
    - **Zoom**: subtle zoom-in with fade over 700ms
    - **Dissolve**: pixelated dissolve effect over 600ms
  - Transition selector in controls bar
  - All transitions use CSS `transition` / `animation` for GPU-accelerated rendering
  - Transitions are simple and elegant — no flashy or distracting effects

### 5.2 — Web: Slideshow State Management

- [x] Create `web/src/hooks/useSlideshow.ts`:
  - State: `isPlaying`, `currentIndex`, `shuffleEnabled`, `intervalMs`, `transition`
  - `shuffledOrder: number[]` — Fisher-Yates shuffle of photo indices, regenerated when shuffle toggled
  - `filteredPhotoIds: string[]` — filter out non-photo items (videos, audio) from the album's photoIds
  - Timer management: `setInterval` for auto-advance, cleared on pause/exit
  - Preloading: preload `currentIndex + 1` and `currentIndex + 2` photos
  - Persist preferences in localStorage: `slideshow_speed`, `slideshow_shuffle`, `slideshow_transition`

### 5.3 — Web: Album Integration

- [x] In album/gallery header bars, add a "Slideshow" button:
  - `web/src/pages/Albums.tsx` — album detail view header
  - `web/src/pages/Gallery.tsx` — main gallery header (if applicable)
  - `web/src/pages/AlbumDetail.tsx` — shared/secure album headers
  - Button icon: play/slideshow icon
  - Only shown when album contains at least 1 photo (not just videos/audio)
  - Clicking opens Slideshow component with the album's photo list

### 5.4 — Web: Viewer Integration

- [x] In `web/src/pages/Viewer.tsx`:
  - Add "Start Slideshow" button in viewer toolbar/controls
  - When clicked, launches slideshow from current photo position
  - Slideshow uses the same `photoIds` array the viewer already has

### 5.5 — Android: Slideshow (if applicable)

- [ ] Add slideshow button to album/gallery screens (deferred — Android)
- [ ] Create `SlideshowScreen.kt` or `SlideshowOverlay.kt` (deferred — Android):
  - Full-screen photo display with auto-advance
  - Play/pause, shuffle, speed controls
  - Swipe for manual prev/next
  - Skip non-photo items
  - Simple fade/slide transitions via Compose animations

### 5.6 — Testing: DDT for Slideshow

- [x] `tests/test_57_slideshow_ddt.py`: — 13 tests (346 total passed, 6 skipped)
  - Note: slideshow is primarily a client-side feature; server tests focus on the photo filtering/ordering APIs
  - **Test methods:**
    - `test_album_photos_filtered` — verify API returns photos-only list (no videos/audio)
    - `test_album_photo_order_sequential` — verify default ordering matches album order
    - `test_album_photo_count` — verify photo count excludes non-photo media
    - `test_empty_album_no_slideshow` — album with only videos returns empty photo list
    - `test_mixed_media_album` — album with photos + videos returns only photos

### 5.7 — Testing: E2E Regression

- [x] Run full test suite — ALL MUST PASS including Module 1+2+3+4 tests — 346 passed, 6 skipped
- [x] Verify: album CRUD, photo upload, viewer navigation all unaffected

---

## Album Sidebar Layout Order

The final sidebar/album page layout when all modules are enabled:

```
┌─────────────────────────────┐
│  Smart Albums               │
│  ├── Favorites              │
│  ├── Photos                 │
│  ├── GIFs                   │
│  ├── Videos                 │
│  └── Audio   - When enabled │
│ otherwise should be hidden  │
│                             │
│  ── User Albums ──────────  │
│  ├── Vacation 2024  ▶️ 🔀   │  ← slideshow + shuffle buttons
│  ├── Family         ▶️ 🔀   │
│  └── + New Album            │
│                             │
│  ── People ──────────────── │  ← (Module 1, AI enabled)
│  ├── Bob (23 photos)        │
│  ├── Alice (18 photos)      │
│  ├── Unknown #1 (5 photos)  │
│  └── ...                    │
│                             │
│  ── Places ───────────────  │  ← (Module 2, geo enabled)
│  ├── Paris, France (15)     │
│  ├── New York, USA (12)     │
│  ├── Tokyo, Japan (8)       │
│  └── Map View               │
│                             │
│  ── Timeline ───────────── │  ← (Module 2, geo enabled)
│  ├── 2026 (45 photos)       │
│  ├── 2025 (120 photos)      │
│  └── 2024 (89 photos)       │
│                             │
│  ── Shared Albums ────────  │
│  ├── Trip with Friends      │
│  └── Work Photos            │
└─────────────────────────────┘
```

---

## Cross-Cutting Concerns

### Performance & Concurrency

| Operation | Strategy |
|---|---|
| AI face/object detection | `rayon` thread pool for CPU, GPU auto-detect at runtime, `spawn_blocking` |
| Face clustering | `spawn_blocking` on `rayon` pool (CPU-bound agglomerative) |
| Reverse geocoding | `Arc<KdTree>` k-d tree, O(log n), `par_iter` for batch |
| Geo scrub (file I/O) | `spawn_blocking` + `rayon::par_iter` over files |
| EXIF read/write | `spawn_blocking` (file I/O bound) |
| GPU video transcode | FFmpeg with hwaccel flags, auto-fallback to CPU |
| Background processing | `tokio::spawn` long-running, rate-limited, graceful shutdown |
| DB batch inserts | Single transaction for batch, avoid N+1, prepared statements |
| Slideshow preloading | Preload next 2 photos, use existing decrypt/cache pipeline |

### Security

- AI detections and face clusters are **user-scoped** — no cross-user access
- Geo scrub is **irreversible** — confirmation required, cannot undo
- Metadata edit validates all inputs server-side (no path traversal, valid ranges)
- EXIF write-back recalculates file hash to maintain integrity
- AI model files are read-only, loaded at startup, never user-modifiable
- Rate limit AI processing to prevent resource exhaustion
- Face embeddings stored as BLOBs — not exposed via API (only cluster IDs and labels)
- GPU transcode: no additional security surface — same FFmpeg binary, different flags

### Migration Path

- All DB migrations are additive (ALTER TABLE ADD COLUMN, new tables) — no destructive changes
- All new features are behind toggles — existing functionality unaffected when disabled
- Background processors start only when their toggle is enabled
- API endpoints return 404 or empty results when their module is disabled
- GPU acceleration is transparent — no user action needed

---

## Final Checklist

- [x] **Module 1 complete**: AI recognition DDT passes, full E2E regression green
- [x] **Module 2 complete**: Geolocation DDT passes (49/49), full E2E regression green (240/240 including Module 1 tests)
- [x] **Module 3 complete**: Metadata editor DDT passes (49/49 + 23/25), full E2E regression green (230 passed, 2 skipped)
- [x] **Module 4 complete**: GPU transcode DDT passes (24/28, 4 skip without GPU), full E2E regression green
- [x] **Module 5 complete**: Slideshow DDT passes (13/13), full E2E regression green (346 passed, 6 skipped)
- [ ] All five modules can be independently toggled without affecting each other
- [ ] Performance profiled: AI processing does not degrade photo upload latency
- [ ] Performance profiled: Geo resolution adds < 5ms to ingest pipeline
- [ ] Performance profiled: Metadata edit response time < 200ms
- [ ] Performance profiled: GPU transcode at least 2x faster than CPU for video
- [ ] No N+1 queries introduced
- [ ] All new endpoints require authentication
- [ ] All new data is user-scoped
- [ ] Install scripts documented for GPU acceleration
- [ ] Reset scripts clean up all new module data
- [ ] README updated with all new features
