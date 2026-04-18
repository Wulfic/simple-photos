# Feature Roadmap — TODO

> Generated: 2026-04-17  
> Status key: `[ ]` not started · `[~]` in progress · `[x]` done

---

## Overview

Six features across three layers (server, Android, web):

| Feature | Test media available | Notes |
|---|---|---|
| Motion Photo | ✅ (from phone) | JPEG+MP4 embedded |
| 360 Photo | ❌ (need to capture/source) | Equirectangular |
| Panorama | ✅ (from phone) | Shares viewer module with 360 |
| HDR | ✅ (from phone) | JPEG with Gainmap or AVIF |
| Burst Photo | ✅ (from phone) | Time-clustered sequence |
| Google Cast | N/A (runtime feature) | Needs Cast App ID registered |

---

## 1 — Foundation: `photo_subtype` DB Column & Detection

These tasks unblock all features. Must be done first.

### 1.1 — Server: DB Migration

- [x] Create `server/migrations/016_photo_subtype.sql`
  - Add `photo_subtype TEXT` column to `photos` table (nullable, default NULL)
  - Valid values: `'motion'`, `'panorama'`, `'equirectangular'`, `'hdr'`, `'burst'`
  - Add index: `CREATE INDEX idx_photos_subtype ON photos(user_id, photo_subtype) WHERE photo_subtype IS NOT NULL;`
  - Add `burst_id TEXT` column to `photos` for grouping burst shots
  - Add index: `CREATE INDEX idx_photos_burst ON photos(user_id, burst_id) WHERE burst_id IS NOT NULL;`
  - Add `motion_video_blob_id TEXT REFERENCES blobs(id)` to store the extracted motion video blob

### 1.2 — Server: XMP Metadata Extraction

- [x] Add `kamadorio/xmp` or `xmp-rs` crate (or use regex/manual XML parse) to `Cargo.toml`
- [x] Extend `server/src/photos/metadata.rs` — `extract_xmp_subtype(file_path)` function:
  - Read first 64 KB of JPEG to locate `<x:xmpmeta` block (UTF-8 scan)
  - **Motion Photo**: detect `Camera:MicroVideo="1"` or `GCamera:MicroVideoOffset`
    - Extract the MP4 trailer: `offset = file_size - MicroVideoOffset`; read bytes `offset..end`
    - Return subtype `"motion"` + extracted video bytes
  - **Panorama/360**: detect `GPano:ProjectionType`
    - `"equirectangular"` → subtype `"equirectangular"` (360)
    - `"cylindrical"` or aspect ratio ≥ 3.5:1 → subtype `"panorama"`
  - **HDR**: detect `hdrgm:Version` (Gainmap / Ultra HDR) or AVIF HDR colour profile
    - Return subtype `"hdr"`
  - **Burst**: detect `MicroVideo:BurstID`, `com.google.photos.burst.id`, or EXIF `ImageUniqueID` shared across sequence
    - Return `burst_id` string
- [x] Extend `MediaMetadata` tuple (or create `ExtendedMediaMetadata` struct) to carry `photo_subtype` and `burst_id`
- [x] Wire new extraction into `ingest.rs` → INSERT path and `photos/upload.rs` → upload path
- [x] Wire into `backup/autoscan.rs` → scan-registration path
- [x] Update `Photo` and `PhotoRecord` models (`server/src/photos/models.rs`) to include `photo_subtype`, `burst_id`, `motion_video_blob_id`
- [x] Expose `photo_subtype` and `burst_id` in `GET /api/photos` response (already via `PhotoRecord`)
- [x] Add `GET /api/photos?subtype=motion|panorama|equirectangular|hdr|burst` filter param

### 1.3 — Server: Motion Video Blob Storage

- [x] During motion photo ingest: extract trailing MP4 bytes, store as a new blob (`blob_type = "motion_video"`)
- [x] Set `motion_video_blob_id` on the parent photo row
- [x] Add `GET /api/photos/{id}/motion-video` endpoint → serve the motion video blob (same auth as regular serving, honour encrypted flag)
- [ ] For encrypted photos: the motion video blob must also be encrypted with the same key envelope — document this; client encrypts and uploads separately via `POST /api/blobs` then calls a `PATCH /api/photos/{id}` to link `motion_video_blob_id`

