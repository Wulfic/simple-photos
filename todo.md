# Secure Smart Albums — Audit & Implementation Plan

Goal: the Secure Albums section gets built-in smart albums — **Secure Gallery** (everything),
**Photos**, **GIFs**, **Videos**, **Audio** — on web AND Android. A smart album tile only
renders when it has at least one item. Membership is derived from `media_type`; nothing new
is stored.

---

## 1. Audit — current state (verified in code, 2026-07-16)

### Server (`server/src/gallery/secure.rs`, routes in `server/src/routes.rs:415-449`)
- `GET /api/galleries/secure` (`list_secure_galleries`) → `id, name, created_at, item_count`
  only. Plain JWT auth, **no gallery token**. No media-type info → the album-list screen
  cannot decide smart-tile visibility today.
- `GET /api/galleries/secure/{id}/items` (`list_gallery_items`) → per-gallery only,
  `X-Gallery-Token` verified server-side. Each item already carries
  `media_type, photo_subtype, burst_id, duration_secs, motion_video_blob_id, width, height,
  encrypted_thumb_blob_id` via `COALESCE(p.*, op.*)` (clone row `p` joined on
  `encrypted_blob_id IS NOT NULL`, original `op` via `original_blob_id`).
- **There is NO endpoint returning items across all of a user's secure galleries.** That is
  the one real backend gap.
- **`media_type` can come back NULL**: on a backup server the clone `photos` row may not
  exist and `original_blob_id` may not resolve (metadata sync stores enc ids directly on
  `encrypted_gallery_items`). Any classifier must define a NULL bucket.
- **Duplicate-membership invariant is CLIENT-ONLY.** `add_gallery_item` (secure.rs:300)
  never checks whether `req.blob_id` already lives in another secure gallery. The picker
  hides `secureBlobIds` (web `utils/secureAdd.ts`, Android `SecureGalleryViewModel.loadPhotos`),
  but two windows / a stale picker / a raw API call can double-add. The user's assumption
  "same photo can't be in 2 secure albums" — and the "Secure Gallery = union with no dupes"
  view — needs this enforced server-side (Phase 0).
- `media_type` domain observed: `photo | gif | video | audio` (blob_type derivation at
  secure.rs:391-397 confirms the set).

### Web
- [SecureGallery.tsx](web/src/pages/SecureGallery.tsx) — single page, three states:
  password gate → album card grid → album detail. Albums from `api.secureGalleries.list()`;
  items per album from `listItems(galleryId, token)`. Burst collapse done inline.
  Detail's Remove is hardwired to `selectedGallery.id` (line ~263).
- Return-from-viewer restore: `?album=<id>` is re-matched against `galleries.find(...)`
  (line ~144-150). A smart-album id will NOT match → must add a synthetic branch, or
  closing a photo inside a smart album dumps the user back at the album list.
