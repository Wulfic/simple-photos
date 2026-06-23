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

- [ ] **W5 — `<SmartClusterList>` + `<SmartAlbumDetail>`**: Trips/Memories/Pets/People
  views are ~95% identical (list = cluster cards + thumb loader; detail = fetch →
  resolveServerPhotos → header → grid → slideshow). Collapse 4 files to thin configs.

- [ ] **W6 — `<DetailHeader>`**: back-arrow + title + count + actions repeated 6+ times.

## ANDROID (`android/.../com/simplephotos`)

- [ ] **A1 — shared `PhotoTile`**: 6 separate tile composables (AlbumPhotoTile, MediaTile,
  SearchResultTile, TrashTile, ClusterTile, SecureItemTile). `JustifiedGrid` is shared;
  tiles aren't. One tile with badge/selection slots.

- [ ] **A2 — `rememberAuthedThumbnailRequest()`**: `ImageRequest.Builder` (auth header +
  crossfade) hand-built 18x across 11 screens. Mechanical extraction, zero behavior change.

- [ ] **A3 — shared selection controller**: selectedIds/selectionMode state machine
  re-implemented in 6 screens. Web has `usePhotoSelection`; Android has nothing.

- [ ] **A4 — extract shared viewer core**: PhotoViewerScreen (1120L) + SecurePhotoViewer
  (411L) overlap heavily. Pull common gesture/paging/chrome into a core.

- [ ] **A5 — consolidate thumbnail decode/decrypt**: Base64/BitmapFactory decode scattered
  across ~8 files. Route through one decode path (`ChunkedBlob`/`AvifCoilDecoder`).

## SERVER (`server/src`) — deferred, different flavor

- [ ] **S1 — `blob_stream` module** (LATER): blob decrypt/stream logic spread across
  backup/serve/blobs.rs, photos/serve.rs, gallery/secure.rs, blobs/. Not the "re-coded
  component" smell; defer until UI work lands.

## Cross-cutting note (not scheduled)

- web↔android DTO shapes drift (server authoritative). Flagged, out of scope this pass.

---

## Execution order
W2 → A2 → W1 → W3 → W4 → A1 → A3 → W5/W6 → A4/A5 → S1