### 1.4 — Android DTO / Entity Updates

- [ ] Add `photoSubtype: String?`, `burstId: String?`, `motionVideoBlobId: String?` fields to `PhotoDto` / `PhotoEntity` (local DB entity + remote DTO)
- [ ] Update Room entity migration (local DB version bump)
- [ ] Update `PhotoRepository` mapping functions

### 1.5 — Web TypeScript Types

- [x] Add `photo_subtype?: string`, `burst_id?: string`, `motion_video_blob_id?: string` to the `Photo` type (`web/src/types/`)
- [x] Update any places that construct or map photo objects

---

## 2 — Motion Photo

> Google Motion Photos: JPEG file with an embedded MP4 appended after the JPEG end marker.

### 2.1 — Server (already covered in §1.2–1.3 above)

- [x] Verify extraction handles both old (`MicroVideo`) and new (`MotionPhoto`) XMP schemas
- [x] E2E test: `tests/test_44_motion_photo.py`
  - Upload a real motion photo JPEG from phone
  - Assert `photo_subtype == "motion"` in list response
  - Assert `GET /api/photos/{id}/motion-video` returns `video/mp4` content
  - Assert `Content-Length` matches the extracted trailer size

### 2.2 — Android Viewer

- [ ] In `PhotoViewerScreen.kt`: detect `photoSubtype == "motion"` for the current page
- [ ] Create `MotionPhotoPlayer.kt` composable:
  - Shows the static JPEG via Coil as normal
  - On long-press OR when `autoPlayMotion` setting is true: fetch `GET /api/photos/{id}/motion-video` → load into a silent looping `ExoPlayer` instance layered over the image
  - Overlay a small ▶ badge in the top-left corner of the gallery tile to indicate motion photo
  - Looping: `repeatMode = Player.REPEAT_MODE_ONE`, muted
  - Stop playback and show JPEG again when user navigates away
- [ ] `PhotoViewerComponents.kt`: add motion badge overlay to `MediaTile`
- [ ] Settings: add "Auto-play motion photos" toggle (default ON)
- [ ] For encrypted motion photos: decrypt motion video blob in-memory same as regular encrypted blobs (use existing decrypt-to-memory pipeline in `PhotoViewerViewModel`)

### 2.3 — Web Viewer

- [ ] In `Viewer.tsx` / `useViewerMedia` hook: detect `photo_subtype === 'motion'`
- [ ] Create `MotionPhotoViewer.tsx` component:
  - Renders `<img>` for the static JPEG
  - On hover (desktop) or tap-hold (mobile): fetch motion video URL → create `<video>` overlay (`autoPlay`, `loop`, `muted`, `playsInline`)
  - Small animated ▶ badge on the image
  - Clean up video element on unmount / navigation
- [ ] Gallery tile: add motion badge overlay in `web/src/gallery/components/`
- [ ] Encrypted support: decrypt motion video blob in a Web Worker (same pattern as existing encrypted blob decryption)

---

## 3 — Panorama + 360 Shared Viewer Module

> Panorama (cylindrical, wide-angle) and 360° (equirectangular, full sphere) share a common renderer.  
> The shared module renders both; the projection type determines the field of view clamp.

### 3.1 — Server (already covered in §1.2)

- [x] E2E test: `tests/test_45_panorama_360.py`
  - Upload a real panoramic JPEG → assert `photo_subtype == "panorama"`
  - Upload an equirectangular 360 JPEG (source from Wikimedia Commons for test) → assert `photo_subtype == "equirectangular"`
  - Assert both appear in `GET /api/photos?subtype=panorama` and `equirectangular` respectively

### 3.2 — Android Shared Panorama/360 Module

- [ ] Add dependency: **Panorama SDK** — use `com.google.vr:sdk-panowidget` (Android Jetpack VR / deprecated but stable) OR migrate to a maintained alternative such as **SceneView** (`io.github.sceneview:sceneview`) for equirectangular rendering
  - Recommended: use SceneView's equirectangular sphere approach for both, with cylindrical UV clamp for panorama
  - Add to `android/app/build.gradle.kts`
