# TODO — Open work

> The previous `todo.md` / `todo-completed.md` were deleted in `f6c9ded` ("stuff",
> 2026-11-23). They are the record of *why* most of the current code looks the way it
> does (fourteen plans that were wrong before they were right). Recover with
> `git show f6c9ded^:todo.md` / `git show f6c9ded^:todo-completed.md` before
> re-planning anything in an area they cover.

**Ground rules (unchanged):** one conventional commit per issue on `dev`; every fix
ships with a test that fails without it; never commit red — `cargo test --bin
simple-photos-server`, `npx vitest run` + `npx tsc -b` in `web/`,
`.\gradlew.bat :app:testDebugUnitTest`, and the touched `tests/` E2E files.

---

## 1. Session 2026-11-23 — Android duplicates + secure alignment (Tier 2)

### Baseline (measured at start, `f6c9ded`)
- All suites green: server 555, web 319 + clean `tsc`, Android unit tests pass.
  IDE Problems panel: 0 errors / 0 warnings. "Lots of errors" = behaviour, not build.
- CT132 runs `228b98b` (v1.7.0, image 2026-08-05). Only dep/version/gradle/agent-config
  commits since, so the box is effectively current.

### Findings — measured against the live server + the phone's own client logs
- **D1 Concurrent backup workers.** Periodic, reactive and "immediate" are three
  *differently named* unique works, so they overlap. 2026-11-17: workers found 9,470 /
  4,787 / 4,303 pending in overlapping windows; 5,327 registrations were a stale
  snapshot re-registering a row another worker had just synced (`donorId == localId`).
- **D2 Twin Room rows.** 7,540 local rows were registered as "copies" onto **4,283**
  server photos that another Room row already represented. Room holds 18,826 rows vs
  ~14.5k eligible server photos — the excess *is* the duplicate tiles. Cause:
  `register-encrypted` dedups by hash and returns the EXISTING photo id, the phone
  marks its row with it, and `deduplicateLocalEntities` only considers rows with
  **no** `serverPhotoId`, so the twins are never collapsed.
- **D3 Re-upload of what the server already has.** 4,565 of 5,078 uploaded filenames
  already existed server-side before 11-17. Bytes travel, then the server discards them.
- **D4 Stale whole-row writes revert synced rows** (found while implementing P3). The
  hash-dedup branch ran `update(photo.copy(photoHash = …))` from the pass-start
  snapshot right after `markSynced`, so the row went back to PENDING with no server
  ids. Logs: row `6bd7bd62` uploaded 10:50, re-registered 11:19, picked up *again* by a
  new pass at 14:38; 3,837 rows went register → full re-upload. The EXIF dimension fix
  did the same and wiped the just-saved thumbnail path; a failed copy registration was
  marked SYNCED with no server id (a permanent separate tile).
- **S1 Secure leak on Android main grid.** 158 of those registrations landed on photos
  hidden by secure albums. `excludeSecure` matches only `serverBlobId`; the server hides
  by `photos.id NOT IN (original_blob_id)` and `/galleries/secure/blob-ids` already
  carries those photo ids (158/158 verified). A row whose blob id is the phone's own
  upload is shown.
- **S2 Web secure removal strands burst frames.** The grid collapses a burst to one
  tile; the trash button removes only that item id. Android expands (`expandForRemoval`).
- **S3 Web has no multi-select removal**, and "Select" only appears when another secure
  album exists. Android: long-press select + remove, always.
- **S4 Android deletes a secure album on a second tap** of the trash icon — no dialog,
  no statement of outcome. Web: danger `ConfirmDialog` explaining it.
- **S5 Pull picker ("Move here") splits bursts on Android** — panel collapses bursts and
  moves only the cover id. Web shows every frame uncollapsed (inconsistent display).
