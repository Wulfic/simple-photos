# Modularization TODO

Deduplication / modularization pass across web + android. The pattern everywhere:
shared primitives exist, but consumers re-hand-wire boilerplate instead of plugging
into one module. Work top-down by impact/risk. Record to memory after each item.

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done (built + tested)

---

## WEB (`web/src`)

- [x] **W1 — `<SlideshowHost>`**: DONE (2026-06-23, tsc clean, uncommitted). Built
  `SlideshowHost` (overlay) + `SlideshowTriggers` (play/shuffle buttons) +
  `usePhotoSlideshow(photos)` hook. Migrated all 7 sites (Viewer + 6 albumDetail views);
  each album view shed ~50 lines of duplicated spread/memo/buttons.

- [x] **W2 — `<Modal>` primitive**: DONE (2026-06-23, tsc clean, uncommitted). Built
  `ui/Modal.tsx` (Escape/backdrop close, scroll-lock, optional header, size, testId) +
  `SharePickerModal.tsx` (consolidated 3 identical inline pickers). Migrated 13 sites
  (6 dedicated modal components + 4 inline confirms + 3 share pickers). Fixed 2
  hardcoded `bg-gray-*` light-mode bugs for free; ~9 modals gained Escape-to-close.
  Left full-screen viewers (Slideshow, backup lightbox) alone — those aren't dialogs.

- [x] **W3 — `useIdbThumbnailMap` hook**: DONE (2026-06-23, tsc clean, uncommitted).
  Built `hooks/useIdbThumbnailMap.ts`; migrated the 4 identical cluster-map loaders
  (Trips/Memories/Pets/People). SCOPE: the "13 sites" were not one pattern — ThumbnailTile
  already uses `useThumbnailLoader`; the inline bytes→url sites (Viewer/BurstStrip/Search/
  Trash/Gallery/SecureAlbumCover/AlbumTile) are entangled with media flows and left alone
  (force-abstracting them = leaky). W5 SmartClusterList will reuse this hook.

- [x] **W4 — `formatEta`**: DONE (2026-06-23, tsc clean, uncommitted). Added `formatEta`
  to `utils/formatters.ts`; removed the 5 identical local copies (Ai/Conversion/Encryption/
  Geo/PreciseGeo).
- [ ] **W4b — `usePollingProgress` hook** (DEFERRED — needs runtime verify): Ai+Encryption
  share a pending-decreasing batch tracker, but Conversion uses a different done/total model
  and Encryption adds conversion-suppression + cursor pagination. No web test runner → can't
  verify a shared stateful polling hook without a live processing backlog. Do this with the
  app running, not blind.

- [x] **W5 — `<SmartClusterList>` + `<SmartAlbumDetail>`**: DONE (2026-06-23, tsc clean,
  uncommitted). Built `SmartClusterList` (list scaffold: header + skeleton + empty + card
  grid; `variant` card/avatar; reuses `useIdbThumbnailMap`) and `SmartAlbumDetail` (detail
  scaffold: back header + count + slideshow + grid + SlideshowHost; optional `onRename`).
  Added `resolvePhotosByServerId` to resolveServerPhotos.ts (the shared Pets/People lookup
  loop). Collapsed Trips/Memories/Pets/People from ~765L to ~217L of thin configs; behavior
  preserved (progressive title via load ctx, no skeleton flash on cross-cluster nav, pets
  capitalize + token fallback + rename).

- [x] **W6 — `<DetailHeader>`**: DONE (2026-06-23, tsc clean, uncommitted). Built
  `components/DetailHeader.tsx` (back + truncating title + optional count + inline children +
  optional right-aligned `actions` group). Migrated 4 sites: SmartClusterList, SmartAlbumView,
  RegularAlbumView, SharedAlbumDetail. Left as exceptions: SmartAlbumDetail (inline rename
  form replaces the title), SecureGallery + Diagnostics (visually distinct headers), ViewerTopBar.

## ANDROID (`android/.../com/simplephotos`)