- [ ] Create `PanoramaViewer.kt` composable:
  - `AndroidView` wrapping the chosen renderer
  - Parameter: `projectionType: PanoramaProjection` (enum `EQUIRECTANGULAR`, `CYLINDRICAL`)
  - For `CYLINDRICAL`: clamp vertical FOV to ±60°, allow full horizontal 360° drag
  - For `EQUIRECTANGULAR`: full sphere navigation (gyroscope + drag)
  - Pinch-to-zoom (adjust FOV between 30°–120°)
  - Gyroscope integration with `SensorManager` — toggle on/off via button
  - Share/download actions still available from toolbar
- [ ] In `PhotoViewerScreen.kt`: when `photoSubtype == "panorama"` or `"equirectangular"`, swap out the normal `PhotoPage` composable for `PanoramaViewer`
- [ ] Gallery tile: show panorama badge (wide-angle icon) for both subtypes
- [ ] Rotation disabled in edit panel for panoramic photos (nonsensical)

### 3.3 — Web Shared Panorama/360 Module

- [ ] Add dependency: **Pannellum** (`pannellum`) or **Photo Sphere Viewer** (`@photo-sphere-viewer/core`) — prefer Photo Sphere Viewer (actively maintained, React-friendly, supports both equirectangular and partial panorama)
  - Add to `web/package.json`
- [ ] Create `PanoramaViewer.tsx` component:
  - Wraps `PhotoSphereViewer` with a `container` div
  - Props: `src: string`, `projection: 'equirectangular' | 'cylindrical'`
  - For cylindrical: use `littlePlanet: false`, `minFov`/`maxFov` appropriate settings, `sphereCorrection` to clamp vertical
  - Keyboard + mouse drag navigation
  - Fullscreen button
  - Gyroscope plugin (mobile) via `@photo-sphere-viewer/gyroscope-plugin`
  - Clean destroy on unmount
- [ ] In `Viewer.tsx`: render `<PanoramaViewer>` when `photo_subtype === 'panorama' || 'equirectangular'`; bypass `useZoomPan` hook for these photos (the viewer handles it internally)
- [ ] Gallery tile: panorama badge
- [ ] Encrypted support: decrypt to object URL first, pass URL to viewer

---

## 4 — HDR Photo Support

> Ultra HDR (Gainmap JPEG / JPEG XL) and AVIF HDR. HEIC HDR is already converted to AVIF by the conversion pipeline.

### 4.1 — Server

- [x] Extend XMP extraction (§1.2) to detect `hdrgm:Version` (Ultra HDR Gainmap)
- [x] Detect AVIF HDR: check for `av1C` box with HDR colour primaries (use `imagesize` crate or raw byte inspection of the AVIF container)
- [x] Mark `photo_subtype = 'hdr'`; do not strip the Gainmap during conversion — ensure `ffmpeg`/`ImageMagick` conversion paths preserve HDR metadata (may need `-map_metadata 0` flags in conversion pipeline)
- [x] E2E test: `tests/test_46_hdr.py`
  - Upload a real HDR JPEG from phone
  - Assert `photo_subtype == "hdr"` in list response
  - Assert served file contains Gainmap XMP (verify Content-Type + byte signature)

### 4.2 — Android Viewer

- [ ] In `PhotoViewerComponents.kt` / image loading: for `photoSubtype == "hdr"`, request the blob and load via `BitmapFactory` with `BitmapFactory.Options.inPreferredColorSpace = ColorSpace.get(ColorSpace.Named.DISPLAY_P3)` on API 26+, or use `ImageDecoder.createSource` with HDR output intent
- [ ] For Android 14+ (`UltraHDRImage`): use `ImageDecoder` with `setTargetColorSpace(ColorSpace.get(DISPLAY_P3))` and `setPostProcessor` if needed
- [ ] Fallback: SDR rendering for older API levels (standard Coil load, no special handling needed — Gainmap is ignored gracefully)
- [ ] Add small "HDR" badge on the photo details in `ViewerInfoPanel.kt`
- [ ] Gallery tile: HDR badge overlay (subtle, e.g. small "HDR" chip)