- **S6 Web secure viewer has no "Remove from secure album"** (Android's does).

### Plan — one commit each, in this order
- [x] **P1 server** `register-encrypted` duplicate responses also return the existing
      photo's `encrypted_blob_id` / `encrypted_thumb_blob_id`. Additive; old clients
      ignore. `9c3a7a0` — test_95 RED on both duplicate arms, GREEN 3/3.
- [x] **P2 android (D1)** single-flight `BackupWorker`: process-wide gate, a skipped run
      still re-arms the reactive trigger; each row re-read before upload. `88fa1f1` —
      6 tests, RED = exactly the wiring guard.
- [x] **P2b android (D4)** column writes (`setPhotoHashIfMissing`, `updateDimensions`)
      instead of whole stale rows; a failed copy registration is FAILED (retried), not
      SYNCED. Source guard: RED 3/3 on the old worker.
- [x] **P3 android (D2)** one Room row per `serverPhotoId`: pure `planTwinCollapse`
      (keep the oldest device row, adopt the server-only twin's canonical blob ids and
      state, delete the rest + thumbs only they used; refuses rows of different photos).
      Runs after every duplicate register, after a backup batch, and once per process
      *before* the sync's SKIPPED shortcut; a heal forces that pass FULL so crop/favourite
      reconcile. `3b0c6be` — 16 tests, RED = exactly the 3 wiring guards.
- [x] **P4 android (S1)** `excludeSecure` also matches `serverPhotoId`. `0d45cb2`.
- [x] **P5 web (S2/S3)** burst-aware removal (`expandBurstStacks`) sharing one expansion
      with the prompt; selection bar Remove; list-form `secureRemovalPrompt` ported from
      Android + `secureRemovalToast` from the same counts; Select no longer needs a second
      album. `aac357e` — RED = exactly 4 wiring guards while 25 helper tests stay green.
- [x] **P6 android (S4)** secure-album delete asks first; `secureAlbumDeletePrompt` on both
      clients, strings asserted verbatim on both sides. `5e754c8`.
- [x] **P7 both (S5)** pull picker draws one tile per stack and moves whole stacks
      (web `resolveSecureMoves` over `expandBurstStacks`, Android `planPull`); both fixed a
      `picked - moved` failure count that went negative. `ea84a75`.
- [x] **P7b web (found in the browser)** the pull picker could not select ANYTHING: tile and
      wrapper both toggled, the click bubbled, select-then-deselect → "Move here (0)"
      forever. Browser: "(0)" before, "(1)" after. `b97b0aa`.
- [x] **P8 verify** — see "Verification" below. **Deploy to CT132 still needs approval.**

### Verification (2026-11-24)
- Unit: server 555/555, web 339/339 + clean `tsc`, Android 318/318 (`testDebugUnitTest`).
- E2E: `test_95` 3/3 (RED on the old server for both duplicate arms). `test_03`, `test_33`,
  `test_73`, `test_94`, `test_95`: all green. **`test_06` 46/46** — its "known red
  baseline" of 8 (since 2026-07-15) was stale tests omitting `x-gallery-token`; fixed in
  `e44ad10` with a guard that the gate still refuses token-less downloads.
- `assembleDebug` builds (`app-debug.apk`, 34 MB) — the APK to ship for F7.
- **Browser, real UI, disposable local server (never CT132): 12/12.** Album delete dialog
  + Cancel keeps it; burst trash removes all 3 frames ("Remove 3 photos…"); multi-select
  Remove (2) removes both; pull picker shows 2 tiles for 1 single + 1 burst and moving the
  burst tile moved 3 frames, the single stayed. Script + screenshots were session-scratch.
- **Not verified on a device** (none attached): P2/P2b/P3/P4/P6/P7-Android are JVM-tested
  only. The heal on the owner's phone should drop Room from ~18.8k rows to the server's
  ~14.5k on the first sync after the new APK — check "Local DB photo status" in
  `/api/admin/client-logs` after install.

### NOT doing this session (and why)
- **No cleanup of orphaned blobs on CT132.** The 11-17 run left orphan media/thumbnail
  blobs; `orphan_sweep.rs` only reclaims rendition blobs. Deleting needs a guarded
  dry-run-first tool and the owner's OK. → follow-up F1.
- **No pre-upload "does the server have this hash" check (D3).** New API surface; the
  correctness bugs (D1/D2/S1) come first. → follow-up F2.
- **No merging of server-side near-duplicates** (same filename, `taken_at` differing only
  in sub-second precision, different bytes — phone original vs Takeout copy; ~12 live).
  Different bytes means a product decision, not a dedup bug. → follow-up F3.
- **S6 web viewer removal** — needs the three-way prompt inside the viewer route. → F4.
- **No live deploy without explicit approval.**

### Rollback
Every phase is a single commit on `dev`: `git revert <sha>`. No migrations, no schema
change, nothing destructive run against shared state. P3 deletes only *Room mirror rows*
that duplicate another row's `serverPhotoId` (rebuildable by a full sync / rescan; the
device's media files are never touched).

---

## 2. Follow-ups
- [ ] **F1** Guarded sweeper for unreferenced photo/thumbnail blobs (dry-run default,
      counts, refuses empty filter, test that a referenced blob survives).
- [ ] **F2** Pre-upload hash lookup so a reinstalled phone stops re-sending its library.
- [ ] **F3** Decide what to do with phone-vs-Takeout near-duplicates.
- [ ] **F4** "Remove from secure album" in the web viewer.
- [ ] **F5** Album manifests that reference a device row's *own* (orphan) blob id lose that
      member when P3 moves the row onto the canonical blob. Rare (only where blob dedup
      missed — e.g. videos whose server blob has no `content_hash`); remap ids in local
      manifests if it shows up.
- [ ] **F6** `register-encrypted`'s check-then-insert is not atomic; two concurrent
      registrations of one hash can both insert. P2 removes the phone-side concurrency
      that triggered it; a server-side guard (transaction or unique constraint that still
      admits "Save Copy" rows) is still owed.
- [ ] **F7** Install the new APK on the S21+ and confirm: Room row count ≈ server count
      after first sync; no secured photo in the main grid; delete-album dialog; "Move
      here" pulls a whole burst.

## 3. Verification debt carried from the old todo.md (still open)
Deploy-side (CT132): A1/#42 badge counts, A2/#38 migration 033 backfill, B2/#40 3-strike
cap + ETA calibration readout, B3/#46 codec backfill drains to zero, B4/#49 rung backfill.
Device (S21+): B4 quality swap, E1/#43, F1/#41, D2/#50, E3, E3a, Z1e, B5.
Browser: B4 web quality swap, D2 iOS safe-area, A3/#51 virtualization, E3 web, Z1d.
Full detail: `git show f6c9ded^:todo.md`, section "2. Verification debt".