- [x] **A1 — shared photo-tile overlays**: DONE (2026-06-23, compileDebugKotlin green,
  uncommitted). Built `ui/components/PhotoTileOverlays.kt` with two `BoxScope` extensions:
  `TileSelectionCircle(isSelected, padding/size/checkSize)` (3 consumers: MediaTile,
  AlbumPhotoTile, TrashTile — latter two byte-identical, MediaTile only smaller) and
  `CloudBackupBadge()` (2 consumers: MediaTile, AlbumPhotoTile). SCOPE DECISION: did NOT merge
  the 6 tiles into one composable — they are not one family (ClusterTile is a label/subtitle
  cluster card; SearchResultTile/SecureItemTile aren't selectable the same way) and the
  duration/subtype/burst badges diverge per-tile in device-verified ways (AlbumPhotoTile omits
  the play-prefix; TrashTile badges at BottomEnd; crop transforms are MediaTile-only). A blind
  single-tile merge would risk the crop/badge regressions the device sessions fixed. Extracting
  the two genuinely-identical overlays is the safe, compile-verifiable win.

- [x] **A2 — `rememberThumbnailRequest()`**: DONE (2026-06-23, compileDebugKotlin green,
  uncommitted). Built `ui/components/ThumbnailRequest.kt` (`@Composable rememberThumbnailRequest(
  data, size?, crossfade=true, allowHardware=true)`). Migrated 17 `ImageRequest.Builder` sites
  across 9 files (gallery/album list+detail/search/trash/library/secure tile+viewer/pano
  overlays/photo-viewer components). Pano sites map `if(isPano){size;allowHardware(false)}` →
  `size=…, allowHardware=!pano`. CORRECTION: there was NO per-request auth header (TODO's
  "Authed" premise wrong) — auth is global via the OkHttp interceptor in NetworkModule backing
  the Coil ImageLoader. Skipped Sphere360View (non-composable coroutine scope + memoryCachePolicy
  DISABLED + bitmap-recycle rationale). Removed 3 orphaned `LocalContext` imports.

- [x] **A3 — shared selection controller**: DONE (2026-06-23, compileDebugKotlin green,
  uncommitted). Built `ui/components/SelectionState.kt` (`selectedIds`/`isSelectionMode` +
  `enter`/`toggle`/`setSelection`/`clear`) — the Android `usePhotoSelection`. The machine was
  byte-identical in 3 ViewModels (Gallery/AlbumDetail/Trash); each now holds a private
  `SelectionState` and re-exposes its public API by delegation, so the screens are UNCHANGED
  (lowest risk). SecureGallery uses a different selection mechanism — not affected.

- [x] **A4 — viewer core: investigated, no safe extraction remains** (2026-06-23). The
  shareable cores are ALREADY shared from prior sessions + A2: `PanoramaOverlay` is reused by
  SecurePhotoViewer; `Sphere360View`, `MAX_PANO_DECODE_PX`, and `describeImageBytes` are each
  defined once; the pano capped-decode now routes through `rememberThumbnailRequest` (A2). The
  remaining "overlap" is NOT literal duplication: SecurePhotoViewer (383L) pages over
  `SecureGalleryItem` + client-side decrypt with a single `panoLive` flag and NO zoom/pan;
  PhotoViewerScreen (1057L) pages over `PhotoEntity` + server fetch with a SUPERSET gesture
  machine (`panoLiveActive`+`liveVerticalDragActive`+`editMode`+swipe-down-dismiss+info-panel)
  — all device-verified (memory: pano vertical-drag split, gate on showInfoPanel). A blind
  generic-core merge would risk those regressions with no device to verify → deferred, same
  disposition as W4b. (Zero code change is the correct outcome here.)

- [x] **A5 — consolidate thumbnail-envelope decode**: DONE (2026-06-23, compileDebugKotlin
  green, uncommitted). Built `data/ThumbnailEnvelope.kt` → `decodeThumbEnvelope(decrypted):
  ByteArray?` (parse `{ "data": "<base64>" }` JSON → NO_WRAP decode; null when data
  absent/empty). Migrated the 5 identical inline parses across 4 files: PhotoRepository,
  TrashViewModel, PhotoViewerViewModel, SecureGalleryViewModel (×2). SCOPE: did NOT touch the
  two genuinely-different decode paths — the newline-wrapped full-media `Base64InputStream`
  streaming (PhotoRepository media path; memory: never fixed-chunk a wrapped value) and the
  `BitmapFactory`/`AvifCoilDecoder` bitmap decode (Samsung-AVIF, device-verified). Those are
  not duplication, they are distinct correctness-critical paths.

## SERVER (`server/src`) — deferred, different flavor

- [~] **S1 — `blob_stream` module: investigated; core goal already met, residual deferred**
  (2026-06-23). The blob decrypt/stream PRIMITIVE is already centralized: AES-GCM in
  `crypto.rs`, the SPCHNKB2 chunked decrypt (`decrypt_photo_blob`, `decrypt_blob_file_to_file`)
  in `blobs/chunked.rs`, storage in `blobs/storage.rs`. Every scattered call site already routes
  through `crate::blobs::` / `crate::crypto::` (verified: gallery/secure.rs, photos/serve.rs,
  backup/serve does no blob decrypt). The ONE genuine residual duplication is the HTTP
  ranged-streaming RESPONSE builder (parse Range → ReaderStream → 206+Content-Range / 200): it
  exists as `photos/serve.rs::serve_file_with_range` but is ALSO inlined a 2nd time in
  `serve_photo` (:279, differs only by an `open_file` tracing closure) and a 3rd time in
  `blobs/download.rs`. DEFERRED: that code is correctness-critical (range/seek), only truly
  verifiable against a live server with real video seeking, and lives in files the user is
  actively refactoring (recent commits split backup/serve + photos/metadata) — exactly why the
  original note said "defer until the server split lands." Recommend the in-flight server
  refactor fold the 3 ranged-serve copies into one `serve_file_with_range` (in `http_utils.rs`,
  next to the already-shared `parse_range_header`).

## Cross-cutting note (not scheduled)

- web↔android DTO shapes drift (server authoritative). Flagged, out of scope this pass.

---

## Execution order
W2 → A2 → W1 → W3 → W4 → A1 → A3 → W5/W6 → A4/A5 → S1
