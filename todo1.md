# TODO — Album photo counts: slow on web, unstable on Android (investigated 2026-07-15)

> **Status (2026-07-16): all four phases implemented; device verification still
> outstanding.** Unit tests are green on both platforms (`npm test` 106 passing,
> `gradlew testDebugUnitTest` passing). No server changes were needed, as
> predicted. The E2E churn test and the on-device checks under "Verification"
> below are the remaining work — they need two real clients and a real library,
> so they can't be closed from the code side.
>
> One deliberate deviation from the plan, in Phase 3: the thumbnail split does
> **not** copy bytes inside the Dexie upgrade. That would put hundreds of MB in
> one IndexedDB transaction and leave a DB that won't open if it throws part-way.
> The v10 upgrade is schema-only and `db/thumbs.ts::backfillThumbs` moves the
> bytes chunked, in the background, with `resolveThumb` still reading the legacy
> field until each row's turn comes.

**Symptoms reported:**
1. Web: counts aren't cached — opening the Albums page / an album visibly
   re-counts the items every time.
2. Android: tiles show a count, but it keeps changing every time the app
   checks in with the server.
3. Counts appear to depend on what each client thinks is in the album — "it's
   almost like each client forces the server to recreate the album structure
   each time."

All three are real, and observation 3 is literally true — see Finding 3.

## Architecture constraint (why counts are client-computed at all)

Regular albums are E2E-encrypted manifests (`blob_type = "album_manifest"`);
the server only ever holds ciphertext and cannot know membership or counts
(server-side resolution was investigated and rejected as infeasible — see the
`useAlbumPhotos.ts` header comment). So every count on every platform is
computed as:

```
count = manifest.photo_blob_ids ∩ local-photo-mirror − secure-gallery-blobs
```

