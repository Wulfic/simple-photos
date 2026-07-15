# TODO — Album photo counts: slow on web, unstable on Android (investigated 2026-07-15)

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

- [ ] **Android: store manifest membership verbatim.** Add a
  `photoBlobIds: List<String>` (JSON TypeConverter) column to `AlbumEntity`
  (Room migration). `syncAlbumsFromServer` writes the decrypted list as-is;
  xrefs remain a *derived* view for detail-grid queries. Unsynced members are
  no longer lost.
- [ ] **Android: build uploads from the stored list, never from the mirror.**
  `syncAlbum()` manifest payload = stored `photoBlobIds` (plus/minus explicit
  user add/removes), NOT `getPhotoIdsForAlbum → serverBlobId`. A
  partially-synced device can then never shrink a server manifest.
- [ ] **Both platforms: upload-then-delete manifest replacement.** Upload new
  manifest → persist new blob id locally → best-effort delete old blob.
  Web `takeoutAlbums.ts::saveAlbumManifest`, Android `AlbumRepository.syncAlbum`
  + `deleteAlbum`. (Supersedes/absorbs TODO.md Phase 3 bullet.)
- [ ] **Both platforms: skip unchanged manifests.** Blob ids are immutable ⇒
  if a listed blob id equals the locally stored `manifestBlobId` /
  `serverManifestBlobId`, skip download + decrypt + xref rewrite entirely.
  This makes the steady-state ON_RESUME / page-mount sync a no-op (one
  `listBlobs` call) and kills most Room Flow churn as a side effect.
- [ ] **Tests:** manifest round-trip with members missing from the mirror
  (must survive); shrink-regression test (sync down manifest with N ids while
  mirror holds N−k → re-upload still contains N); unchanged-blob-id
  short-circuit does zero downloads.

### Phase 2 — Android count stability

- [ ] Wrap each album's xref rewrite (`deleteAllXRefsForAlbum` + inserts) in a
  single Room `@Transaction` DAO method so observers never see the half-empty
  state. (Mostly moot after the Phase 1 short-circuit, but required for the
  cases that do rewrite.)
- [ ] Sequence `refresh()`: secure-ids fetch → server sync → Takeout pass →
  ONE count recompute at the end. Remove the `albums.first()` pre-sync
  snapshot race; let `LaunchedEffect(albums)` remain the only other trigger.
- [ ] Compute counts from the new stored `photoBlobIds ∩ mirror − secure`
  (same predicate as web's `countRegularAlbum`) and only publish
  `albumCounts` when the map actually changed (value-equality guard) so
  Compose doesn't re-render identical numbers.
- [ ] Persist the last computed count per album (column `cachedCount` on
  `AlbumEntity`) so a cold start renders the previous stable number instantly
  instead of 0 → summary → local churn.
- [ ] **Tests:** count unchanged across a full refresh with no server-side
  changes; secure-exclusion; no intermediate emission during a manifest
  rewrite (Room transaction test).

### Phase 3 — Web: cached counts + stop the full-table scans

- [ ] **Persist `cachedCount` on `CachedAlbum`** (Dexie schema bump). Update
  it wherever resolution already happens (`useAlbumPhotos`,
  `countRegularAlbum` call sites, `takeoutAlbums` writes). Albums tiles
  render `cachedCount` immediately and reconcile in the background —
  perceived "counting on open" disappears.
- [ ] **Split thumbnails out of the `photos` rows** (new Dexie table
  `thumbs {blobId, data, mime}` + migration copying existing bytes).
  Membership/count queries then hydrate lean metadata rows only; the grid
  fetches thumbs per-tile (most tile components already go through helpers,
  so the touch points are bounded: `usePhotoSync` writer, tile/cover readers).
  This is the single biggest perf lever for large libraries — counting and
  album-open stop deserializing every thumbnail in the library.
  (Fallback if the migration is deemed too risky for one pass: keep bytes but
  switch count paths to `db.photos.orderBy("takenAt").primaryKeys()` +
  a lean index; still fixes counting, not album-open.)
- [ ] **`loadAlbums()` short-circuit** (same as Phase 1 bullet): only
  download/decrypt manifests whose blob id isn't already in `db.albums`;
  stale-album cleanup can run off the listing alone.
- [ ] **Tests:** `countRegularAlbum` + cachedCount reconciliation; Dexie
  migration test (thumbs copied, photos still resolve); manifest
  short-circuit fetch-count assertion.

### Phase 4 — kill the re-materialization churn

- [ ] Persist the Takeout "fully materialized" latch (web: `localStorage`
  keyed by server + user; Android: DataStore) including the
  `photosUnmatched`-stable heuristic from TODO.md, instead of per-session
  refs that reset on every mount/process. Reconstruction then runs only when
  the source-albums response or mirror size actually changed.
- [ ] Re-check the 5 s secure polling interval after Phases 2–3; with cheap
  counting it can stay, otherwise widen to 30 s + refresh-on-focus.

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

- Unit: web `npm test` (useAlbumPhotos/countRegularAlbum/migration), Android
  JVM tests for repository logic; `cargo test --bin simple-photos-server`
  untouched (no server changes expected).
- E2E churn test: simulate two clients with disjoint partial mirrors
  alternately syncing — assert server manifest membership is monotonically
  the union and blob ids stop churning after both settle.
- Device: open Albums on Android, background/foreground 5×, assert badge
  values never change absent real edits; web Albums page open on a >5k
  library renders badges instantly (cachedCount) with no long task in the
  Performance tab.
