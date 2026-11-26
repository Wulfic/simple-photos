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
- [ ] **F12 tests (found by F9's run, PRE-EXISTING, masked by the cascade)** test_10
      `TestRecoverySecureGallery` (6 setup errors) + `TestRecoveryPresyncedGallery::
      test_presynced_gallery_decryptable_after_recovery`: `download_blob` of a secure-gallery
      blob → **401 on the primary, before any sync/recovery** (`test_10:559`, `:1065`). Same
      shape as test_06 before `e44ad10`: no `x-gallery-token` since the B6 gate (`8a1d736`). Fix
      the tests to present the token + assert the gate still refuses without it — then see what
      the recovery assertions behind them say (they may be F10).
- [ ] **F10 server (found 2026-11-25 reading for F9, PRE-EXISTING since `b5d722d` 2026-04-05) —
      UNVERIFIED until RED.** `recovery_engine.rs` ~L254: the recovery `INSERT … SELECT` string
      is spliced into itself (`…AND id != ?1sh, crop_metadata) SELECT …`) → SQLite parse error at
      runtime → every recovered photo is logged + skipped. Suspicions to prove/disprove:
      (a) recovery registers 0 photos (does test_10 show it, once F9 stops the cascade?);
      (b) the error and `rows_affected()==0` branches `remove_file(full_path)` on the path they
      just wrote — if a local row already owns that path (the autoscan case the comment
      describes), that deletes a live row's file; (c) recovery `download`s every missing row,
      so `file_path=''` rows (Android) can't be recovered by pull at all.
- [ ] **F11 (suspicion, unverified)** `soft_delete_photo` does not copy `encrypted_blob_id` /
      `encrypted_thumb_blob_id` into `trash_items`. Does restoring a trashed Android
      (`register-encrypted`) photo produce a row with no file AND no blob id?

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

## 4. Verification debt carried from the old todo.md (still open)
Deploy-side (CT132): A1/#42 badge counts, A2/#38 migration 033 backfill, B2/#40 3-strike
cap + ETA calibration readout, B3/#46 codec backfill drains to zero, B4/#49 rung backfill.
Device (S21+): B4 quality swap, E1/#43, F1/#41, D2/#50, E3, E3a, Z1e, B5.
Browser: B4 web quality swap, D2 iOS safe-area, A3/#51 virtualization, E3 web, Z1d.
Full detail: `git show f6c9ded^:todo.md`, section "2. Verification debt".