That formula is correct and intentional (#12/#16/#20 history). The bugs are in
**how and when** each platform evaluates it, and — worst — in Android
**writing its partial view of membership back to the server**.

---

## Findings

### Finding 1 — Web: counts are recomputed from full-table scans, never persisted

- `web/src/pages/Albums.tsx:94` — `db.photos.toArray()` loads the **entire**
  photo mirror to compute smart-album + regular-album badge counts.
- `web/src/hooks/useAlbumPhotos.ts:142-145` — opening any album detail loads
  the entire mirror again (`db.photos.orderBy("takenAt").reverse().toArray()`).
- The killer: `CachedPhoto.thumbnailData: ArrayBuffer` lives **in the same
  Dexie row** (`web/src/db/index.ts:27`). Both queries above deserialize every
  thumbnail in the library (thousands of ArrayBuffers, hundreds of MB of
  structured-clone work) just to produce an integer. This is the "it has to
  count the items when opening" lag.
- `Albums.tsx` `loadAlbums()` (:362-401) additionally **downloads and decrypts
  every `album_manifest` blob on every mount**. Manifest blobs are immutable
  (an update is delete + upload of a *new* blob id), and we already store
  `manifestBlobId` locally — so an unchanged blob id proves the cached
  manifest is current, yet there is no short-circuit.
- Cold-cache wobble: `countRegularAlbum()` (`useAlbumPhotos.ts:110-122`) falls
  back to raw `photoBlobIds.length` until the mirror has rows, then switches
  to the intersected count → the badge changes value once sync fills in.
- No count is ever persisted anywhere (no field on `CachedAlbum`), so all of
  the above re-runs from scratch on every navigation.

### Finding 2 — Android: every ON_RESUME destructively rewrites membership, and counts are read mid-rewrite

`AlbumListScreen.kt:84` calls `viewModel.refresh()` on **every** ON_RESUME.
`AlbumViewModel.refresh()` (:143-181) then launches **two racing coroutines**:

- Job A: `syncAlbumsFromServer()` → Takeout `recreateAlbumsFromServer()`.
- Job B: fetch secure blob ids → `loadSmartAlbumCounts()` +
  `loadCoverPhotos(albums.first())` — a snapshot taken *before* Job A lands.

Inside `AlbumRepository.syncAlbumsFromServer()` (:124-216):

- It re-downloads and decrypts **every** manifest blob every time — same
  missing blob-id short-circuit as web (we store `serverManifestBlobId` and
  blobs are immutable, so unchanged id ⇒ nothing to do).
- For each album it runs `deleteAllXRefsForAlbum()` then re-inserts xrefs one
  by one (:192-200) — **not in a transaction**. Every one of those writes fires
  Room's InvalidationTracker → the `albums`/xref Flows emit →
  `LaunchedEffect(albums)` re-runs `loadCoverPhotos()` → counts get recomputed
  **between the delete-all and the re-insert**, so the badge transiently shows
  0/partial and then settles. This is the visible "count keeps changing when
  it checks in with the server."
- The xref rebuild only inserts members whose `serverBlobId` already exists in
  the local Room mirror (:194). Android has **no storage for the raw blobId
  list** (`AlbumEntity` has no `photoBlobIds`; xrefs are the only membership
  store) — so members not yet synced locally are **silently dropped from
  local membership**.
- Secure-count shift: `secureBlobIds` starts as `emptySet()` (:68), so the
  first count pass after cold start is secure-inclusive, a later pass
  secure-exclusive → another visible count change.

### Finding 3 — The cross-device ping-pong: Android uploads its truncated membership back to the server

This is the root of "each client forces the server to recreate the album
structure" and of counts depending on which device looked last:

1. `AlbumRepository.syncAlbum()` (:77-116) **rebuilds the manifest from local
   xrefs** (`getPhotoIdsForAlbum` → mirror lookup) and replaces the server
   blob — delete old first (:90), upload new (:107). After Finding 2's
   truncation, any member not yet in Android's mirror is *gone from the
   uploaded manifest*.
2. `recreateAlbumsFromServer()` (:244-328) calls `syncAlbum()` for every album
   where it matched at least one new Takeout photo — so a partially-synced
   device routinely re-uploads shrunken manifests.
3. Web's Takeout pass (`web/src/utils/takeoutAlbums.ts:100`) then union-merges
   its own matched set back into the shrunken manifest and re-uploads (new
   blob id, delete-first at :152-156).
4. New blob id ⇒ Android's next ON_RESUME re-downloads that manifest, rewrites
   xrefs (dropping whatever *it* hasn't synced), possibly re-uploads again…

Membership genuinely oscillates server-side between each client's
locally-visible subset. Counts differ per device and per check-in **because
the underlying manifests really are being rewritten by each client**. The
delete-before-upload ordering also leaves a data-loss window (already noted as
TODO.md Phase 3, but it belongs here too since both loops hammer it).

### Finding 4 — Churn amplifiers

- Takeout reconstruction latches (`fullyMaterializedRef` in `Albums.tsx:139`,
  `takeoutFullyMaterialized` in `AlbumViewModel.kt:109`) are in-memory only —
  every new session/page-mount re-runs reconstruction against a possibly
  still-syncing mirror (also noted in TODO.md "minor issues").
- Web polls secure blob ids every 5 s (`useSecureBlobFilter.ts:48`); harmless
  by itself (set-equality guarded) but it means any secure change re-derives
  every count — fine once counting is cheap, painful today.

---

## Plan

### Phase 1 — stop the manifest ping-pong (correctness; do this first, it's the actual data bug)

- [x] **Android: store manifest membership verbatim.** `AlbumEntity.photoBlobIds`
  (`List<String>` via `data/local/Converters.kt`, JSON — blob ids are opaque, so
  a delimiter would be an assumption about their alphabet). DB version 10 → 11;
  the existing `fallbackToDestructiveMigration` handles it (the DB is a cache).
  `syncAlbumsFromServer` writes the decrypted list as-is; xrefs are now a
  *derived* projection (`reconcileXRefs`). Unsynced members are no longer lost.
- [x] **Android: build uploads from the stored list, never from the mirror.**
  `AlbumManifest.payloadFor(album, cover)` takes `album.photoBlobIds` and nothing
  else; `syncAlbum` re-reads the entity first, because callers routinely hold one
  captured before their own edit landed. `addPhotosToAlbum`/`removePhotosFromAlbum`
  maintain the stored list (batched — per-photo writes fired one Room
  invalidation each). Also fixed: `GalleryViewModel` added photos to albums
  without ever uploading a manifest.
- [x] **Both platforms: upload-then-delete manifest replacement.** Already landed
  with the uncommitted Takeout work — web `utils/albumManifest.ts::saveAlbumManifest`
  (the single writer for all 5 call sites), Android `AlbumRepository.syncAlbum`.
- [x] **Both platforms: skip unchanged manifests.** Android `syncAlbumsFromServer`
  short-circuits on `getByManifestBlobId`; web `loadAlbums()` indexes
  `db.albums` by `manifestBlobId` and skips matches. The xref projection tracks
  the *mirror*, not the manifest, so it can't ride on the same check — it's gated
  separately on `AlbumRepository.xrefMirrorSize`, which is what keeps the
  steady-state resume to one `listBlobs` call.
- [x] **Tests:** `AlbumManifestTest` — round-trip with members missing from the
  mirror (`keeps members that are missing from the local mirror`), shrink
  regression (`a partially-synced device cannot shrink an album`: N=5 manifest
  onto a 2-photo mirror → re-upload still carries 5), and cross-platform format
  parity in both directions. `ConvertersTest` covers ids containing separator
  characters.

### Phase 2 — Android count stability

- [x] xref rewrites run inside `db.withTransaction { }` (room-ktx) rather than a
  `@Transaction` DAO method — Room defers invalidation to commit either way, and
  this avoids converting the DAO interface to an abstract class. `reconcileXRefs`
  also diffs first and writes nothing when the projection already matches, so the
  common case emits nothing at all.
- [x] `refresh()` is now one strictly ordered coroutine: secure ids → server sync
  → Takeout pass → one count recompute. The `albums.first()` snapshot is read
  *after* the sync instead of racing it.
- [x] Counts come from `AlbumRepository.visibleMemberCount(photoBlobIds, mirror,
  secure)` — the same predicate as web's `countRegularAlbum`. No value-equality
  guard was needed: `mutableStateOf` already compares structurally, so
  re-publishing an identical map is a no-op for Compose. The guard that *did*
  matter is on the DB write (`AlbumDao.updateCachedCount`'s `AND cachedCount !=
  :count`), without which the write would invalidate the albums Flow and loop.
- [x] `AlbumEntity.cachedCount` persists the last count; `AlbumListScreen` renders
  `albumCounts[id] ?: album.cachedCount`. `loadCoverPhotos` returns early on an
  empty mirror so a cold start can't overwrite good counts with zeros.
- [x] **Tests:** `VisibleMemberCountTest` — stability across repeated recomputes,
  secure exclusion, unsynced members, and parity with web's suite. The Room
  transaction ordering is structural (Room's own guarantee) rather than
  unit-tested; observing intermediate emissions needs an instrumented test.

### Phase 3 — Web: cached counts + stop the full-table scans

- [x] **`cachedCount` on `CachedAlbum`** (Dexie v10). `countRegularAlbum` prefers
  it over the raw manifest size while the mirror is cold; `reconcileAlbumCount`
  (guarded against the write→live-query→recount loop) is called from
  `useAlbumPhotos` for the open album and from `Albums.tsx` for every tile.
- [x] **Thumbnails split into `thumbs {blobId, data, mime}`** — see the status
  note at the top for why the copy is a background backfill rather than a Dexie
  upgrade function. `db/thumbs.ts` is now the only door: `putThumb` (writer),
  `resolveThumb`/`getThumb` (readers, legacy-aware), `copyThumb`, `deleteThumbs`,
  `backfillThumbs`. `useThumbnailLoader` reads the table itself, so
  `ThumbnailSource` carries ids only — passing bytes through it was what made
  every list hydrate the whole library before rendering one tile.
  `thumbnailMimeType` deliberately stays on the photo row: it's a short string,
  and the tile needs it synchronously to decide GIF autoplay.
- [x] **`loadAlbums()` short-circuit** — done, see Phase 1.
- [x] **Tests:** `useAlbumPhotos.test.ts` extended with cachedCount preference,
  live-count override, and the persist→cold-start round trip. `db/thumbs.test.ts`
  (23 tests) runs against real in-memory IndexedDB via `fake-indexeddb`
  (new devDependency) and covers the backfill: bytes moved, rows leaned, other
  fields preserved, idempotent, resumable after interruption, and >1 chunk.
  The short-circuit's "zero downloads" is asserted structurally (the cached
  branch `continue`s before any `api.blobs.download`) — a fetch-count assertion
  would need the whole api client mocked, which these suites don't set up.

### Phase 4 — kill the re-materialization churn

- [x] The latch is persisted as **the mirror size at the moment a pass settled**,
  not a bare "done" flag — that's what makes it self-healing (the instant the
  mirror grows, the recorded size no longer matches and reconstruction runs
  again). Web: `utils/takeoutLatch.ts` (localStorage, keyed by user — the origin
  already scopes it to the server; cleared on logout, since logout wipes the
  mirror the latch describes). Android: DataStore int key, cleared with every
  other preference on logout. The `photosUnmatched`-stable heuristic is now a
  shared, tested rule on both platforms (`takeoutSettled`), where before only web
  had it.
- [x] Re-checked the 5 s secure poll: left at 5 s. It was only ever painful
  because each change re-derived every count over fat rows; with `cachedCount`
  and lean rows the recount is trivial, and widening it would delay reflecting a
  photo secured on another device for no benefit.

### Explicit non-goals

- **No server-side count.** Membership is E2E; publishing per-album counts
  server-side would leak metadata for zero benefit once counts are cached
  client-side. (A count field inside the encrypted manifest is redundant —
  it's derivable from `photo_blob_ids.length` after decrypt.)
- TODO.md's Takeout *fidelity* plan (backfill, titles, tombstones) stays
  separate — this file is about count computation/stability. The two overlap
  only on the manifest upload-then-delete fix and the materialization latch;
  implement those once, here (Phase 1 / Phase 4).

### Verification (end state)

- [x] Unit: web `npm test` — 106 passing (useAlbumPhotos/countRegularAlbum,
  thumbs backfill, takeoutLatch, takeoutSettled). `npx tsc -b` and
  `npm run build` clean. Android `gradlew testDebugUnitTest` passing, including
  23 new tests. No server changes were needed, so `cargo test` is untouched.
- [ ] E2E churn test: simulate two clients with disjoint partial mirrors
  alternately syncing — assert server manifest membership is monotonically
  the union and blob ids stop churning after both settle.
- [ ] Device: open Albums on Android, background/foreground 5×, assert badge
  values never change absent real edits; web Albums page open on a >5k
  library renders badges instantly (cachedCount) with no long task in the
  Performance tab.
- [ ] Device: watch the first web load on the real library — `backfillThumbs`
  drains in the background, so confirm the gallery stays responsive while it
  runs and that thumbnails keep rendering throughout (the legacy read path is
  what should make that invisible).