### 4.3 — Web Viewer

- [ ] For `photo_subtype === 'hdr'`: add `<meta name="color-scheme" content="light dark">` and ensure the `<img>` is served with the correct colour profile (`Content-Type: image/jpeg` — browser auto-applies ICC profile if embedded)
- [ ] Chrome 116+ supports Ultra HDR natively via `<img>` — no JS needed for supported browsers
- [ ] Add feature-detection: `const supportsHDR = window.matchMedia('(dynamic-range: high)').matches`
- [ ] Show "HDR" badge in `ViewerTopBar.tsx` when `photo_subtype === 'hdr'` and `supportsHDR`
- [ ] Show "HDR (SDR display)" badge when subtype is HDR but display does not support HDR
- [ ] Gallery tile: HDR badge

---

## 5 — Burst Photo Support

> A burst is a rapid sequence of photos sharing the same `burst_id`. Display as a stacked tile in the gallery; view individual frames in a filmstrip.

### 5.1 — Server

- [x] `GET /api/photos` response: optionally collapse bursts — add query param `?collapse_bursts=true`
  - When enabled: return only the first shot in each burst (representative), with a `burst_count: N` field added to the `PhotoRecord`
  - When disabled (default): return all photos individually (backwards-compatible)
- [x] `GET /api/photos/burst/{burst_id}` endpoint — returns all photos in a burst ordered by `taken_at ASC`
- [x] Add `burst_count` field to `PhotoRecord` (populated as 1 when not in a burst, or as the group size when `collapse_bursts=true`)
- [x] E2E test: `tests/test_47_burst.py`
  - Upload a sequence of burst photos (real burst JPEGs from phone → verify they share `burst_id`)
  - Assert `GET /api/photos?collapse_bursts=true` returns only representative + `burst_count > 1`
  - Assert `GET /api/photos/burst/{burst_id}` returns all frames
  - Assert individual frame viewing still works normally

### 5.2 — Android Gallery + Viewer

- [ ] In `GalleryScreen.kt`: when `burstId != null`, show a stacked-tile visual (offset shadow layers) and a frame count badge (e.g. "12")
- [ ] Tapping a burst tile: navigate to viewer with the full burst sequence as `photoIds`, opening at the cover/representative frame
- [ ] In `PhotoViewerScreen.kt`: when viewing a burst, show a horizontal filmstrip scrubber at the bottom (reuse `HorizontalPager` or a `LazyRow` thumbnail strip)
  - Filmstrip navigates frames; main pager and filmstrip stay in sync
  - "Burst" label in the top bar replacing the filename
- [ ] Option to "Pick best" — copies the selected frame as a standalone photo (calls existing download/save-to-device flow)
- [ ] In `PhotoViewerViewModel.kt`: add `loadBurstFrames(burstId)` which calls `GET /api/photos/burst/{burst_id}`

### 5.3 — Web Gallery + Viewer

- [ ] Gallery tile (`web/src/gallery/`): detect `burst_id` on photo; show stacked tile + frame count badge
- [ ] In `Gallery.tsx` / `useGalleryPhotos` hook: support `collapse_bursts` query param toggle; add "Show individual burst frames" toggle in gallery settings
- [ ] In `Viewer.tsx`: when `burst_id` is set, fetch burst frames via `GET /api/photos/burst/{burst_id}`, show a horizontal filmstrip at the bottom
  - Clicking a filmstrip frame updates the main viewer image
  - "Burst" shown in `ViewerTopBar`
- [ ] "Save this frame" button → download only the currently displayed frame

---

## 6 — Google Cast (Chromecast)

> Cast photos and videos to any Google Cast-compatible device on the local network.  
> Requires a registered Cast Application ID (receiver app hosted or using the default media receiver).

### 6.1 — Prerequisites & Infrastructure