- [Viewer.tsx](web/src/pages/Viewer.tsx#L109-L111) only uses `secureAlbumId` to build
  `/secure-gallery?album=${secureAlbumId}` — smart ids flow through untouched. Good.
- [SecureAlbumCover.tsx](web/src/gallery/components/SecureAlbumCover.tsx) fetches
  `listItems` per gallery just for a cover — the new aggregate response can feed smart-tile
  covers with zero extra requests.
- [smartAlbums.ts](web/src/gallery/smartAlbums.ts) — the main gallery's `SMART_ALBUM_DEFS`
  is the semantic precedent: `smart-photos` includes GIFs, `smart-gifs`/`smart-videos`/
  `smart-audio` are exact matches. **Do not reuse those ids** — they're wired into
  `useAlbumPhotos`/AlbumDetail routing and the secure-add picker; secure smart albums get
  their own `secure-smart-*` namespace.
- `isBackupServer` already gates create/add/delete/remove — smart albums are read-path
  only, so they render fine on backup servers with no extra work.

### Android
- [SecureGalleryScreen.kt](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecureGalleryScreen.kt)
  routes gate → `GalleryDetailView` → `GalleryListView` off `viewModel.selectedGallery`.
- [SecureGalleryViewModel.kt](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecureGalleryViewModel.kt)
  — `selectGallery()` calls `loadItems(gallery.id)` + `loadPhotos()` (picker prep).
  `removeItems()` is hardwired to `selectedGallery.id`. `fetchGalleryCover()` does a full
  `listItems` per card (same N+1 as web covers).
- [SecureGalleryDto.kt](android/app/src/main/kotlin/com/simplephotos/data/remote/dto/SecureGalleryDto.kt)
  — `SecureGalleryItem` already has `mediaType`; it does NOT carry the owning `gallery_id`
  (never needed until now — removal from a smart view must route to the owning album).
- `SecureGallery` DTO has no field that isn't server-supplied — a synthetic smart album on
  Android is best modeled as a separate sealed/enum selection state, not a fake
  `SecureGallery` instance (avoids fake ids leaking into `deleteGallery`/`addPhotosToGallery`).

### Invariants the design leans on
1. One photo lives in at most one secure album → the "Secure Gallery" union view has no
   duplicates. Currently UI-enforced only; Phase 0 makes it real.
2. Burst frames are always added to the same album together (web `secureAdd` and Android
   `addPhotosToGallery` add the whole selection to one target; removal is burst-aware) →
   burst collapse works unchanged on aggregated lists.

---

## 2. Design decisions

- **Smart album set + ids** (new namespace, never colliding with main-gallery `smart-*`):
  | id | label | filter |
  |----|-------|--------|
  | `secure-smart-all`    | Secure Gallery | every item |
  | `secure-smart-photos` | Photos | `media_type ∈ {photo, gif}` or NULL (parity with main `smart-photos`; NULL falls back here so nothing vanishes) |
  | `secure-smart-gifs`   | GIFs   | `media_type == gif` |
  | `secure-smart-videos` | Videos | `media_type == video` |
  | `secure-smart-audio`  | Audio  | `media_type == audio` |
- **Visibility**: a tile renders only when its filtered count > 0. With zero secure items
  the whole smart section disappears (existing empty-state untouched).
- **Read-only membership**: smart detail views hide "Add Photos" (no target album) but keep
  per-item Remove, routed to the item's owning `gallery_id`.
- **Ordering**: `added_at DESC` (matches existing per-album order).
- **Data flow**: ONE new server endpoint returning all items across the user's secure
  galleries (token-gated), each item tagged with `gallery_id`. Both clients fetch it once
  after unlock; tiles, counts, covers, and detail views all derive from it client-side —
  same philosophy as `SMART_ALBUM_DEFS`. Rejected alternative: N× per-gallery `listItems`
  aggregation on each client (N+1 requests, logic duplicated in 2 codebases).

---

## 3. Phases

### Phase 0 — Server: enforce the one-secure-album invariant (small, do first)
- [ ] `add_gallery_item` (secure.rs:300): before cloning, check `req.blob_id` (and its
      resolved original/encrypted ids) against `encrypted_gallery_items.blob_id` /
      `original_blob_id` for ALL of the user's galleries. On hit → `AppError::Conflict`
      ("Photo is already in a secure album") — add the variant if `Conflict` doesn't exist
      (409). Log with `[DIAG:SECURE_ADD]` including both gallery ids.
- [ ] Unit/E2E: add same blob twice (second → 409); add to a *second* gallery (→ 409).
- [ ] Web + Android: surface the 409 message verbatim in the existing per-item failure
      paths (web `secureAdd` batch, Android `addPhotosToGallery` outcome counting) — both
      already tolerate per-item failure, so no flow change.

### Phase 1 — Server: aggregate items endpoint
- [ ] New handler `list_all_gallery_items` in `server/src/gallery/secure.rs`:
      `GET /api/galleries/secure/items`. Same `X-Gallery-Token` verification as
      `list_gallery_items` (copy the verify block, no gallery-id ownership check needed —
      scope is `g.user_id = ?`). SQL = existing item SELECT minus the `gallery_id = ?`
      predicate, plus `gi.gallery_id` (and `g.name as gallery_name` for the detail header),
      `JOIN encrypted_galleries g ON g.id = gi.gallery_id WHERE g.user_id = ?`,
      `ORDER BY gi.added_at DESC`.
- [ ] Also add `gallery_id` to the per-gallery `list_gallery_items` JSON (trivial, keeps
      the item shape identical everywhere).
- [ ] Register in `routes.rs` next to the existing secure routes (~line 439). Static
      `/items` vs `/{id}/items` don't collide (different segment count; `blob-ids` vs
      `{id}` precedent already exists).
