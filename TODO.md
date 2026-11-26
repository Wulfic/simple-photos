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

## 2. Session 2026-11-25 — follow-ups F1/F2/F4/F6 (Tier 2)

### Measured at start (read-only against CT132 + the phone's client logs)
- CT132 runs `5286392` (image built 06:52Z today), `setup_complete:true`: **not wiped**, so
  the 11-17 orphans are still there.
- The S21+ got the new APK as a **fresh install** (~07:00Z): Room is rebuilding from empty
  (3,000 → 7,904 rows in 3 min, pending climbing to 3,118). The heal works: every
  hash-dedup registration logs `twinRowsCollapsed: 1`, plus "Collapsed 6 twin row(s)".
- **D3 is live, measured:** in the first ~4 min the phone did 278 full uploads (536.7 MB).
  The server reported **298/298 media uploads** as `Duplicate blob upload detected
  (content_hash match)` and kept none of them. Every one also uploaded a **thumbnail that
  WAS stored** (no hash match on encrypted thumbs), then `register-encrypted` answered
  "photo already exists for this blob". So **each re-upload leaves one orphan thumbnail
  blob**. Device holds ~5.3k media, so expect ~5k orphan thumbs from this install.

### Plan — one commit each, in this order
- [x] **F2a server** `POST /api/photos/lookup-hashes` — batch (≤500) of content hashes →
      `{matches: {hash: {photo_id, encrypted_blob_id, encrypted_thumb_blob_id}}}` for this
      user only. Unit 7 (RED by mutating the user/blob predicates: exactly the 2 scoping
      tests) + `test_96` 4/4 E2E. Server 562/562, clippy + fmt clean.
- [x] **F2b android** after hashing and the local-donor check, ask the server; a hit is
      adopted (`markSynced` with its ids + `collapseTwinsOf`) with **no bytes and no
      thumbnail**. Any failure → today's upload. A 404/405 (every server before `87927fd`,
      i.e. CT132 today) turns the lookup off for the rest of the pass: one wasted call per
      pass, not per photo. Pass summary logs `adoptedFromServer` + `hashLookupDisabled`.
      `ServerHashLookupTest` 8 + `BackupHashLookupWiringTest` 3 (RED 3/3 on the old worker).
      Android 329/329, `assembleDebug` builds. **Not device-verified** (no deploy).
- [x] **F6 server** — **re-diagnosed while testing F2a:** `idx_photos_user_hash` is a partial
      UNIQUE index, so two concurrent registrations can NOT both insert (Save Copy rows have
      a NULL hash and are exempt). The loser got a raw constraint error → **500** → phone
      marked FAILED → retry healed. Fix: `ON CONFLICT (user_id, photo_hash) WHERE photo_hash
      IS NOT NULL DO NOTHING` + re-read the winner → `200 duplicate`. Not `INSERT OR IGNORE`
      (would swallow PK/NOT NULL bugs as "duplicates"). `test_97` RED on the old server
      (4/12 concurrent → 500), GREEN ×4 runs; 5 unit tests, each mutation (no clause /
      `OR IGNORE`) turns exactly its one test red. Server 567/567, clippy + fmt clean;
      E2E 95/96/97/03/33 green.
- [x] **F1 server** guarded sweeper for orphan **thumbnails** (re-scoped, see "F1 design"
      below). 21 unit tests; 10 guard mutations each turn their own test red (scan + txn path
      guard, NULL filter, empty-reference refusal, confirm_count, backup mode, grace, txn id
      re-check, restore live-row check, path normalization). Server 588/588, clippy
      `-D warnings` + fmt clean; E2E 98/03/04/33/95/96/97 = 57/57 against a binary verified
      to contain the new routes. **Not run on CT132 — needs deploy + approval, dry-run first.**
- [x] **F4 web** "Remove from secure album" in the secure viewer, reusing the three-way
      prompt + `expandBurstStacks` from P5. See "F4 design" below. Shared pipeline
      `gallery/secureRemoval.ts` (23 tests; RED = exactly the 4 viewer wiring guards on the
      pre-F4 viewer, exactly the 3 burst tests with expansion mutated out). **Browser, real UI,
      disposable local server: 22/22** — single/burst/shared dialogs, Cancel + Escape + arrows
      keep the viewer, confirm closes to the grid with the right toast, Back/reload don't
      reopen or replay, server counts match, grid trash still works, main viewer has no item.
- [x] **F4a web (found in the browser)** Escape on ANY Modal also reached the viewer's
      `window` Escape and left the viewer. A guard in the viewer can't see it: React flushes the
      close between the `document` and `window` listeners. Fix: Modal `stopPropagation`s the
      Escape it acts on. RED in the browser on the old Modal; 3 unit tests. Also fixes the same
      shape on `LeavePrompt` / `DownloadChoiceModal` over the viewer (not browser-verified).
      Web 368/368 + clean `tsc`.
- [x] **F8 DONE** — see "F8 design". Full E2E (minus 09/10) diffed against the SAME command on
      pre-F8 `02521bd`: **0 new failures**; F8 fixes test_77 ×4 + test_17 trash-restore 500.
      Remaining 54 failed / 59 errors are identical on both binaries (pre-existing; 44 errors =
      tests opening the SQLCipher DB with plain `sqlite3`).
- [x] **F8 server (found 2026-11-25, PRE-EXISTING)** `DELETE /api/photos/:id` returns 500
      `database is locked` (SQLITE_BUSY 5 / BUSY_SNAPSHOT 517) when it races the `[SERVER_MIG]`
      background encrypt of the same photo. `soft_delete_photo` (`trash/operations.rs`) opens a
      DEFERRED txn, reads, then writes — a write committed in between makes the read→write
      upgrade fail instantly; `busy_timeout` does not retry 517. test_77 fails 1–4 of 4 params
      per run, **identically on `a5b9cbc` without F1**. Likely fix: `BEGIN IMMEDIATE` for
      read-then-write txns. Audit how many other handlers share the shape before fixing one.

### F8 design (decided 2026-11-25)
- **Audit:** 24 `pool.begin()` sites in 16 files. All are on the WRITE pool (no txn function
  is ever handed `read_pool`), none nested (no savepoints), and every one does its file I/O
  *before* `begin` (blob writes, encrypt) — orphan_thumbs' rename-in-txn is deliberate.
- **Choice: blanket.** `db::begin_write(&pool)` = `pool.begin_with("BEGIN IMMEDIATE")`; all 24
  sites call it. SQLite has one writer regardless, so DEFERRED buys nothing on the write pool
  and makes every read→write txn a latent 517 (`busy_timeout` retries a busy BEGIN IMMEDIATE,
  never a stale-snapshot upgrade). Rejected: fixing only `soft_delete_photo` — leaves 23
  latent copies and the next new txn defaults to DEFERRED again.