- [ ] Register a Cast Application ID at [cast.google.com/publish](https://cast.google.com/publish)
  - Use the **Default Media Receiver** for MVP (no custom receiver app needed initially)
  - Store the App ID in Android `strings.xml` and web `.env` as `VITE_CAST_APP_ID`
- [ ] Server: add `GET /api/photos/{id}/cast-url` endpoint
  - Returns a short-lived (15-minute) signed URL for the photo/video that is accessible over LAN
  - The URL must be HTTP (not HTTPS) OR the Cast device must trust the certificate — plan for HTTP serving on LAN port or use a temporary token param (`?cast_token=...`)
  - Token stored in-memory (`HashMap<token, (photo_id, user_id, expires_at)>`), not in DB
  - Include `Content-Type` header in the served response for Cast to detect media type
- [ ] Server: `GET /api/cast/{token}` — serves the blob (decrypted) using the temporary token; validates token not expired

### 6.2 — Android Cast Module

- [ ] Add Cast SDK dependency to `android/app/build.gradle.kts`:
  ```kotlin
  implementation("com.google.android.gms:play-services-cast-framework:21.5.0")
  ```
- [ ] Create `cast/CastManager.kt` singleton:
  - Initialises `CastContext` with the registered App ID
  - Provides `fun castPhoto(photo: PhotoEntity, sessionToken: String)` and `fun castVideo(...)`
  - Handles session state: `SESSION_STARTING`, `SESSION_STARTED`, `SESSION_ENDING`, `SESSION_ENDED`
  - `CastStateListener` updates UI state via a `StateFlow<CastState>`
- [ ] Create `cast/CastOptionsProvider.kt` implementing `OptionsProvider` (required by Cast SDK)
- [ ] Register `CastOptionsProvider` in `AndroidManifest.xml`
- [ ] `AppHeader.kt`: add Cast button (media route button) when Cast devices are available — use `MediaRouteButton` wrapped in `AndroidView`
- [ ] In `PhotoViewerScreen.kt`: add "Cast" action in the top-bar action row
  - On tap: call `GET /api/photos/{id}/cast-url` → get token → call `CastManager.castPhoto(...)`
  - While casting: show a mini Cast controller at the bottom of the viewer (pause / stop casting / seek for video)
- [ ] Handle encrypted photos: server `GET /api/cast/{token}` must serve the decrypted blob — Cast receiver has no decryption capability. Implement decryption server-side for the cast endpoint only (use the existing `crypto.rs` decrypt utilities with the stored key)
- [ ] Cast queue: support casting the full album/gallery as a slideshow (set `MediaQueueData` with all cast URLs, 5-second advance)

### 6.3 — Web Cast Module

- [ ] Load Cast Sender JS SDK in `index.html`:
  ```html
  <script src="https://www.gstatic.com/cv/js/sender/v1/cast_sender.js?loadCastFramework=1"></script>
  ```
- [ ] Create `web/src/hooks/useCast.ts` hook:
  - Initialises `cast.framework.CastContext` with App ID and auto-join policy
  - Exposes `castState`, `castPhoto(photoId)`, `stopCasting()`, `sessionAvailable`
  - Handles `CAST_STATE_CHANGED` events → updates React state
- [ ] `AppHeader.tsx`: render Cast button (Google Cast icon) when `sessionAvailable` or devices detected
- [ ] `ViewerTopBar.tsx`: add Cast action button
  - On click: fetch `GET /api/photos/{id}/cast-url` → construct `chrome.cast.media.MediaInfo` with returned URL → load via `RemoteMediaClient`
- [ ] Mini cast controller component: show during active cast session (pause/stop/seek)
- [ ] Slideshow mode: queue all photos in current gallery/album as a cast queue
- [ ] TypeScript types: add `@types/chromecast-caf-sender` or declare types manually

---

## 7 — Testing

### 7.1 — Server E2E Tests (Python/pytest)

- [ ] `tests/test_44_motion_photo.py` — motion photo detection, video extraction, serve endpoint
- [ ] `tests/test_45_panorama_360.py` — panorama/360 subtype detection, filter API
- [ ] `tests/test_46_hdr.py` — HDR Gainmap detection, file integrity (Gainmap preserved)
- [ ] `tests/test_47_burst.py` — burst grouping, collapse API, burst frame listing
- [ ] `tests/test_48_cast_tokens.py` — cast token generation, expiry, single-use enforcement, decrypted serving

### 7.2 — Android Unit Tests

- [ ] `XmpParserTest.kt` — unit test XMP extraction for motion, panorama, 360, HDR, burst signatures
- [ ] `CastManagerTest.kt` — mock CastContext, verify session state transitions
- [ ] `BurstGroupingTest.kt` — verify burst_id grouping in `GalleryScreen` data layer

### 7.3 — Web Unit Tests

- [ ] `useCast.test.ts` — mock `window.cast`, verify state transitions
- [ ] `PanoramaViewer.test.tsx` — renders without crash, correct projection prop forwarded
- [ ] `MotionPhotoViewer.test.tsx` — hover triggers video load, unmount cleans up

### 7.4 — Manual Test Checklist

- [ ] Motion photo plays on long-press (Android) and hover (web)
- [ ] Panorama photo opens in spherical scroll view (not pinch-zoom)
- [ ] 360 photo opens in equirectangular sphere, gyroscope works on mobile
- [ ] HDR badge visible on compatible display; graceful fallback on SDR
- [ ] Burst tile shows stacked visual + frame count in gallery
- [ ] Burst filmstrip scrubs smoothly in viewer
- [ ] Cast session starts, photo appears on TV
- [ ] Cast encrypted photo decrypted correctly server-side (sensitive path)
- [ ] Slideshow advances through gallery on Cast device

---

## 8 — Notes & Decisions

### Panorama ↔ 360 shared module design
- Both features use a single `PanoramaViewer` component on each platform.
- The `projectionType` property determines rendering: cylindrical (clamp vertical) vs. equirectangular (full sphere).
- Detection priority: XMP `GPano:ProjectionType` is authoritative; aspect ratio fallback (≥3.5:1) used only when XMP is absent.
- 360 test photo: source from [Wikimedia Commons equirectangular images](https://commons.wikimedia.org/wiki/Category:Equirectangular_photographs) for automated tests; real device capture needed for manual QA.

### Cast & Encryption
- The Cast receiver (Default Media Receiver) cannot run custom JS → it cannot decrypt AES-256-GCM client-side encrypted blobs.
- The server must decrypt blobs before serving them over the cast endpoint.
- The cast token endpoint (`/api/cast/{token}`) is the only place where server-side decryption happens for client-encrypted data.
- Tokens are short-lived (15 min) and stored in-memory only — they are never persisted to SQLite.
- Rate-limit `/api/photos/{id}/cast-url` to prevent token farming.

### HDR Compatibility Matrix
| Platform | Format | Support |
|---|---|---|
| Android 14+ | Ultra HDR JPEG | Native (`ImageDecoder`) |
| Android 10–13 | Ultra HDR JPEG | SDR fallback (Gainmap ignored) |
| Chrome 116+ | Ultra HDR JPEG | Native `<img>` |
| Safari | Ultra HDR JPEG | SDR fallback |
| All | AVIF HDR | Browser-dependent; AVIF already in serving pipeline |

### Motion Photo Schema Notes
- `motion_video_blob_id` is a foreign key to `blobs` — if the blob is deleted the column is not cascade-deleted automatically (set to NULL on blob delete via trigger or application logic).
- Encrypted motion videos: the client extracts the MP4 trailer, encrypts it separately, uploads as a blob, then PATCHes the photo record. This keeps the existing end-to-end encryption model intact.

### Burst Detection Fallback
- Primary: `GCamera:BurstID` or `com.google.photos.burst.id` XMP field.
- Secondary: EXIF `ImageUniqueID` is per-shot; instead use filename pattern `IMG_20260417_102345_BURST001.jpg` — extract common prefix + sequence number.
- Tertiary: cluster photos with same `camera_model` and `taken_at` within a 2-second window during scan (heuristic, may produce false positives — only apply during autoscan, not upload).
