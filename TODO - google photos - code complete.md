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
**Code complete on `dev` (not committed, not deployed). Only the CT132 ops run is left.**
- [x] **Server: dedicated album backfill endpoint** —
  `POST /admin/import/google-photos/backfill-albums { path }`. Re-walk the
  Takeout tree with the existing `TakeoutDirContext`, match media files to
  existing photos by `photo_hash` (fallback filename+size — same dedup keys as
  `import_takeout`), `INSERT OR IGNORE` into `photo_source_albums`. No photo or
  metadata inserts, so it is cheap and cannot duplicate anything. Factor the
  membership-recording block out of `import_takeout` and share it.
  (Rejected alternative: "just re-run `import_takeout`" — would work for
  membership but currently duplicates `photo_metadata` rows every run; fix that
  either way, see Phase 3.)
  Landed as `takeout.rs::backfill_takeout_albums`. Three things are now shared
  rather than re-implemented: `walk_takeout_tree` (one walk, one resolver — the
  bug class behind minor issue #6), `find_existing_photo` (the dedup keys), and
  `record_source_album` (moved out of `register.rs`; now returns whether a row
  was actually inserted so the count is real). Only files in a genuine album
  folder are hashed, so the `Photos from YYYY` copy of the whole library costs
  nothing.
- [x] **Web: expose it** — button in Import page (server mode), e.g. "Rebuild
  Takeout albums", reusing the path input; report `albums_recorded`.
  "Rebuild Takeout Albums" in the Server Directory card + `api.admin.backfillTakeoutAlbums`.
- [x] **Tests**: backfill matches an existing plain photo, an encrypted photo
  (photo_hash survives encryption), skips non-Takeout folders (`is_takeout`
  gate), is idempotent on re-run.
  5 unit tests in `takeout.rs` (server suite 239 → 244, green) + 7 E2E in
  `tests/test_87_takeout_album_backfill.py`, which seeds a library via
  `/photos/upload` (records no albums — the real pre-fix state), asserts the
  albums are absent, then asserts the backfill recovers them through
  `/api/photos/source-albums`. Verified: encryption only sets `encrypted_blob_id`
  and never touches `photos.photo_hash`, so hash-matching works on the live
  encrypted library.
- [ ] **Ops**: deploy to CT132 and run against the Takeout directory (TEMP mount),
  then let clients re-materialize; verify album counts vs Google Photos.
  - Endpoint is **synchronous** (like `import_takeout`): it hashes every file in
    an album folder, so the CT132 run may take minutes and hold the request open.
    If it times out in the browser, that's the first thing to revisit (make it
    fire-and-forget + a status endpoint) — the work itself is idempotent, so a
    timed-out run can simply be re-run.
  - `photos_unmatched` in the response is the real gap metric: album files with
    no photo row at all. Expect it to be > 0 for anything trashed or moved to the
    secure gallery.

### Phase 2 — capture faithfully at import time
**Code complete on `dev` (not committed, not deployed).**
- [x] **Real album titles**: when recording membership (import + backfill), read
  the album folder's `metadata.json` `"title"`. Keep the folder name as the
  identity key (so the deterministic album id doesn't churn and duplicate
  existing albums) and carry the title as a display name — new column
  `photo_source_albums.album_title` + expose in `/api/photos/source-albums`;
  clients use title for the manifest `name`, key stays folder-based.
  Landed as `sidecar.rs::parse_album_title` + `TakeoutDirContext::resolve_album_title`
  (one read per *album* directory, so the `Photos from YYYY` copy of the library
  still costs nothing), migration 029, and `SourceAlbum.title`. All four writers
  (import, backfill, scan/autoscan register, upload) go through the shared
  `record_source_album`, which now takes the title.
  Two things worth remembering:
  - `INSERT OR IGNORE` alone would have made titles unreachable forever for every
    membership already recorded (i.e. the whole live library, once Phase 1 runs).
    `record_source_album` therefore returns `Inserted | TitleUpdated | Unchanged`
    and applies the title as a second narrow UPDATE that never blanks a known
    title. Backfill reports `albums_retitled` (distinct albums, not rows).
  - Clients rename an album ONLY when it still carries the raw folder name —
    i.e. a name we wrote. A user's own rename is left alone. The rule is pure and
    duplicated by necessity across platforms (`resolveAlbumDisplayName` in
    `web/src/utils/takeoutAlbums.ts` and `AlbumRepository`), so it is pinned by
    case-for-case identical tests on both sides; they must not drift apart or the
    two devices will rename each other's albums forever.