- [ ] Route must also be reachable in backup-server mode (it's the same router — verify,
      don't assume).
- [ ] Tests: extend `tests/test_06_albums_secure.py` — unlock → add photo+video to two
      albums → `GET /galleries/secure/items` returns both with correct `gallery_id` +
      `media_type`; 401 without token; 401 with garbage token.
      ⚠ test_06 has 8 PRE-EXISTING secure 401 failures on clean dev (see memory
      `e2e-preexisting-failures-2026-07-15`) — baseline before blaming the diff.

### Phase 2 — Web
- [ ] New `web/src/gallery/secureSmartAlbums.ts`: `SECURE_SMART_ALBUM_DEFS`
      (id → label, `filter(item)`), `isSecureSmartAlbum(id)`,
      `computeSecureSmartAlbums(items)` → `[{ id, label, count, coverItem }]` (count>0
      only; cover = newest matching item). Pure, no I/O — unit-test it
      (`secureSmartAlbums.test.ts`): each filter, NULL media_type → Photos + All,
      visibility rule, cover pick, empty input.
- [ ] `web/src/api/galleries.ts`: add `listAllItems(galleryToken)` for
      `/galleries/secure/items`; add `gallery_id: string` to the item type.
- [ ] [SecureGallery.tsx](web/src/pages/SecureGallery.tsx):
  - [ ] After auth, fetch `listAllItems` alongside `loadGalleries`; hold `allItems` state.
        Refresh it wherever `loadGalleries()` is re-called (create/delete/remove/add paths).
        On `isGalleryTokenRejection` → existing `lock()` path; on other failure log +
        setError (no silent catch).
  - [ ] Album-list view: "Smart albums" card row above user albums, rendered from
        `computeSecureSmartAlbums(allItems)`. Cover = thumbnail of `coverItem` (reuse the
        `ThumbnailSource`/`useThumbnailLoader` path from `SecureAlbumCover` — new tiny
        component `SecureSmartAlbumCover` that takes the item directly, NO extra fetch).
        Card click → `navigate(/secure-gallery?album=<smart-id>)` + select synthetic
        gallery `{ id, name: label, item_count: count }`.
  - [ ] Detail view: if `isSecureSmartAlbum(selectedGallery.id)` → items =
        filtered `allItems` (skip `loadItems`); hide "Add Photos"; `handleRemoveItem` uses
        `item.gallery_id` instead of `selectedGallery.id`; after remove, refresh
        `allItems` + `loadGalleries`. Burst collapse/viewer wiring unchanged
        (`secureAlbumId` = smart id round-trips through Viewer's backTo already).
  - [ ] `?album=` restore effect (~line 144): add branch — smart id → synthesize selection
        from `computeSecureSmartAlbums(allItems)` once `allItems` loaded (fixes
        return-from-viewer inside a smart album).
- [ ] Guard: `secure-smart-*` ids must never reach `api.secureGalleries.delete/addItem`
      — delete button only renders for real albums (smart cards get no trashcan).

### Phase 3 — Android
- [ ] DTO: `SecureGalleryItem` += `@SerializedName("gallery_id") val galleryId: String? = null`.
      New `ApiService` route `@GET("api/galleries/secure/items")` (same `X-Gallery-Token`
      header pattern as the per-gallery call) + `SecureGalleryRepository.listAllItems(token)`.
- [ ] New pure object `SecureSmartAlbums` (e.g. `ui/screens/securegallery/SecureSmartAlbums.kt`
      or `data/` beside `SecureExclusion.kt`): same defs/compute as web — id, label,
      `matches(item)`, `compute(items)` with count>0 visibility. Unit test it
      (`SecureSmartAlbumsTest.kt`, mirror `SecureExclusionTest` style): NULL media_type,
      per-type filters, visibility, cover selection.
- [ ] `SecureGalleryViewModel`:
  - [ ] `allItems` state; load after `unlock()`/`loadGalleries()`; refresh after
        `addPhotosToGallery`, `removeItems`, `deleteGallery`. Log failures (`Log.e`).
  - [ ] Smart selection: `selectSmartAlbum(id)` — sets a `selectedSmartAlbumId` (separate
        state, NOT a fake `SecureGallery`), `items = SecureSmartAlbums.filter(allItems, id)`,
        does NOT call `loadPhotos()` (no picker in smart views).
  - [ ] `removeItems`: gallery id per item — `item.galleryId ?: selectedGallery.id`; keep
        burst-expansion logic (frames share an album, but scope the burst-sibling scan to
        the same `galleryId` for correctness).
- [ ] `GalleryListView`: smart-album cards section above user albums (only non-empty).
      Cover: new lightweight composable that renders a thumb from an already-known item
      (`downloadThumb(item.blobId, item.encryptedThumbBlobId)`) — do NOT reuse
      `GalleryCoverThumbnail` (it re-fetches listItems). No delete affordance on smart cards.
- [ ] `SecureGalleryScreen`: branch on `selectedSmartAlbumId` → `GalleryDetailView` with
      smart items, `readOnlyMembership = true` (hides Add Photos), title = label.
- [ ] `GalleryDetailView`: accept the read-only flag + item-level remove routing; viewer
      hand-off unchanged (items already carry everything the secure viewer needs).

### Phase 4 — Verify (not optional)
- [ ] `cargo test --bin simple-photos-server` (crate is BINARY — see memory), `cargo clippy`.
- [ ] Web: `npm run build` + vitest green (new `secureSmartAlbums.test.ts`).
- [ ] Android: `gradlew test` green (new `SecureSmartAlbumsTest`).
- [ ] E2E: `test_06_albums_secure.py` — new aggregate-endpoint + 409-dup cases green;
      pre-existing failures unchanged vs baseline.
- [ ] Manual/device checklist:
  - [ ] Web: unlock → smart tiles show only non-empty types; open GIFs → only gifs; open
        photo from Videos smart album → close viewer → back INSIDE the smart album
        (`?album=secure-smart-videos` restore); remove item from smart view → it leaves the
        owning real album and returns to main gallery; empty type → tile gone after refresh.
  - [ ] Web backup-server mode: smart tiles render, no Add/Remove/Delete affordances.
  - [ ] Android (S21 harness, `.device-test\dev.ps1`): same pass; video plays from smart
        Videos view; burst stack removes whole burst from smart view.
  - [ ] Dup guard: attempt API double-add → 409, client shows per-item failure message.

---

## Out of scope (noted during audit, don't sneak in)
- Refactoring web `SecureAlbumCover` / Android `fetchGalleryCover` N+1 cover fetches onto
  the aggregate response — worthwhile follow-up, separate change.
- Favorites/Recently-Added secure smart albums — secure items don't sync favorite state;
  user didn't ask.
- Any Dexie/Room schema change — none needed; everything derives from the server response.
