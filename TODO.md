# TODO — Google Photos Takeout album fidelity (investigated 2026-07-15)

**Symptom:** Takeout imports do not faithfully recreate Google Photos albums —
albums come out missing, partially populated, or wrongly named.

## How album recreation works today (for context)

1. **Server captures membership at import** into `photo_source_albums`
   (migration 027), keyed by `photo_id` + folder-derived `album_name`. Two
   writers, both routed through the shared resolver `server/src/import/sidecar.rs`
   (with the `is_takeout` gate so plain user folders never become albums):
   - `POST /admin/import/google-photos` → `import::takeout::import_takeout`
   - the auto-scan register path → `photos/scan.rs` walk → `photos/register.rs::register_native_file`
2. **Clients materialize E2E album manifests** from `GET /api/photos/source-albums`:
   web `web/src/utils/takeoutAlbums.ts` (debounced trigger in `pages/Albums.tsx`),
   Android `AlbumRepository.recreateAlbumsFromServer`. Both bridge
   `photo_id → blobId` via the synced local mirror and derive the deterministic
   album id `"src-" + sha256(source + " " + name)` so platforms converge.

## Root causes found (ranked by impact)

### 1. PRIMARY — no backfill path for photos imported before the Jul 4 album-capture fixes
- `photos/scan.rs:135`: any file whose `rel_path` is already in `photos`/`trash_items`
  is skipped **before** the album-recording code in `register_native_file` can run.
  So re-running a scan/ingest never backfills `photo_source_albums` for the
  existing library.
- The hash-duplicate backfill added Jul 4 (`register.rs` ~264–295) only fires for a
  *new physical copy at a new path* — not for files already registered by path.
- The one endpoint that DOES backfill existing photos by content hash
  (`import_takeout` — its album-recording deliberately runs for deduped/existing
  photos) is **unreachable from the UI**: zero callers of
  `/admin/import/google-photos` in `web/src` (the Import page only calls
  `/admin/import/ingest`).
- Net effect: everything imported before the Jul 4 fixes (i.e. the whole live
  library on CT132) has sparse/absent `photo_source_albums` rows → client
  reconstruction builds partial or empty albums. **This alone explains the symptom.**
- Note: those Jul 4 fixes live on `dev`; the live deployment may also predate them.

### 2. Web "Local Upload" import captures no albums at all
- `web/src/pages/Import.tsx` (local mode): file picker / drag-drop flatten folder
  structure (no `webkitdirectory` / `webkitGetAsEntry`), `/api/photos/upload` has
  no album field, and nothing server-side writes `photo_source_albums` for
  uploads. Any Takeout imported through the browser loses 100% of album data.

### 3. Album names are Takeout's sanitized folder names, not the real titles
- The true album title lives in the album-level `metadata.json` (`"title"`).
  `sidecar.rs::is_photo_sidecar` correctly rejects it as a photo sidecar — but
  then nobody reads it. `derive_album_from_dir` uses the raw folder name, which
  Takeout mangles (special characters → `_`, length truncation, `(1)` counters).

### 4. User curation is overridden — zombie albums and members
- No tombstone mechanism anywhere. Deleting a Takeout-derived album
  (web `RegularAlbumView.deleteAlbum`, Android `AlbumRepository.deleteAlbum`) is
  undone the next time reconstruction runs (fresh session → refs reset → album
  recreated). Removing individual photos is undone too: the merge is a pure
  union (`takeoutAlbums.ts:100`).

### 5. Manifest replace is delete-before-upload — data-loss window (both platforms)
- Web `takeoutAlbums.ts::saveAlbumManifest` and Android `AlbumRepository`
  (~line 90 delete, line 107 upload) delete the old manifest blob **first**, then
  encrypt+upload the new one. A failure in between leaves no manifest server-side
  → the album silently vanishes from every other device.

### 6. Minor issues
- Re-running `import_takeout` inserts **duplicate `photo_metadata` rows** every
  run (the metadata insert has no existence check, unlike the photo insert).