- [x] **Web Local Upload albums** (lower priority — server-directory import is the
  primary path): capture folder structure via `webkitdirectory` /
  `webkitGetAsEntry` relative paths, filter with the same non-album rules as
  `is_non_album_folder`, send an `X-Source-Album` header on `/api/photos/upload`,
  and record it server-side (sanitized).
  Landed as `web/src/utils/pickedFiles.ts` (folder picker + a drop-entry walk that
  loops `readEntries` — it caps at ~100 per call and would silently truncate any
  real Takeout folder) and `web/src/utils/uploadAlbums.ts` (the `is_takeout` /
  non-album rules mirrored from `sidecar.rs`, tested case-for-case against the
  Rust tests). Headers are percent-encoded: `fetch` throws outright on a
  non-Latin-1 header value, so an album called "東京 2019" would have failed the
  upload entirely. Server-side (`photos/upload.rs`) re-checks the non-album rules
  rather than trusting the client, and records membership on the hash-dedup
  branch too — for Takeout that branch is where most memberships arrive, since
  the same bytes ship in the date folder and every album folder.
  Also: an upload carrying `X-Source-Album` is no longer eligible for deferred
  conversion, since deferring returns before a photo row exists to attach the
  membership to.
- [x] **Tests**: server 244 → 258 (title parse/resolve incl. the photo-sidecar
  guard, backfill title + title-repair-on-re-run, `record_source_album` never
  blanking a known title, upload header decode/sanitise/reject). Web +18
  (`takeoutAlbums.test.ts`, `uploadAlbums.test.ts`). Android +6
  (`AlbumDisplayNameTest`, the web parity mirror). E2E: `test_87` +1 (title read,
  keyed by folder name, null for a metadata-less album) and a new
  `test_88_upload_source_albums.py` (12) covering the upload contract end to end.

### Phase 3 — respect curation + robustness
**Code complete on `dev` (not committed, not deployed).**
- [x] **Tombstones**: server table `dismissed_source_albums (user_id, source,
  album_name)`; clients write a tombstone when the user deletes a
  Takeout-derived (`src-…` id) album; reconstruction skips dismissed albums.
  Album names are already plaintext server-side in `photo_source_albums`, so no
  E2E regression. (Per-photo removal tombstones: follow-up, needs design —
  blob-id keyed removals in the manifest itself may be simpler.)
  Landed as migration 030 + `POST /api/photos/source-albums/dismiss`. Two
  decisions worth keeping:
  - **`list_source_albums` filters dismissed albums server-side**, so both
    clients honour a deletion without either knowing tombstones exist. That is
    also why a device that never saw the delete still respects it.
  - **The client identifies the album by its `src-…` id, not by name.** After
    Phase 2 a client's album is displayed under its *title*, and it never stored
    the Takeout folder name — so it cannot name the identity to tombstone. The
    server instead recomputes `"src-" + sha256("<source> <name>")` over the
    caller's own albums to invert the id. Consequences: a retitle can't
    resurrect a dismissed album, and one user can't dismiss another's.
    That formula now exists in THREE codebases (server `source_album_id`, web
    `sourceAlbumId`, Android `AlbumRepository.sourceAlbumId`) and every way they
    can disagree is silent — albums duplicate instead of converging, tombstones
    stop matching. All three are pinned to one shared vector
    (`printf 'google_takeout Trip to Rome' | sha256sum`). Do not "tidy" it in one
    place only.