- **Guard:** a source-scan unit test fails on any bare `.begin()` under `src/` (same idea as
  F1's schema-enumeration test), so a new txn can't silently reintroduce DEFERRED.
- **Tests:** file-backed WAL DB, two connections, deterministic interleave: DEFERRED read →
  other conn commits a write → upgrade fails with extended code 517 (documents the trap);
  same interleave via `begin_write` → both commit. Reverting `begin_write` to `begin()` turns
  it red. E2E: test_77 ×4 runs green (was 1–4/4 params red per run).
- **Not doing:** touching autocommit single statements (no upgrade window); changing
  `busy_timeout`; moving work out of txns.
- Rollback: `git revert` — no schema or API change.
- Then (owner, 2026-11-25): **push dev + deploy CT132**, verify the deployed commit, run F1
  `orphan-thumbnails` with `apply:false` only.

- [ ] **F9 server (found 2026-11-25 by F8's full E2E, PRE-EXISTING)** Backup sync never
      reaches `success` once an **encrypted Save Copy** exists. Such a copy is stored with
      `photos.file_path = ''` (bytes live only in its encrypted blob, `save_copy.rs`), but the
      photo phase of `sync_engine` reads `file_path` from disk → `read error … os error 3`
      every cycle → status `partial` forever. `wait_for_sync` (tests/helpers.py) only accepts
      `success|error|completed`, so a `partial` run hangs 120 s per test instead of failing fast.
      Repro: `pytest test_01..test_09 -x` → `test_sync_creates_user_on_backup` times out;
      test_09 alone is 42/42. **Identical on `02521bd` (pre-F8)** — 174 passed / 1 failed both.
      Full-suite cost today: 47 failed + 3 errors in test_09/test_10, 1h37m.

- [x] **F9 DONE** — see "F9 design". `test_100` RED 4/4 on the old binary ("os error 3"), GREEN
      4/4. Server 605/605, clippy `-D warnings` + fmt clean. E2E test_01–10 + test_100 on one
      session: **241 passed / 1 failed / 6 errors in 9m55s** (was 47 failed + 3 errors in
      test_09/10 alone, 1h37m). test_09 fully green. The 7 remaining reds → F12.
- [x] **F12 tests (found by F9's run, PRE-EXISTING, masked by the cascade)** test_10
      `TestRecoverySecureGallery` (6 setup errors) + `TestRecoveryPresyncedGallery::
      test_presynced_gallery_decryptable_after_recovery`: `download_blob` of a secure-gallery
      blob → **401 on the primary, before any sync/recovery** (`test_10:559`, `:1065`). Same
      shape as test_06 before `e44ad10`: no `x-gallery-token` since the B6 gate (`8a1d736`). Fix
      the tests to present the token + assert the gate still refuses without it — then see what
      the recovery assertions behind them say (they may be F10).
- [x] **F10 server (found 2026-11-25 reading for F9, PRE-EXISTING since `b5d722d` 2026-04-05) —
      UNVERIFIED until RED.** `recovery_engine.rs` ~L254: the recovery `INSERT … SELECT` string
      is spliced into itself (`…AND id != ?1sh, crop_metadata) SELECT …`) → SQLite parse error at
      runtime → every recovered photo is logged + skipped. Suspicions to prove/disprove:
      (a) recovery registers 0 photos (does test_10 show it, once F9 stops the cascade?);
      (b) the error and `rows_affected()==0` branches `remove_file(full_path)` on the path they
      just wrote — if a local row already owns that path (the autoscan case the comment
      describes), that deletes a live row's file; (c) recovery `download`s every missing row,
      so `file_path=''` rows (Android) can't be recovered by pull at all.
- **F10 re-diagnosed (2026-11-25, next session):** `run_recovery` + `recover_users` have **no
  callers** (`#[allow(dead_code)]`). The live recovery is push-based: `recover_from_backup` →
  the backup runs `run_sync(is_recovery=true)` against the new primary. So (a)/(b) can't
  happen; the spliced SQL never ran. **Choice:** delete the dead pair (~310 lines), keep
  `update_recovery_log`; module doc says what was there + `git show 4a8d15b:…` to recover it.
  Rejected: fixing the SQL — repairs code nothing runs, and its write-before-check +
  `remove_file` shape would come back with it. Suspicion (c) recast for the LIVE path: does a
  `file_path=''` row come back through recovery (backup → receive-photo-rows on the new
  primary)? Untested → `test_100::TestEncryptedOnlyRowsSurviveRecovery` (row + blob ids +
  byte-identical media/thumb on a fresh recovered primary).
- **F12 fix:** the 4 secure `download_blob` calls in test_10 pass the token; the 3 on the
  RECOVERED server also assert a token-less download is 401 (a recovery that lost gallery
  membership would silently un-gate the blob).
- **F12/F10 verified (2026-11-25):** test_10 **30/30** (was 7 red) + test_100 **5/5** on one
  binary, 35/35. RED: the recovery test fails alone ("did not come back") with recovery's row
  push mutated out; the 4 primary→backup tests stay green. Server 605/605, clippy + fmt clean.
  ⚠ First mutation ("drop rows when `backup_mode`='backup'") stayed GREEN — the E2E backup
  server never sets backup mode (API key only). A mutation that doesn't bite is a broken probe
  until shown otherwise.
- [x] **F11 — restore returns bytes, not the photo.** Design + verification below.

### F11 measured (2026-11-25, RED on `8b7701f`, `tests/test_101_trash_restore_registered.py`)
- **Filed mechanism was the minor half.** Both clients trash via `POST /blobs/:id/trash`
  (`soft_delete_blob`), which DELETEs the `photos` row; restore re-inserts only `blobs` rows.
  RED: a restored registered photo is **absent from `/photos/encrypted-sync`** — every client
  that syncs has lost it; web only looks right on the trashing device (Dexie trash copy).
- Filed path `DELETE /photos/:id` (`soft_delete_photo`, **no client caller**): row comes back
  with `encrypted_blob_id = NULL` (RED, exactly as filed).
- `foreign_keys` is ON ⇒ deleting the row CASCADEs `edit_copies`, `photo_source_albums`,
  face/object/pet detections, `video_renditions`. RED: edit copy gone after restore.
  `photo_tags` has no FK ⇒ survives (GREEN, kept as a guard).
- Rendition-row deletes queue their blobs to `orphaned_rendition_blobs` (sweeper deletes them)
  ⇒ renditions can't be snapshotted. `ai_processed_photos` has no FK ⇒ survives trash, so a
  restored row would read as "AI done" with its detections gone.
- `photo_change_log` is trigger-fed on INSERT ⇒ a restored row reaches delta sync for free.
- `(user_id, photo_hash)` and non-empty `file_path` are UNIQUE ⇒ a restore can collide.

### F11 design (decided 2026-11-25)
- **Choice: snapshot-and-restore.** New table `trash_photo_snapshots(trash_id → trash_items ON
  DELETE CASCADE, photo_id, photo_row JSON, children JSON, PK(trash_id, photo_id))`, migration
  `041`. Both trash paths capture every `photos` row they delete (blob path: all rows with that
  `encrypted_blob_id` — not unique) plus its user-authored children, in the SAME transaction,
  before the DELETE. Restore re-inserts them in the same transaction as the blobs.
- **Rejected: flag-in-place (`photos.trashed_at`).** Children survive for free, but 215
  `FROM/JOIN photos` sites in 54 files would each have to exclude trashed rows; one miss leaks
  a trashed photo into gallery/search/backup/pipelines. Rejected: hand-listing columns into
  `trash_items` — already drifted once (that IS the filed bug).
- **Generic, drift-proof encoding.** Rows are read with `SELECT *` and encoded by storage
  class (NULL/INTEGER/REAL/TEXT, BLOB as `{"$b64":…}`). Restore inserts only keys that are
  columns of the table NOW (`pragma_table_info`) — a later-added column takes its default,
  a dropped one is skipped. Identifiers come from the schema, are asserted `[a-z0-9_]`, quoted.
- **Children are classified, and a test enforces it.** `SNAPSHOT = edit_copies,
  photo_source_albums` (user-authored). `REGENERATED = face/object/pet_detections,
  video_renditions` (derived; restore deletes the `ai_processed_photos` row so AI re-runs; the
  rung backfill already re-queues videos with no renditions). Unit test: the set of tables with
  an FK to `photos` == SNAPSHOT ∪ REGENERATED — a new child table fails CI until classified.
- **Collision fails closed.** A UNIQUE violation on restore (same hash re-uploaded since) →
  409 Conflict, logged; the transaction rolls back, trash row + snapshot untouched.
- Restore now emits a `"photo"` sync event and invalidates the summary cache (it did neither).
- Legacy trash rows (no snapshot) restore exactly as before.
- **Widened mid-implementation (RED first):** the `blobs` rows had the same disease. Restore
  hand-built them with `size_bytes` = the client's trash-request size (phone sends the ORIGINAL
  file size), `0` for the thumbnail, and no `blob_format`; `size_bytes` IS the download's
  Content-Length. RED: restored v2 blob served 4096 of 8192 bytes; thumbnail served empty.
  → `trash_blob_snapshots` (same migration 041), captured owner-scoped (a client-named
  thumbnail id of ANOTHER user's blob must not be captured — restore would 409 forever).
  Registered with the orphan sweeper's `REFERENCE_COLUMNS`.
- test_77 asserted the legacy `created_at := taken_at` workaround; now asserts restored dates
  equal the pre-trash dates exactly (stricter; HEAD fails 3 of 4 — the 4th is correct on both).

### F11 verified (2026-11-25)
- Unit: 615/615 (10 in `trash::snapshot`), clippy + fmt clean. Mutations bite: drop a child
  classification → drift test red; skip `ai_processed_photos` clear → round-trip red; ignore the
  UNIQUE mapping → collision test red; drop the owner filter → foreign-blob test red.
- E2E on a verified binary (`grep -c trash_blob_snapshots` = 8): test_101 4/4 (RED on HEAD: 3/4,
  then 1/4 after the photos half), test_77 4/4, test_04/03/95–100/10 green (94/94).
- test_21 ×3 + test_41 ×1 fail identically on HEAD `8b7701f` — pre-existing, not F11.

### F11 NOT doing (and why)
- `DELETE /photos/:id` (no client caller) leaves the thumbnail's `blobs` row live, referenced
  only inside snapshot JSON the orphan sweeper can't read — same exposure as before F11, and
  the sweeper is admin-run, dry-run by default, and its quarantine is reversible.
- Reconstructing rows for legacy (pre-041) trash items: a blob with no snapshot may never have
  had a row (pure-E2E web upload) — inventing one could duplicate it. ≤30 days of exposure.
- Snapshots don't travel to the backup (F9 trash rows are row-only); restore ON a backup keeps
  today's behavior.
- `soft_delete_photo` on a registered row leaves its blobs in `blobs`, so permanent delete can't
  find them (leak). No client calls it; separate item if it ever gets one.
- Re-linking detections to clusters (cluster may be gone by restore) — re-detection instead.

### F11 rollback
- Code: `git revert <F11 commit>`. Binary rollback is safe after the migration ran
  (`set_ignore_missing(true)` in `db.rs`). Schema undo: `DROP TABLE IF EXISTS
  trash_photo_snapshots;` — loses only snapshots of currently-trashed items (= today's
  behavior). Not deployed; nothing on CT132 to undo.

### F9 design (decided 2026-11-25, late — re-scoped from "Save Copy" after reading the code)
- **Wider than filed.** `file_path = ''` is not a Save Copy quirk: `register-encrypted` (EVERY
  Android upload) and encrypted Save Copy both write it, and `soft_delete_photo` copies it into
  `trash_items`. So phase 1 (`sync_photos`) AND phase 5 (`sync_trash`) `send_file` the storage
  ROOT for every such row → failure → `partial` forever, and **no Android photo row has ever
  reached a backup** (only its blobs, via phase 3). No E2E covered it: test_09 uploads plaintext
  files and bare blobs, never `register-encrypted`.
- **Choice:** partition by `file_path`. Non-empty → unchanged `send_file`. Empty → new row-only
  endpoints on the backup, JSON batches ≤ 200: `POST /api/backup/receive-photo-rows` and
  `…/receive-trash-rows`. No body bytes, no file I/O, no thumbnail generation.
- **Rejected: letting `/backup/receive` accept an empty path.** It is file-centric, and its
  trash branch runs `DELETE FROM photos WHERE id = ? OR encrypted_blob_id = ? OR file_path = ?`
  — with `''` that matches every encrypted-only row on the backup. A new endpoint keyed ONLY
  on `id` cannot hit that.
- **Row endpoint rules:** upsert `ON CONFLICT(id)` with the same COALESCE semantics as
  `/backup/receive`; `file_path` is forced `''` server-side (never taken from the payload);
  owner mapping shared with `/backup/receive` (extract one helper); tags replicated; stale
  `trash_items` row with the same id removed (same as receive). Per-row result: one bad row
  fails alone, returned as `{id, error}` and counted as a failure by the primary. A hash
  conflict with a DIFFERENT id on the backup fails that row loudly — no delete-to-make-room.
  Trash rows: removing the backup's live `photos` row uses `id = ?` (+ `encrypted_blob_id = ?`)
  and refuses an empty id.
- Extended columns (subtype, burst, motion, EXIF, geo) are NOT in the payload: phase 4's
  `photo_states` already upserts them onto existing rows, and the row now exists.
- A backup older than F9 answers 404 → each row counts as a failure with "backup server lacks
  receive-photo-rows (upgrade it)" — stays `partial`, but says why.
- **Test harness:** `wait_for_sync` treats `partial` as terminal (no more 120 s hang), and
  `trigger_and_wait` raises on `partial` with the log's error — same strictness, fails fast.
- Tests: E2E `test_99` — registered photo reaches the backup with its blob ids and sync is
  `success`; trashed registered photo reaches backup trash and leaves the backup gallery;
  an empty-path row never deletes another backup row. Unit: partition, payload never carries
  a path, row upsert ignores a payload `file_path`, empty-id trash row refused.
- **Not doing:** the latent `''` match in `trash/operations.rs:617` permanent-delete refcount
  (benign: over-counts, so the file is kept) — noted, separate. Sending blobs differently.
- Rollback: `git revert` — additive endpoints, no schema change.

### Owner answers (2026-11-25, session start)
- **No deploy this session** — code + tests only. CT132 and the phone stay as they are.
- F1 quarantine: **kept until a manual purge** (separate call, dry-run default). Never auto-unlinked.
- F1 dry-run against CT132 is OK. With no deploy that means the same read-only query run
  on a *copy* of the live DB, not the endpoint.

### F2 design decisions
- Lookup keys on `photos.photo_hash` (what `register-encrypted` dedups on), not
  `blobs.content_hash`. Android sends the same 12-hex value as both.
- Only hits with a non-NULL `encrypted_blob_id` are returned: a client can only adopt a
  photo it can download. A parked/unencrypted photo is a miss → today's upload path.
- Android looks up **per photo**, right after it hashes (the hash needs the file read
  anyway, so there is nothing to batch ahead without a second full read). The endpoint
  still takes ≤500 for later callers.
- A hit is adopted directly (`markSynced` with the server's ids + `collapseTwinsOf`), with
  no `register-encrypted` call: that call would only answer the same ids from its first arm.

### F1 measured (read-only, DB copy on CT132, 2026-11-25, box `5286392`)
| type | rows | no DB reference |
|---|---|---|
| thumbnail | 34,959 | **18,067 (754 MB)** — clusters 08-05/06, 09-17, 09-27, 11-17, 11-25 |
| video_thumbnail | 691 | 30 |
| photo / gif | 16,326 / 3,636 | **53 / 4** (Aug 5–9) |
| video / motion_video | 1,031 / 276 | 0 / 0 |
| album_manifest | 19 | 19 (by design: manifests are referenced by nothing server-side) |
No `storage_path` is shared by two rows. Live schema columns holding blob ids: photos ×3 +
`photos.id`, `encrypted_gallery_items` ×4, `video_renditions.blob_id`, `trash_items` ×3 +
`photo_id`, `photo_metadata.blob_id`, `shared_album_photos.photo_ref`, `orphaned_rendition_blobs`.

### F1 design (decided 2026-11-25 — supersedes the original "photo/thumbnail/gif/video")
- **Thumbnails only** (`thumbnail`, `video_thumbnail`). Media is never a candidate: web's full
  sync renders an unregistered media blob as a photo (`syncDirectBlobs`), so "no `photos`
  row" ≠ orphan; and the D3 re-uploads left **0** media orphans (dedup kept none).
- **The 57 direct blobs' thumbnails are invisible to the server** (their id lives only in the
  encrypted payload). Protected by proximity: a thumbnail uploaded within **±1h** of an
  unreferenced media blob of the same user is kept and counted (`kept_near_direct_media`).
- **Reference set = every column above**, and a unit test enumerates every `*blob*`/`*ref*`
  column of the migrated schema and fails if the sweeper does not know it.
- **Path guard** (added at review): ids are not the only link — encrypted trash restore
  re-creates a thumbnail blob at `trash_items.thumb_path`. A thumbnail whose `storage_path` is
  named by any `*path*` column, or by a second `blobs` row, is kept (`kept_shared_path`); a
  schema test enumerates `*path*` columns the same way. Checked again inside the move txn.
- `unquarantine_row` must not drop the quarantine record when a live row blocks the re-insert.
- Grace **24h** (`julianday(upload_time)` NULL ⇒ kept). Android uploads+registers in one call
  and re-uploads on retry, so nothing legitimately references a thumb uploaded a day ago.
- **Refuses**: backup-mode servers (blobs arrive before their photo rows); a library with
  candidates but zero referenced blobs (an empty reference set ⇒ "everything is an orphan");
  `apply` whose `confirm_count` ≠ the current candidate count; a path that escapes the root.
- **Quarantine, never unlink**: migration `040` `quarantined_blobs` (the full `blobs` row +
  `quarantine_path` + `run_id`). Per blob: one txn re-checks the reference predicate, copies the
  row, deletes it from `blobs`; then the file is renamed to
  `<root>/.quarantine/<run_id>/<storage_path>`. Rename fails ⇒ the txn is reversed.
- Admin endpoints, all `apply:false` by default: `POST /api/admin/maintenance/orphan-thumbnails`
  (`{apply, confirm_count, limit≤5000}`), `…/restore` (`{run_id, apply}`), `…/purge`
  (`{run_id, apply, confirm_count}`; the only unlink; `run_id` required).
- Tests: referenced (each column) survives; young survives; near-direct-media survives; empty
  reference set refuses; dry-run moves nothing; wrong `confirm_count` refuses; quarantine →
  restore round-trips byte-identical; purge without `run_id` refuses; backup mode refuses.
- **Not doing:** sweeping media or manifests; registering the 57 direct blobs; a scheduler
  (manual only); running it on CT132 (needs deploy + approval).
- Rollback: `git revert`; migration 040 is additive (reverse = `DROP TABLE quarantined_blobs`).

### F4 design (decided 2026-11-25)
- **Parity target = Android `SecurePhotoViewer`**: overflow item "Remove from secure album";
  acts on the open item's whole burst stack; the three-way dialog renders OVER the viewer
  (Cancel keeps it open); Confirm removes, closes the viewer, toast shows on the grid.
- **One planner, two callers.** Extract `requestRemoval`/`confirmRemoval` from
  `SecureGallery.tsx` into pure `gallery/secureRemoval.ts`: `planSecureRemoval(pool, tiles,
  openAlbum)` → `{targets, otherAlbums, verdict}` and `executeSecureRemoval(plan, deps)` →
  `{removedCounts, failed}` + `secureRemovalOutcome` → `{success, error}`. SecureGallery and
  Viewer both call them. Rejected: a viewer-local copy — a separately derived second copy is
  how Z1d's false toast happened.
- Owning-album name: open album's name, else the item's own `galleries` membership (both
  feeds publish it), else `gallery_name`. No new nav state for the name.
- Viewer hides the item when there is no `secureAlbumId`, no matching secure item, or on a
  backup server. Back stack: removal navigates with `replace` so Back cannot reopen it.
- Toast crosses the route as `location.state.secureFlash`, read once on mount.
- Tests: planner/executor unit tests (stack expansion from a non-cover frame, smart-view
  owning id, name source, unknown ⇒ blocked, per-item failure isolation, cache-drop failure
  is not a removal failure); wiring guards on Viewer/ViewerTopBar/SecureGallery source.
- Not doing: advance-to-next-photo after removal (Android closes; parity wins).
- Rollback: `git revert` — web only, no server or schema change.

### NOT doing (and why)
- **F3 won't-fix** (owner, 2026-11-25): phone-vs-Takeout pairs have different bytes; both stay.
- **F5** waits for evidence: no member loss seen yet. Re-check after the reinstall settles.
- Not pausing the phone's backup: it's LAN traffic, it finishes on its own, and it is
  the F7 verification. Its orphans fall to F1.

### Rollback
F2a/F2b/F4 additive (`git revert`). F6 changes write semantics only. F1 never unlinks:
quarantine dir + restore = move back; the endpoint is inert unless called with `apply=true`.

## 3. Follow-ups (carried)
- [ ] **F1** → §2.
- [ ] **F2** → §2.
- [x] **F3** won't-fix (owner decision 2026-11-25).
- [ ] **F4** "Remove from secure album" in the web viewer.
- [ ] **F5** Album manifests that reference a device row's *own* (orphan) blob id lose that
      member when P3 moves the row onto the canonical blob. Rare (only where blob dedup
      missed — e.g. videos whose server blob has no `content_hash`); remap ids in local
      manifests if it shows up.
- [x] **F6** → §2 (the loser 500'd; it never double-inserted — the UNIQUE index exists).
- [ ] **F7** APK installed 2026-11-25 (fresh install). Heal confirmed in logs. Still owed
      once the initial scan drains: Room rows ≈ server count; no secured photo in the main
      grid; delete-album dialog; "Move here" pulls a whole burst.

- [ ] **Android lint is red on dev** (found N1c, pre-existing): `NavGraph.kt`
      `navController.currentBackStack` is `RestrictedApi` (#37 back-history cap). 1 error,
      127 warnings. Replace with `currentBackStackEntryFlow` + own stack tracking, or the
      public `visibleEntries`; then gate lint.

## 4. New issues reported 2026-11-25 (owner) — investigate after F12/F10/F11

Owner answers (2026-11-25): CT132 logs + a DB **copy** may be read **read-only** for these
(no restart, no writes, no deploy). Order: oldest item first, so these queue behind §2.

- [ ] **N1 Android must log out + back in after every server restart.** Android only (web is
      fine). JWT secret comes from `config.toml` (stable), so the access token itself should
      survive a restart — suspects to check, in order: refresh-token storage/rotation on the
      server (in-memory? rotated on restart?), Android's refresh path treating a connection
      error during the restart window as "logged out" (cf. memory "Android refresh rot":
      `RefreshResponse` lacks `refresh_token` ⇒ wipes sessions), and the beacon/discovery
      re-resolving the server URL. Evidence first: `/api/admin/client-logs` around a known
      restart time.
- [ ] **N2 Server stops answering until restarted; the VM is fine.** Shape (owner): the page
      **won't load at all** (connection refused/timeout — looks dead, not a hung API). So: does
      the process exit (container restart policy? OOM kill?), or does it stay up with a
      wedged listener (runtime threads all blocked on sync I/O / ffmpeg / a DB lock)? Evidence
      first: container state + exit code + restart count, `docker logs` tail before the last
      restart, kernel OOM messages, memory/FD counts of the live process.
- [ ] **N3 Secure section loads slowly while scrolling — owner suspects double encryption.**
      Check: what a secure tile actually fetches (full blob vs thumbnail?), whether a secure
      item's blob is encrypted again on top of the per-photo encryption (blob header / size vs
      the source), per-tile request timing, and whether secure grids skip the virtualization
      / thumbnail cache the main grid has (#51). Measure before fixing.
- [ ] **N4 Android re-login + re-enabling backup folders re-encrypts everything.** (owner
      2026-11-26) After logging back in and turning the backup folders back on, the app runs
      the whole encrypt+upload pass again. Likely mechanism (unverified): logout wipes Room
      (`photoDao`/`blobQueueDao`/`backupFolderDao`) and the key, so the phone has no record
      that a local file already has a server twin ⇒ encrypts it before asking the server.
      Check: whether the server-hash lookup (`ServerHashLookup`, `BackupHashLookupWiringTest`)
      runs BEFORE encryption on a fresh mirror; whether a dead session (N1c) can avoid the
      wipe altogether. Measure on a re-login: files encrypted vs files already on server.
- [ ] **N5 Android biometric prompt for the secure folder even with biometrics disabled.**
      (owner 2026-11-26) Setting off ⇒ secure-folder unlock still prompts for biometrics.
      Check: which pref the secure-unlock path reads vs. which one the settings toggle writes
      (cf. `UnlockSessionTest`, memory "Biometric landscape").
- [ ] **N6 No draggable scrollbar on the photo grids — owner must swipe repeatedly to reach
      the bottom.** (owner 2026-11-26) Measured: Android `GalleryScreen.kt` has no scrollbar /
      fast-scroller at all (grep: zero hits); web has native overflow only
      (`scrollbar-gutter`, memory "Gallery scrollbars"). Wanted on EVERY grid (main, album,
      secure, search, smart albums). Likely shape: one shared draggable thumb + date bubble
      (year/month) driven by the grid's `LazyGridState`, reused by every grid screen.
      **Owner 2026-11-26: Android only.**
- [ ] **N7 Smart albums don't show up while the server is processing photos.** (owner
      2026-11-26) Check: does the smart-album list endpoint block/skip while the
      conversion/AI/geo queues run (lock contention, a "pipeline busy" gate, an empty result on
      timeout), or does the client hide the section on a slow/failed fetch? Measure the
      endpoint's latency + response during an active pass before guessing.
      **Owner 2026-11-26: both Android and web** ⇒ suspect the server side first.
- [ ] **N8 Face groupings still weak.** (owner 2026-11-26) Needs a measurement first: on the
      live library, count clusters, singleton rate, and a sample of mis-merges vs. splits
      (memory "8-issue triage": centroid linkage; "#48 face centering"). Decide split-vs-merge
      bias from the numbers, not from the complaint.

### N1/N2 measured (2026-11-26 ~03:20Z, read-only: `docker logs`/`inspect` + a /tmp DB copy on CT132, box `5286392`)
- **Server cannot invalidate a session on restart.** Access-token check is stateless (HS256
  sig + `exp`, `auth/middleware.rs`); refresh tokens are DB rows; `jwt_secret` is from the
  mounted `config.toml`. Android never logs itself out: Login is only the start route when no
  token is stored (`NavViewModel.init`). So "must log out + in" = the owner's workaround for
  a broken-but-logged-in app, OR the tokens really died server-side.
- **Tokens did NOT die server-side in the 30d audit window:** 0 `revoked_token_reuse`, 0
  `login_failure`. Android refreshed ~hourly 11-17 → 11-25 on ONE login (chain survived a
  4-day gap 11-19→11-24). Android logins: 11-17 10:22, **11-25 07:00:39** — 7 min after the
  06:53 deploy (container recreate). First post-login run logged "one-time EXIF dimension
  repair" + 45 photos to process ⇒ looks like a fresh APK install, not a dead session.
- ⚠ Blind spot: an hourly task DELETES revoked refresh tokens, so a replay >1h later answers
  "invalid" with **no audit row**; and client logs need auth to upload, so a phone stuck on
  401/unreachable leaves **no trace**. Absence of evidence here is weak.
- **16:52Z was a CT132 reboot** (box uptime 10:25 at 03:18Z; container exit=0, restarts=0,
  policy unless-stopped). Server logged normally until 16:49 (autoscan every 5 min, ingest,
  crypto, ladder) ⇒ if it was rebooted for N2, the shape is **HTTP dead, runtime alive**.
  No connection cap / timeout layer on the listener. `dmesg` OOM = Plex in LXC 888 (unrelated).
  Live process now: RSS 1.3 GB, 24 threads, 24 FDs.
- **Since the reboot: zero Android contact for 10h** (no refresh, no login, no client log)
  although the access token expired 14:31Z.
- Dead code found: `AuthRepository.refreshToken()` (no callers) drops the rotated refresh
  token — the "refresh rot" bug, latent. Delete it with the N1 fix.

### N1 owner answers (2026-11-26)
Symptom: after the server has run ~24h and died (N2), tapping a photo in the Android app
gives **401 Unauthorized**; restarting the server does NOT clear it; only logout + re-adding
the server does. Happens on plain restarts too (no APK). The 16:52Z reboot WAS an N2 death.
Owner approved one `docker restart` for a repro (not used yet — a restart can't reproduce a
~24h-onset fault; a live observation of the CURRENT phone state is the better probe).

### N1 suspicions — questions until a RED run separates them
- **S1 (lead): non-session 401s drive the session authenticator.** The secure gate answers an
  expired/missing gallery token with **401** (`gallery/access.rs` `require_secure_access`;
  token = stateless HMAC, 1h TTL ⇒ survives restarts). Android's OkHttp `Authenticator`
  treats ANY 401 as an expired session: refresh → retry once with the same stale
  `GalleryTokenHolder.token` → 401 → give up. Persistent across restarts, cleared only by
  logout (which nulls `GalleryTokenHolder`). Open question: which "photos from the server"
  are `is_secure_item` (secure originals leaking into the main grid? B5 filter?).
  Fingerprint: one `token_refresh` audit row PER TAP while the photo still fails.
- **S2: lost refresh response** (rotation committed, client timed out at 10s) ⇒ dead
  refresh token. Fingerprint: NO `token_refresh` rows on taps (refresh returns 401
  "invalid", unaudited).
- Side bug (certain, from code): logout `dataStore.clear()` also wipes `KEY_SERVER_URL` /
  `KEY_SERVER_CONFIGURED` — that's why "log out" also means "forget the server".
- Logging gap (certain): server emits NOTHING for a 401 (`AppError::Unauthorized` silent,
  `TraceLayer` at debug) — the failure is invisible in `docker logs`.

### N1 MEASURED — live repro 2026-11-26 04:09Z (owner tapped photos, no logout)
**S1 refuted, S2 confirmed.** Zero `token_refresh` rows; 55 `Rate limit exceeded
ip=192.168.86.28 limit=100` in 04:09:35–57 ⇒ ≥155 refresh calls in 22 s, the first 100
reached the handler and NONE succeeded ⇒ the phone's refresh token is not in the DB. The
DB's newest Android token (`d720fdd3`, issued by the 13:31:35 rotation) is valid/unrevoked
⇒ the phone never persisted the 13:31 rotation and still holds the 10:29 token, which the
13:31 rotation revoked and the 13:53 cleanup DELETED. 13:31 is the phone's last request to
reach the server (hourly client-log uploads stop at 11:29; nothing request-driven 13:31 →
16:52 reboot) ⇒ the rotation response was lost while the server went HTTP-dead (N2).
DTO mapping is correct (`@SerializedName("refresh_token")`).

Root causes (all certain from code + the run above):
1. **Rotation is not lost-response safe.** Server commits the rotation before the client
   has the new token; a lost response strands the client on a revoked token forever.
2. **Cleanup purges every revoked token within an hour** (`tasks.rs`: `revoked = 1 AND
   created_at < now` — always true). Destroys the chain needed to recover AND silently
   downgrades theft detection to "invalid" after 1 h.
3. **Android refresh storm.** A failed refresh leaves the access token unchanged, so every
   queued 401 runs its own refresh (`NetworkModule` authenticator) ⇒ 100+/min ⇒ 429.
4. **Android never treats a dead session as dead.** It stays on the gallery 401-ing instead
   of routing to Login; and logout's `dataStore.clear()` wipes the server URL ⇒ "forget
   server".
5. **Refresh "invalid" path is silent** (no log, no audit) — why this took a live repro.

### N1 plan (Tier 2 — server migration + Android; one commit per step, in order)
- **N1a server — lost-response-safe rotation (migr 042, additive).** `refresh_tokens.
  replaced_by TEXT NULL` (successor id). Rotation sets it. Presenting a revoked token whose
  successor is still unrevoked (= never presented) ⇒ lost response: revoke the successor,
  issue a fresh pair, chain it, audit `token_refresh {"recovered":"lost_rotation"}` + WARN.
  Successor already used/revoked ⇒ today's theft path. All inside one write txn (F8
  IMMEDIATE) so two concurrent replays can't both recover. Cleanup deletes only expired
  rows (revoked rows live to `expires_at`; ≤ ~720 rows/device/30 d). Log + audit the
  "invalid" path (`login_failure {"reason":"refresh_not_found"}`).
  Tests (E2E, `tests/test_01_auth.py` style, RED first): replay-after-lost-response
  succeeds; replay after the successor was used still revokes everything; two concurrent
  replays ⇒ exactly one succeeds-or-both-serialise with one live family; cleanup keeps
  revoked-unexpired rows. Rust unit test for the cleanup predicate.
- **N1b Android — no storm.** Authenticator remembers the refresh token that got a
  definitive 401 from `/auth/refresh`; later 401s with that same token return null without
  calling the server. 429/IO ⇒ no retry storm (fail this request, keep tokens). JVM test.
- **N1c Android — dead session routes to Login, server kept.** Definitive refresh 401 ⇒
  session-expired signal ⇒ NavGraph → Login (popUpTo 0). Logout keeps `KEY_SERVER_URL` +
  `KEY_SERVER_CONFIGURED`. Delete dead `AuthRepository.refreshToken()`. JVM tests.
- Device verification (owner, S21+): after deploy, a forced lost response can't be staged
  on the phone; verification = E2E for N1a + JVM for N1b/c + "no 401 after the next N2".

### N1a verified (2026-11-26)
- [x] Migr 042 `refresh_tokens.replaced_by`; `auth::refresh_chain::{redeem, purge_expired}` —
  decision + writes in one `begin_write` txn; handler logs/audits all three outcomes.
- RED on HEAD (`187f599` code): 4/5 of `tests/test_102_refresh_lost_rotation.py` failed —
  replay ⇒ `"Refresh token has been revoked"` (theft path), no `refresh_not_found` audit row.
  The 2 theft guards were green before and after.
- GREEN: test_102 5/5 + test_01 23/23; 8 `refresh_chain` unit tests; full unit suite 622/622
  (+1 since); clippy/fmt clean. Mutations bite: DEFERRED txn ⇒ concurrent replay fails BUSY;
  old purge predicate ⇒ purge test fails.
- Contract change: `test_01::test_refresh_token_rotation` now asserts "old token dead once
  the successor is USED" — "dead right after rotation" is exactly the N1 bug.
- test_15 (7 red) is environmental: `os.chmod(dir, 0o000)` is a no-op on Windows.
- Rollback: revert the N1a commit; migr 042 stays harmless (nullable, unread by old code).

### N1b design (2026-11-26)
Storm mechanism (from code): a failed refresh leaves `KEY_ACCESS_TOKEN` unchanged, so every
queued 401 passes the "another request already refreshed" check and calls `/auth/refresh`
itself. Fix: move the decision out of the `NetworkModule` lambda into a pure
`data/remote/SessionRefresher` (token I/O, refresh call, clock injected ⇒ JVM-testable):
1. Access token changed since this request was sent ⇒ retry with it (unchanged).
2. Refresh token == the one that got a **401** from `/auth/refresh` ⇒ `null`, no network.
   401 is the only definitive answer (server: NotFound/Theft both 401). Fires
   `onSessionDead` once per dead token (N1c hooks it).
3. Any other failure (429, 5xx, IO, parse) ⇒ tokens kept, cooldown (Retry-After if present,
   clamped 5..60 s, else 10 s); 401s inside the cooldown ⇒ `null`, no network. Also stops a
   refresh-timeout × N queued requests from blocking OkHttp threads serially.
- Tests (`SessionRefresherTest`, RED first against a copy of today's logic): 50 401s after
  a definitive 401 ⇒ exactly 1 refresh call; 429 ⇒ tokens unchanged + 1 call in cooldown,
  a 2nd call after it; success ⇒ both tokens saved; concurrent 401s ⇒ 1 call.
- Rollback: revert the commit (no data or schema).

### N1b verified (2026-11-26)
- [x] `data/remote/SessionRefresher` + `NetworkModule` wiring (refresh client built once).
- RED with today's logic moved verbatim into the class: 3/9 failed — 50 refresh calls for 50
  401s after a definitive 401 / a 429, 20 for 20 after an IOException (expected 1 each).
- GREEN: `SessionRefresherTest` 9/9; full Android unit suite 338/338.
- Dead-token memory is in-process only: an app restart costs one more refresh call (fine).
- `onSessionDead` fires under the lock ⇒ N1c must only post a signal from it.

### N1c design (decided 2026-11-26)
A dead session is an **expiry, not a logout**: the server refused the refresh token, nothing
about the device's library changed. So:
1. `SessionRefresher.onSessionDead(deadRefreshToken)` → `SessionEvents.report(token)`
   (`@Singleton`, a `StateFlow` — non-blocking under the lock, conflated, kept if no
   collector is attached yet, and never consumed: every window (#21) decides by step 2's
   compare, so a window opened after a re-login ignores the old dead token).
2. `NavViewModel` collects it → `AuthRepository.expireSession(dead)` → NavGraph navigates
   Login `popUpTo(0)`. `expireSession` removes ONLY the access + refresh token, and only if
   the stored refresh token still IS the dead one (compare-and-clear: a login that raced
   the signal must not be logged out). Room mirror, key, backup folders, server, username
   all kept ⇒ re-login does not re-encrypt (the dead-session half of N4).
3. Keeping the mirror across a re-login is only safe for the SAME account. New pref
   `session_owner = "<server url>|<username>"` written at login; `login` with a different
   owner ⇒ the full local wipe (today's logout minus the server call) BEFORE saving tokens.
   Legacy installs (no owner pref) fall back to `KEY_SERVER_URL|KEY_USERNAME`. Unknown ⇒
   wipe (fail closed).
4. Explicit logout keeps `KEY_SERVER_URL` + `KEY_SERVER_CONFIGURED` + theme (wipes the
   rest, as today; theme because a fresh device's first login runs the same wipe and Login
   has its own theme toggle). Login gains a "Change server" link → ServerSetup, since logout no longer
   forgets it. Login pre-fills the stored username.
5. Delete dead `AuthRepository.refreshToken()`; the logout `catch {}`s get a log line.
- Pure pref logic in `data/repository/SessionPrefs.kt` (MutablePreferences in, no Android)
  ⇒ JVM-tested: expire keeps everything but tokens; expire with a non-matching token
  changes nothing; logout keeps the server; owner match / mismatch / legacy / unknown.
  `SessionRefresherTest`: the dead signal carries the token.
- Rollback: revert the commit. One new pref key; old code ignores it.

### N1c verified (2026-11-26)
- [x] `SessionEvents` + `SessionPrefs` + `AuthRepository.expireSession/establishSession/
  wipeLocalAccount`; `NavViewModel` → conflated `Channel` → NavGraph Login `popUpTo(0)`
  (a SharedFlow drops the event when a worker reports before NavGraph collects);
  Login "Change server" + username pre-fill; dead `refreshToken()` deleted; logout
  `catch {}`s now log.
- RED: `SessionPrefsTest` against today's behaviour stubbed in (every exit = `clear()`,
  every login wipes) — 6/11 failed (expire keeps nothing, logout drops server, same-account
  re-login wipes). The 5 wipe / fail-closed guards pass both ways by design.
- GREEN: full Android unit suite 349/349 (+11). Lint: 0 hits in touched files; the 1 lint
  ERROR (`NavGraph` `currentBackStack` RestrictedApi, #37) predates this — see §3.
- Not device-verified: needs the next real dead session on the S21+ (expected: Login with
  username filled, server kept, re-login does NOT re-encrypt).

### Deploy 2026-11-26 (F2a..N1c, `5286392` → `a1e29c9`, `-NoWipe -SkipAssets`)
- Backup (verified: integrity ok, counts = live: 17,553 photos / 57,418 blobs / 6 refresh
  tokens / migr 39): CT132 `/root/sp-deploy-backups/n1-20261126-054303/` — keyed
  `sqlcipher_export` of the live DB (read-only attach), old APK, `SHA256SUMS`.
  Old image tagged `simple-photos-server:rollback-5286392` (`bdc0d563`).
- Rollback (inside CT132, instance dir): `git -C /opt/simple-photos reset --hard 5286392`;
  `docker tag simple-photos-server:rollback-5286392 simple-photos-server:latest`;
  `docker compose up -d --force-recreate --no-build`. Migr 040–042 additive ⇒ old binary
  runs on the new DB. DB restore only if corrupted: stop, move `data/db/simple-photos.db*`
  aside, copy the backup in. APK: copy `$BK/simple-photos.apk` back to `downloads/`.

### N1 NOT doing (and why)
- Not fixing N2 here — N1a makes the client survive N2; N2 is next.
- Not changing the secure-gate 401 → 403 (S1 was refuted; separate, cosmetic).
- Not logging every 401 server-wide (noisy; secure gate + expiry are normal) — only the
  refresh path, which is the one that strands clients.
- Not lengthening the Android refresh read timeout — N1a makes a timed-out refresh harmless.
- Not a "session expired" banner on Login (cosmetic; the pre-filled username is the cue).
- Not fixing N4 for EXPLICIT logout (that wipe is deliberate; N4 is its own item).
- Not pausing backup workers on a dead session: their 401s now cost no network (N1b) and
  they resume on re-login.

### N1 rollback
Migr 042 is one nullable column (old binary ignores it; `set_ignore_missing`). Revert the
N1a commit ⇒ old behaviour; revoked rows then purge at the next hourly cleanup. Android:
revert N1b/N1c commits. Owner's phone right now: just log in again (evidence captured).

### N1 device-verified (owner, 2026-11-26)
APK installed on the S21+; the stranded phone landed on Login with the username filled, and
re-login did NOT re-encrypt. N1 closed except "no 401 after the next N2".

### N2 owner answers (2026-11-26)
Browser **spins, then times out** (not an instant refusal) ⇒ the process is alive and the
kernel accepts or queues the TCP connection, but no HTTP response comes back. Owner approved
building + deploying a diagnostics-only build (N2a) so the next death records itself.

### N2 suspicions — questions until a measurement separates them
Facts (code): plain HTTP on CT132; `#[tokio::main]` with 4 workers; accept loop runs on the
main thread (`block_on`), connections on workers; no header-read / request timeout, no
connection cap; `/health` reads one atomic (no DB, no storage); storage is an SMB mount.
- **S1 blocking-pool exhaustion by stalled streams.** `photos/serve.rs` `chunked_decrypt_body`
  decrypts on `spawn_blocking` and `blocking_send`s into a 4-slot channel with no timeout. A
  client that keeps the socket open but stops reading (a paused `<video>`, a phone in Doze)
  parks that thread until the socket closes. 512 parked ⇒ every `spawn_blocking` / `tokio::fs`
  (incl. `ServeDir`) queues forever ⇒ page spins. Fingerprint: thread count climbs; the
  blocking-pool ping times out while the async ping is fine; many `futex` threads.
- **S2 all 4 workers blocked in sync I/O** (std::fs on a hung SMB, std Mutex, sync process
  wait). Fingerprint: async ping times out; worker threads in state `D` (SMB) or `futex`.
- **S3 a handler wedged on a lock/DB held across an await.** Fingerprint: async + blocking
  pings fine, `/health` self-probe fine, but in-flight requests stuck awaiting headers.
- **S4 outside the process** (docker-proxy / conntrack). Fingerprint: every in-process
  probe green while the browser times out.

### N2a plan — diagnostics only, no behaviour change (Tier 1; `diagnostics/watchdog.rs`)
1. **Request tracker layer** (outermost, also wraps the `ServeDir` fallback): registers each
   request (method, path WITHOUT query — queries can carry tokens, start time, phase
   `awaiting_headers` → `streaming_body`) in a registry; the guard lives inside a body
   wrapper that forwards `poll_frame` / `is_end_stream` / `size_hint` (must keep
   `Content-Length`, cf. B4), so a stuck body stays visible. WARN when headers took ≥10 s.
2. **Gauge** for live chunked-decrypt threads (S1's direct counter).
3. **Watchdog = a dedicated OS thread** (a tokio task dies with the wedge). Every 30 s:
   async ping (`Handle::spawn`, 5 s timeout), blocking-pool ping (`spawn_blocking`, 5 s),
   self-probe `GET /health` on 127.0.0.1:<port> (connect 5 s / response 10 s; plain HTTP
   only), `/proc/self` threads, RSS, FD count, tokio `num_alive_tasks` /
   `global_queue_depth`, in-flight counts + 5 oldest. INFO heartbeat every 10 min; WARN
   immediately on an anomaly, plus a thread table grouped by (name, state, wchan) at most
   once per 5 min while it lasts. Non-Linux: the /proc fields are absent, everything else runs.
- Tests (unit, RED first where there's a behaviour): /proc `status` + task `stat` parsing
  (comm with spaces/parens); anomaly classifier (pure); registry guard drop + oldest order +
  query stripped; body wrapper keeps `size_hint`; probe vs a listener that accepts and never
  answers ⇒ timeout, vs one that answers ⇒ ok; a 1-worker runtime whose worker is blocked ⇒
  async ping times out.
- Deploy: `-NoWipe -SkipAssets` after a verified DB backup (same procedure as 11-26).
- Rollback: revert the N2a commit + the 11-26 image-tag procedure (no schema, no data).

### N2a verified (2026-11-26, `af5ffca`)
- 18 new unit tests (full suite 640/640); clippy `-D warnings` + fmt clean. Mutations bite:
  dropping `size_hint` forwarding or the guard before the body ⇒ 2 inflight tests red.
- E2E `test_103_watchdog_heartbeat` (main.rs wiring) green; regression set test_01/02/03/09/
  12/23/91/93/102 = 202/202 with the tracker wrapping every body.

### Deploy 2026-11-26 #2 (N2a, `a1e29c9` → `af5ffca`, `-NoWipe -SkipAssets -SkipAndroid -SkipWebBuild`)
- Backup (verified: integrity ok, counts = live: 17,553 photos / 57,419 blobs / 8 refresh
  tokens / migr 42): CT132 `/root/sp-deploy-backups/n2a-20261126-062900/` (keyed
  `sqlcipher_export`, read-only attach; `SHA256SUMS`). Old image tagged
  `simple-photos-server:rollback-a1e29c9` (`daf0b90dc824`). APK + web unchanged.
- Rollback (inside CT132, instance dir): `git -C /opt/simple-photos reset --hard a1e29c9`;
  `docker tag simple-photos-server:rollback-a1e29c9 simple-photos-server:latest`;
  `docker compose up -d --force-recreate --no-build`. No schema change ⇒ no DB step.

### N2a NOT doing (and why)
- Not fixing S1 yet (e.g. a send timeout) — suspicion ≠ finding; the watchdog measures it.
- Not adding request/header timeouts or a connection cap — behaviour change before evidence.
- No admin endpoint for the watchdog — a wedged server can't serve it; logs are the channel.

## 4b. P1 — downloaded photos sort at the bottom (owner's top priority, 2026-11-26; Tier 2)

### Owner answers (2026-11-26)
- Rule: **everything outside the camera uses the date it landed on the phone**; camera
  photos keep their capture date.
- Existing wrong dates: **fix them too, dry-run first** (show which + how many, apply on OK).
- Read-only DB copy approved (taken, copy deleted afterwards).
- Deploy approved for this session once P1a–c are green (backup + rollback shown first);
  the repair is applied only after the owner OKs the dry-run counts.

### Measured (read-only keyed copy of the live DB, 2026-11-26 06:39Z, box `31429f5`)
- **5,003 of 5,146 Android-uploaded rows have `taken_at = 1970-01-01T00:00:00.000Z`**
  (4,934 with no camera make, e.g. `Tumblr_l_*.jpg`; 69 camera-named). Server-file rows
  (Takeout/scan, 12,407): 1 bad (`0000`).
- Mechanism (code): Android takes `MediaStore DATE_TAKEN` only (`SyncRepository.scanImages`
  / `scanVideos`); NULL for a downloaded file ⇒ `getLong` = 0 ⇒ epoch. Phone encrypts
  before upload, so the server never sees EXIF ⇒ the phone's value is final.
- Server `created_at` for those rows = upload time (e.g. 2026-08-05), NOT the phone date ⇒
  only the phone can supply the right date for existing rows.

### Decisions (mine, 2026-11-26)
- **Date rule (pure fn, `data/media/LibraryDate.kt`):** in the camera tree (`RELATIVE_PATH`
  under `DCIM/`) with `DATE_TAKEN > 0` ⇒ `DATE_TAKEN`. Everything else ⇒ the **earlier of
  `DATE_ADDED` and `DATE_MODIFIED`** (both seconds → ms): a fresh download has both ≈ the
  download time; a phone migration resets `DATE_ADDED` but keeps mtime; an edit bumps mtime
  but not `DATE_ADDED`. Non-positive values ignored; nothing usable ⇒ `DATE_TAKEN` if > 0,
  else now — never 0.
- **Repair is phone-driven** (only the phone knows folder + dates), server-guarded:
  `POST /api/photos/dates/repair` `{dry_run (default TRUE), items:[{photo_id, taken_at,
  expect_taken_at}]}`. Refuses an empty list; ≤500 items/call; own photos only;
  compare-and-set on `expect_taken_at` (a row someone changed since is skipped, counted);
  apply writes old→new into `taken_at_repairs` (migr 043, batch id) in ONE write txn, also
  recomputes `photo_year`/`photo_month`, emits change_log/sync so web + other devices
  re-sort, audit row with counts. `POST …/repair/undo {batch_id}` restores old values
  (compare-and-set on the new value). Dry-run answers per-item would-change/skip + counts.
- Android UI: Settings → "Fix photo dates" → dry run ⇒ "N photos will move (sample list)"
  ⇒ Apply. Room `takenAt` updated only for rows the server confirmed.
- Rows whose phone file is gone: counted and listed in the dry-run, NOT changed.

### Plan — one commit each
1. **P1a Android rule for new backups.** `LibraryDate` + `scanImages`/`scanVideos` query
   `DATE_ADDED`, `DATE_MODIFIED`, `RELATIVE_PATH`. JVM tests (RED first: today's rule gives
   0 for a dateless download).
   - [x] **P1a done (2026-11-26).** RED on a stub keeping today's rule: 12/14 failed (the 2
     passing = cases where `DATE_TAKEN` was already right). GREEN: 14/14; full JVM suite
     363/363; lint = pre-existing baseline only (1 `NavGraph` error, 127 warnings, unchanged).
     Extra rules decided while building: mtime < 2008-01-01 ignored (zip FAT 1980 stamps
     would win "earlier of"); `DATE_TAKEN` read via the per-type API-1 column (the
     `MediaColumns` one is API 29); API < 29 reads `DATA` and looks for a `/DCIM/` segment.
     **Risk until P1c is applied:** existing 1970 rows only change via the repair; a
     fresh-mirror rescan gives the new date while the server still says 1970, so the
     `filename + takenAt` merge fallback misses (hash path still matches). Gone once P1c
     writes the same `LibraryDate` value to the server.
2. **P1b server repair + undo endpoints + migr 043 (up/down, down actually run).** E2E
   `test_104`: dry-run changes nothing; empty list refused; other user's photo refused;
   stale `expect_taken_at` skipped; apply moves the row + year/month + change_log; undo
   restores exactly. Unit tests for the validation.
   **P1b design detail (decided 2026-11-26, before code):**
   - `server/src/photos/date_repair.rs`: pure `validate_items` + `repair(pool, user, items,
     dry_run)` / `undo(pool, user, batch, dry_run)` taking a pool, so unit tests drive the
     real SQL on a migrated temp DB; handlers are thin wrappers.
   - Per-item outcome: `change` / `unchanged` (already that instant) / `stale` (current ≠
     `expect_taken_at`) / `not_found` (missing OR another user's — same answer, no leak).
     Instants are compared as parsed ms, not strings (`.000Z` vs `Z` must not read stale);
     `expect_taken_at: null` matches a NULL column.
   - Validation (400, nothing touched): empty list, > 500, duplicate `photo_id`, unparseable
     date, new date ≤ 1970-01-01T00:00:00Z (the bug value itself), new date > now + 1 day.
     New value normalised to `YYYY-MM-DDTHH:MM:SS.mmmZ`.
   - Apply: ONE `begin_write` txn; each row re-read inside it (dry-run answer can be stale);
     `UPDATE … WHERE id AND user_id` sets taken_at, photo_year/month, `taken_at_offset =
     NULL` (a DATE_ADDED is a true UTC instant); old taken_at/year/month/offset → 
     `taken_at_repairs`. change_log rides the existing UPDATE trigger (033). After commit:
     `summary_cache.invalidate`, ONE `emit_sync("photo", batch_id)` (500 per-photo events
     would overflow the 256-slot channel), audit `photo_date_repair` with counts, info log
     naming the undo call.
   - Undo `{batch_id, dry_run (default TRUE)}`: empty id refused; unknown/other user's
     batch = 404; per row compare-and-set on `new_taken_at` (edited since ⇒ `stale`, kept);
     restores all four old columns exactly; marks `undone_at` (a second undo = 404).
   - **Found while exploring:** neither client adopts a changed `taken_at` on a row it
     already caches — web `syncReconcile` existing-row path and Android
     `syncFromServerEncrypted` update only crop/favorite/etc. So a server repair would be
     invisible on web. ⇒ **P1b-web** (own commit): reconcile adopts `taken_at` (server
     wins, like crop/favorite) + vitest. Android half goes into P1c.
   - Encrypted blob payloads still carry `taken_at: 1970` inside — server can't rewrite
     them. Only the legacy unregistered-blob path reads it; registered rows use the column.
   - [x] **P1b done (2026-11-26).** Unit 17/17 in `date_repair` (validation, classify, and
     dry-run/apply/undo/other-user/stale/reverse on a migrated temp DB); full unit suite
     657/657; clippy `-D warnings` clean. RED by mutation: stale check removed ⇒ the 2 stale
     tests fail; dry-run default flipped ⇒ `only_an_explicit_false_writes` fails; SELECT
     user-scope removed ⇒ `another_users_photo_is_not_found_and_untouched` fails (the
     UPDATE's own scope then refuses the batch). E2E `test_104` 5/5; neighbours
     test_95–97, 100–103: 24/24. Migration 043 reverse run by
     `the_documented_reverse_drops_the_table`. API_REFERENCE updated.
   - [x] **P1b-web done (2026-11-26).** `recordTakenAt` (= server `COALESCE(taken_at,
     created_at)`) shared by insert + existing-row paths; existing rows adopt it when the ms
     differ; unparseable server value never overwrites. RED first: the 2 adoption tests left
     `takenAt` = 0; GREEN 24/24 reconcile, full web 372/372, `npm run build` green (web has
     no lint config). Delta (`syncPass.ts:175`) and full walk (`:317`) both reach it.
   - [x] **P1b-guard (2026-11-26, found designing P1c).** `register-encrypted` returns the
     EXISTING row on a hash match, so a phone can hold the id of a Takeout/scan row whose
     date is right. Server now answers `protected` for any row with a non-empty `file_path`
     (the "not touching server-file rows" decision, enforced where the knowledge is). RED:
     check removed ⇒ both new unit tests fail; GREEN 659/659, clippy clean, test_104 6/6.
3. **P1c Android "Fix photo dates" screen** (dry run → apply), JVM tests for the candidate
   builder (camera keeps date, download gets date added, gone file excluded, unchanged rows
   excluded). Also: existing-row server merge adopts `taken_at` (server wins), and an
   "Undo last fix" action for the most recent batch.
   **P1c design detail (decided 2026-11-26, before code):**
   - Compare against the SERVER's value, not Room's: a hash-merged local row keeps its
     scan date while the server says 1970, so a Room compare would skip exactly the broken
     rows. ⇒ dry run walks `encrypted-sync` (metadata only, ~35 pages) for `taken_at`, and
     sends that exact string as `expect_taken_at`.
   - Only rows Room links to a phone file (`serverPhotoId` + `localPath`); no name/size
     guessing. Unlinked rows counted "not on this phone"; a URI MediaStore no longer
     resolves counted "file gone". Server additionally answers `protected` (P1b-guard).
   - Pure `data/media/DateRepairPlanner.kt` (JVM-tested): rule = `LibraryDate.forScanOrNull`
     (new — the scanner's "else now" fallback would re-date a photo to TODAY; repair skips
     it as "no usable date"); a date > now + 1 day is skipped too (server would 400 the
     whole chunk); equal ms ⇒ "already right".
   - `DateRepairRepository`: dry run = plan + server dry runs in chunks of 500 (counts +
     first 20 samples); apply = same chunks with `dry_run:false`, Room `takenAt` updated
     only for `change` outcomes, batch ids saved in prefs; undo-last = undo dry run →
     confirm → undo each batch, Room restored for `change` outcomes, prefs cleared.
   - UI: Settings card "Photo dates" (after Backup Folders): Check → dialog with counts
     and samples → Apply; "Undo last fix" when batches are saved. State machine in its own
     `FixDatesViewModel`.
   - Sync adoption: pure `ServerDate.adopt(existingMs, takenAt, createdAt)` used by the
     existing-row branch of `syncFromServerEncrypted` (JVM-tested; same rule as web).
   - **Found building:** the app's Retrofit Gson omits nulls ⇒ `expect_taken_at: null` would
     go out ABSENT ⇒ server 400s the whole chunk. `DateRepairItem` has its own TypeAdapter
     that writes that one null (`DateRepairWireTest`).
   - [x] **P1c done (2026-11-26).** JVM 17 new (planner 13, wire 4); full JVM 380/380; lint =
     baseline (1 `RestrictedApi` NavGraph error, 127 warnings; 0 hits in new files). RED by
     mutation: planner on `forScan` (else-now) ⇒ "never re-dated to today" fails; adapter
     removed ⇒ the explicit-null wire test fails. NOT device-tested yet — the dry run on the
     S21+ after deploy is the first real run.
4. Deploy (backup first) → owner runs the dry run on the S21+, sends the count → apply on OK.

### Deploy 2026-11-26 #3 (P1a..P1c, `31429f5` → `2d006dc`, `-NoWipe -SkipAssets`)
- Backup (verified: integrity ok, counts = live: 17,553 photos / 57,419 blobs / 10 refresh
  tokens / migr 42 / **5,003 rows at 1970**): CT132 `/root/sp-deploy-backups/p1-20261126-075249/`
  — keyed `sqlcipher_export` (read-only attach), old APK, `web-dist.tgz` (80 entries),
  `SHA256SUMS`. Old image tagged `simple-photos-server:rollback-31429f5` (`20c02f16b6e5`).
- Rollback (inside CT132): `git -C /opt/simple-photos reset --hard 31429f5`;
  `docker tag simple-photos-server:rollback-31429f5 simple-photos-server:latest`;
  in the instance dir `docker compose up -d --force-recreate --no-build`;
  web: `rm -rf /opt/simple-photos/web/dist && tar -C /opt/simple-photos/web -xzf $BK/web-dist.tgz`;
  APK: `cp $BK/simple-photos.apk /opt/simple-photos/downloads/`. Migr 043 is one new table
  ⇒ the old binary runs on the new DB. DB restore only if corrupted (stop, move
  `data/db/simple-photos.db*` aside, copy the backup in).
- **Verified 2026-11-26 07:57Z:** box HEAD `13b4e3c` clean, image `20ba0ba5671a` healthy,
  health 200, `/photos/dates/repair{,/undo}` = 401 unauthenticated (route + gate), migr 43,
  counts unchanged (17,553 / 57,419), 5,003 at 1970, 0 repairs, new APK (34,555,928 B),
  watchdog heartbeat probe=200. deploy-fresh's own backup: `/root/sp-deploy-backups/20261126-075440`.
- **Next (owner):** install the APK → Settings → Photo dates → Check → send the counts.
  Apply only on OK. Then web: hard-refresh and confirm the downloads moved off the bottom.
- Applied date repairs (after the owner's OK): Settings → Photo dates → Undo last fix, or
  `POST /api/photos/dates/repair/undo {batch_id, dry_run:false}` per batch (ids in the
  server log line "date repair: applied").

### P1 NOT doing (and why)
- No server-side guess (e.g. upload time) for rows the phone can't supply — a wrong date
  presented as right is worse than an obviously wrong one; owner can decide after the count.
- Not touching server-file rows (Takeout/scan resolver is correct; 1 bad row ≠ a pattern).
- Not reusing `metadata_edit` PUT for the repair — no dry-run, no undo, no batch.

### P1 rollback
P1a/P1c: revert (client-only). P1b: revert; migr 043 is one new table (down drops it).
Applied repairs: `…/repair/undo {batch_id}`, or restore the pre-deploy DB backup.

## 4c. P2 — secure gallery still wrong + make the date repair automatic (owner, 2026-11-26; Tier 2)

### Owner report (2026-11-26, after applying P1 on the S21+)
- "Fixed the issue with regular photos but not in the secure gallery."
- "We don't need a button to fix manually — this process worked, it should be automatic."

### Measured (read-only keyed attach of the live DB, box `13b4e3c`)
- Owner's Apply: 10 batches, **4,900 rows** re-dated 08:01Z, 0 undone.
- Still at 1970: **104 client rows, ALL secure** (the `original_blob_id` rows of secure
  items; 0 clone rows; filenames `tiktok_*.mp4`, `26638257.webp` …) + 1 regular.
- Why missed: the phone's Check walks `encrypted-sync`, which excludes secure photos by
  design (eligibility predicate) ⇒ they were never candidates.
- Why the secure grid looks wrong even with right dates: **neither secure feed returns a
  date and both `ORDER BY gi.added_at DESC`**; both clients keep server order. Items were
  bulk-added 2026-08-07 in the phone's (1970-sorted) order, so add order = the old wrong
  order. Repairing dates alone would not move them.
- Android keeps the Room row of a secured photo (hidden by `excludeSecure`) with its
  `serverPhotoId` and `localPath` ⇒ the phone CAN compute the landed date for them.

### Decisions (mine, 2026-11-26)
- **Secure feeds sort by the photo's date, newest first**, like the main gallery:
  `COALESCE(p.taken_at, op.taken_at, p.created_at, op.created_at, gi.added_at) DESC, gi.id`
  (`p` = server-side clone row, `op` = original). Both feeds also return that value as
  `taken_at`. Only consumer of the old order = smart-album cover ("first = newest"), which
  now means newest by date — same as the regular smart albums. Comments updated.
- **`GET /api/photos/dates/needs-repair`**: the caller's CLIENT rows (`file_path = ''`)
  with `taken_at LIKE '1970-01-01%'`, secure included (ids + dates only — the caller
  uploaded them; the repair dry run already returned dates for any own id). ≤ 2,000 per
  call; the next run takes the rest. Usually empty ⇒ one cheap request per run.
- **Automatic on the phone, no UI:** after each successful backup run, at most once per
  6 h: needs-repair → Room links → MediaStore → `DateRepairPlanner` → apply (chunks of 500,
  `dry_run:false`). Guards kept: server compare-and-set with `expect_taken_at` = the epoch
  string the server just listed (so an auto run can only ever replace a 1970 value — it can
  never fight a manual edit), `protected` server-file rows, undo log (batch ids logged +
  kept in prefs; undo via `POST …/repair/undo`). Phone drops any candidate whose expect is
  not 1970 before sending (second guard, tested). A failure is logged and never fails the
  backup.
- Settings card + `FixDatesViewModel` removed (owner: no button).

### Plan — one commit each
1. **P2a server:** secure feeds ordered by date + `taken_at` field; `needs-repair`
   endpoint. Unit tests + E2E `test_105` (secure order by date; needs-repair lists a secure
   1970 row, not server-file / other user / dated rows).
2. **P2b Android:** auto repair in the backup worker; remove the card; JVM tests (epoch-only
   filter, throttle decision).
3. Deploy (backup first). The first automatic run fixes the 104; verify count → 0 on box.

### P2 progress
- [x] **P2a server.** `SECURE_ITEM_DATE` / `SECURE_ITEM_ORDER` consts used by both feeds
  (+ `taken_at` in each item); `needs_repair` + `GET /api/photos/dates/needs-repair`. Unit
  663/663 (+4), clippy clean. RED: order const back to `gi.added_at DESC` ⇒
  `secure_items_sort_by_photo_date_not_by_when_they_were_secured` fails. E2E `test_105`
  3/3 + test_104 6/6 + test_94 13/13.
- [x] **P2b Android.** `AutoDateRepair` (6 h throttle, epoch-only guard) +
  `DateRepairRepository.autoRepairIfDue` called at the end of `BackupWorker` (try/catch,
  `diag` log ⇒ reaches `/api/admin/client-logs`). Settings card, `FixDatesViewModel`
  and the Android undo DTOs removed. JVM 383/383 (+3); lint = baseline. RED: guard
  weakened to `ms >= 0` ⇒ `only candidates replacing a 1970 value are kept` fails.
  Web: comment + `taken_at?` type only; 372/372, build green.
- Secure-related E2E sweep (13 files): 364 passed, 1 skipped, **8 failed — all pre-existing,
  none about order**: test_21 ×3 + test_41 ×1 already recorded failing on `8b7701f` (§2 F11);
  test_25 ×4 fetch a secure clone's file/thumb WITHOUT `x-gallery-token` ⇒ 401 from the B6
  secure-media gate (`8a1d736`) — same stale-test class as test_10 (fixed in `ae95a0f`). A HEAD
  worktree run was attempted and could not build (gotchas 2026-11-26). → follow-up F13 below.

### Deploy 2026-11-26 #4 (P2, `13b4e3c` → `19e9ac7`+, `-NoWipe -SkipAssets`)
- Backup (verified: integrity ok, counts = live: 17,553 photos / 57,419 blobs / 12 refresh /
  migr 43 / 105 at 1970 / 4,900 repairs): CT132 `/root/sp-deploy-backups/p2-20261126-084738/`
  — keyed `sqlcipher_export`, APK, `web-dist.tgz`, `SHA256SUMS`. Old image tagged
  `simple-photos-server:rollback-13b4e3c` (`20ba0ba5671a`).
- Rollback (inside CT132): `git -C /opt/simple-photos reset --hard 13b4e3c`;
  `docker tag simple-photos-server:rollback-13b4e3c simple-photos-server:latest`;
  instance dir `docker compose up -d --force-recreate --no-build`; web
  `rm -rf /opt/simple-photos/web/dist && tar -C /opt/simple-photos/web -xzf $BK/web-dist.tgz`;
  APK `cp $BK/simple-photos.apk /opt/simple-photos/downloads/`. No schema change.
- Automatic repairs it applies: `POST /api/photos/dates/repair/undo {batch_id, dry_run:false}`
  (batch ids in `taken_at_repairs`, the server log, and the phone's client log
  "Automatic date repair").

### P2 NOT doing (and why)
- Not fixing test_25's missing gallery token here (F13, separate test-only commit).
- No server-side guess for secure rows whose phone file is gone (same reason as P1).
- No client-side re-sort of secure items — server order stays the single contract.
- Not touching the 1 regular 1970 row by hand; the automatic run will take it if linked.

## 5. Verification debt carried from the old todo.md (still open)
Deploy-side (CT132): A1/#42 badge counts, A2/#38 migration 033 backfill, B2/#40 3-strike
cap + ETA calibration readout, B3/#46 codec backfill drains to zero, B4/#49 rung backfill.
Device (S21+): B4 quality swap, E1/#43, F1/#41, D2/#50, E3, E3a, Z1e, B5.
Browser: B4 web quality swap, D2 iOS safe-area, A3/#51 virtualization, E3 web, Z1d.
Full detail: `git show f6c9ded^:todo.md`, section "2. Verification debt".