- `scan_takeout` (the scan *report*) uses naive pairing instead of the shared
  resolver — undercounts pairs (misses duplicate-counter/truncated names),
  making the pre-import report misleading.
- `fullyMaterializedRef` in `Albums.tsx` never latches when some photos will
  never sync (moved to secure gallery / trashed) → reconstruction re-runs every
  session; cheap but wasteful.
- Stale doc comment in `sidecar.rs` pointing at a web allowlist in
  `takeoutAlbums.ts` that no longer exists there.

## Plan

### Phase 1 — restore membership for the already-imported library (fixes the symptom)
- [ ] **Server: dedicated album backfill endpoint** —
  `POST /admin/import/google-photos/backfill-albums { path }`. Re-walk the
  Takeout tree with the existing `TakeoutDirContext`, match media files to
  existing photos by `photo_hash` (fallback filename+size — same dedup keys as
  `import_takeout`), `INSERT OR IGNORE` into `photo_source_albums`. No photo or
  metadata inserts, so it is cheap and cannot duplicate anything. Factor the
  membership-recording block out of `import_takeout` and share it.
  (Rejected alternative: "just re-run `import_takeout`" — would work for
  membership but currently duplicates `photo_metadata` rows every run; fix that
  either way, see Phase 3.)
- [ ] **Web: expose it** — button in Import page (server mode), e.g. "Rebuild
  Takeout albums", reusing the path input; report `albums_recorded`.
- [ ] **Tests**: backfill matches an existing plain photo, an encrypted photo
  (photo_hash survives encryption), skips non-Takeout folders (`is_takeout`
  gate), is idempotent on re-run.
- [ ] **Ops**: deploy to CT132 and run against the Takeout directory (TEMP mount),
  then let clients re-materialize; verify album counts vs Google Photos.

### Phase 2 — capture faithfully at import time
- [ ] **Real album titles**: when recording membership (import + backfill), read
  the album folder's `metadata.json` `"title"`. Keep the folder name as the
  identity key (so the deterministic album id doesn't churn and duplicate
  existing albums) and carry the title as a display name — new column
  `photo_source_albums.album_title` + expose in `/api/photos/source-albums`;
  clients use title for the manifest `name`, key stays folder-based.
- [ ] **Web Local Upload albums** (lower priority — server-directory import is the
  primary path): capture folder structure via `webkitdirectory` /
  `webkitGetAsEntry` relative paths, filter with the same non-album rules as
  `is_non_album_folder`, send an `X-Source-Album` header on `/api/photos/upload`,
  and record it server-side (sanitized).

### Phase 3 — respect curation + robustness
- [ ] **Tombstones**: server table `dismissed_source_albums (user_id, source,
  album_name)`; clients write a tombstone when the user deletes a
  Takeout-derived (`src-…` id) album; reconstruction skips dismissed albums.
  Album names are already plaintext server-side in `photo_source_albums`, so no
  E2E regression. (Per-photo removal tombstones: follow-up, needs design —
  blob-id keyed removals in the manifest itself may be simpler.)
- [ ] **Fix manifest replace ordering on BOTH platforms**: upload new manifest →
  persist new blob id locally → best-effort delete the old blob. Never
  delete-first.
- [ ] **Minor cleanups**: existence-check before `photo_metadata` insert in
  `import_takeout`; route `scan_takeout` pairing through the shared resolver;
  latch `fullyMaterializedRef` when `photosUnmatched` is stable across runs;
  fix the stale allowlist comment in `sidecar.rs`.

### Verification (end of each phase)
- Unit: `cargo test --bin simple-photos-server` (crate is a binary) for
  backfill/title/resolver tests; web `npm test` for takeoutAlbums changes.
- E2E: import a synthetic Takeout fixture (album folders + `metadata.json` +
  duplicate copies in `Photos from YYYY` + `-edited` pairs + `(1)` counters),
  assert `photo_source_albums` contents, then assert client manifests match.
- Live: CT132 backfill run; album count + membership spot-check vs Google Photos.