- [x] **Fix manifest replace ordering on BOTH platforms**: upload new manifest →
  persist new blob id locally → best-effort delete the old blob. Never
  delete-first.
  Web had **five** copies of the delete-then-upload sequence (takeoutAlbums,
  secureAdd, AddToAlbumModal, RegularAlbumView, useViewerActions); all now go
  through one `web/src/utils/albumManifest.ts::saveAlbumManifest`, which is the
  only place a manifest is written. Android fixed in `AlbumRepository.syncAlbum`.
  Worst case is now an orphaned blob (a few hundred bytes) instead of an album
  that vanishes from every device with no way to recover it.
- [x] **Minor cleanups**: existence-check before `photo_metadata` insert in
  `import_takeout`; route `scan_takeout` pairing through the shared resolver;
  latch `fullyMaterializedRef` when `photosUnmatched` is stable across runs;
  fix the stale allowlist comment in `sidecar.rs`.
  Notes:
  - Routing `scan_takeout` through the shared walk immediately exposed a second
    copy of the same bug class: `is_google_photos_json` didn't recognise
    duplicate-counter sidecars (`IMG_1.jpg(1).json`), so the report called them
    "not a sidecar" at all. It now lives in `sidecar.rs` as
    `is_photo_sidecar_name`, next to the naming rules it has to agree with.
  - The `fullyMaterializedRef` latch is deliberately **conservative and
    session-scoped**: it needs a pass that changed nothing AND left the same gap
    as the previous one, because latching early means silently incomplete albums
    — the exact bug this whole TODO is about. Refs reset on reload, so a
    premature latch self-heals on the next visit.

### Status

All three phases are code-complete on `dev`, uncommitted and undeployed. Suites:
server `cargo test --bin simple-photos-server` **264** (clippy + fmt clean), web
`npm test` **63** (tsc + build clean), Android `AlbumDisplayNameTest` **8**, E2E
`test_87` (8) + `test_88` (17) green. The wider E2E set covering the touched
paths (photos / albums / upload parity / subtype scan / motion / date ordering)
is 121 passed with only the pre-existing failures listed below.

The one outstanding item is Phase 1's **CT132 ops run** (deploy + run the
backfill against the Takeout directory, then verify album counts vs Google
Photos). Run the backfill *after* deploying Phase 2, so titles land in the same
pass — order doesn't matter for correctness (a re-run repairs titles in place and
reports `albums_retitled`), only for saving a second long run.

### Pre-existing E2E failures found while verifying (NOT caused by this work)

Confirmed by stashing all of the above and re-running on a clean `dev` — they
fail identically there. Unrelated to Takeout albums; noted so they aren't
rediscovered as "regressions" next time:
- `test_06_albums_secure.py` — 8 failures, all **HTTP 401** downloading secure
  gallery item/thumbnail blobs (primary *and* backup).
- `test_20_photo_date_ordering.py` — 4 failures around backup/recovery date
  preservation and TIFF-with-EXIF conversion.
- `test_58_subtype_scan_regression.py` — not a product bug but a **test harness**
  one, worth fixing: its `server_binary` fixture checks
  `target/release/simple-photos-server`, which on Windows never exists (the file
  is `…-server.exe`), so it re-runs `cargo build --release` on *every* run with a
  600s timeout. In a long session — or if anything else touches cargo's target
  lock concurrently — that build overruns/fails and all 21 tests ERROR in fixture
  setup. Appending the platform's exe suffix would make it reuse the binary as
  intended. The suite passes 21/21 when run with warm release artifacts.

### Verification (end of each phase)
- Unit: `cargo test --bin simple-photos-server` (crate is a binary) for
  backfill/title/resolver tests; web `npm test` for takeoutAlbums changes.
- E2E: import a synthetic Takeout fixture (album folders + `metadata.json` +
  duplicate copies in `Photos from YYYY` + `-edited` pairs + `(1)` counters),
  assert `photo_source_albums` contents, then assert client manifests match.
- Live: CT132 backfill run; album count + membership spot-check vs Google Photos.
