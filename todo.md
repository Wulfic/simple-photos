# TODO — Open work

**Completed work lives in [todo-completed.md](todo-completed.md).** Split out
2026-07-30, when the code side of the #38–#52 batch closed. That file is not an
archive you can ignore: **fourteen of its plans were wrong before they were
right**, and the correction blocks next to them are the only record of why. Read
the relevant section there before re-planning anything that touches the same
code.

This file holds **only what is not done**: one live-data cleanup, five follow-ups,
and the standing deploy/device verification debt.

**Ground rules (unchanged, non-negotiable):**
- One commit per issue, conventional commit, referencing `(#NN)`.
- Every fix ships with a test that FAILS before the fix. No exceptions.
- Never commit red. `cargo test --bin simple-photos-server`, `npm test` in
  `web/`, `.\gradlew.bat :app:testDebugUnitTest`, and the `tests/` pytest E2E
  suite must be green.
- Known-red baseline before you start (memory `e2e-preexisting-failures-2026-07-15`):
  `test_06` secure 401s (8), `test_20` dates (4), `test_58` Windows harness bug,
  `test_18` audio 403s (6 — `audio_backup_enabled` defaults false). Do not blame
  your diff for those; do not let them grow.

---

## 1. Code

### B3a — Parked (permanently unencrypted) photos — CODE DONE 2026-07-30

Found 2026-07-22: **2,500 of 15,014 live photos (~17%) were `encryption_deferred
= 1`** — parked after three hard encryption failures (the pre-chunked OOM, ~5x
RAM), never retried, sitting as **plaintext originals at rest**.

**The live evidence changed the item. CT132 was wiped and reinstalled
2026-07-22 18:06Z** (DB birth time; all 38 migrations at 18:06:33; first photo
row 18:14Z) — *after* the 05:51Z verification in memory
`issue46-rethrash-parked-2026-07-22`, which is why that memory reads as current
and is not. Live state now: 12,856 photos, **0 unencrypted, 0 parked,
`encryption_attempts = 0` on every row**, 235 blobs on the chunked v2 path.

- [x] Confirm the failure cause per row. Per-row `encryption_error` was
      destroyed by the wipe — but **superseded by a stronger result**: the same
      source library re-ingested through the chunked path with zero failures and
      zero parks, and 235 files cleared the >32 MiB threshold that used to OOM.
      That is a re-run of the experiment, not a guess. **Hypothesis confirmed.**
- [x] ~~One-shot un-park migration~~ — **rejected, deliberately.** Nothing is
      left to un-park on any reachable box, so a blanket
      `UPDATE … SET encryption_deferred = 0` migration would be a no-op
      everywhere we can observe, unverifiable by construction, and — being
      one-shot — useless against the *next* backlog, which is the failure mode
      this item is actually about. Replaced by the operator action below.
- [x] Surface the parked count **and** give it a remedy. These were two halves
      of one feature: a count nobody can act on is half a fix, an un-park nobody
      can see the need for is the other half. Shipped:
      `parked` on `/api/status/encryption` (held **outside** `pending`/`total`/
      `done` — the exclusion that keeps the banner from wedging is correct and
      stays; being reported *nowhere else* was the bug); `unencrypted_count` +
      `parked_count` in admin Diagnostics with a red card and a Retry button; a
      terminal `encryption_parked` audit event mirroring #40's
      `conversion_retired`; and `POST /api/admin/encryption/retry-parked`,
      the counterpart to `/admin/conversion/retry-failed`.
- [ ] **Live verify — blocked on the redeploy below.** On a clean box the
      expected reading is `parked: 0` everywhere. The real check is the
      round-trip: flip one row to `encryption_deferred = 1`, confirm Diagnostics
      turns red and `/status/encryption` reports `parked: 1` **without**
      `pending`/`total` moving, hit Retry, confirm it drains to 0 and the photo
      re-encrypts. A genuinely corrupt original must re-park **terminally, not
      loop** — the 3-attempt cap is untouched, so this is a check, not a hope.

> **Found while fixing it, in a surface the item never named:** the admin
> Diagnostics "Encrypted" card ran `SELECT COUNT(*) FROM photos` — the *same
> query as `total_photos`*, no predicate — so it reported 100% coverage
> unconditionally and read "15,014 of 15,014 encrypted" on the very library that
> had 2,500 plaintext originals. Fixed here.
> `encrypted_count_does_not_just_echo_the_total` asserts the two counts *differ*,
> because a single-row fixture passes against the broken query.

---

### C1(d) — Pet cluster tiles have no representative bbox (#48 remainder) — server task

`PetCluster` carries no `rep_bbox_*` anywhere — not on the server, not in
[web/src/api/ai.ts](web/src/api/ai.ts), not in Android's `AiDto.kt`. Only
`FaceCluster` does. Pet tiles are circular (shipped in `5c4d776`) but
centre-cropped, and no client-side framing can fix that.

- [ ] Server: resolve and emit a representative pet detection bbox, mirroring
      `fetch_face_clusters`. **Include the eligibility join from `9046915`** —
      the pet path will otherwise ship the exact secured-representative defect
      the face path just had fixed.
- [ ] Then wire the existing shared `clusterFaceCropStyle` / `FaceCrop.kt` (both
      already parameterised) to pet tiles on both clients.

---

### B3 — `needs_web_preview` still guesses by file extension (#46 remainder)

[server/src/photos/web_preview.rs](server/src/photos/web_preview.rs) has the same
extension-only blind spot the codec probe fixed in `conversion_target`.
**Deliberately deferred from `6663a4c`,** and the reasoning still holds: wiring
the probe in turns a pure `fn(&str) -> Option<&str>` into an async, path-taking
probe touching every caller in `server_migrate_encrypt.rs` — a separable refactor
with its own risk, for a path that only handles on-the-fly web previews. A fresh
install re-ingests everything through the already-fixed probe.

- [ ] Do it as its own commit, or delete the item and record the decision. Do not
      leave it half-deferred forever.

---

### A2 — Tombstone retention (#38 remainder)

`photo_change_log` rows for deleted photos accumulate without bound.

- [ ] Pruning needs a **policy**, not just a `DELETE` — a client offline longer
      than the retention window must be forced through a full reconcile, which is
      what `head_seq`/`total` exist for. Prune without that branch and a
      long-offline client silently keeps rows the server deleted, forever.
- [ ] Not urgent at current library sizes. Do not forget it.

---

### B2 — Persist measured per-category conversion rates (#40 remainder)

The ETA's seed rates only govern a machine that has never converted anything —
but today **every server boot is that machine**.

- [ ] Persist the last measured rate per category so the seeds retire on any box
      that has run one pass. The seeds are conservative and decay on the first
      sample, so this is an accuracy win, not a correctness fix.

---

### B4 — Ladder loose ends (#49 remainder)

- [ ] **Cost control.** The sweep is **serial** — sequencing, the wall-clock
      budget and the shared thread budget all landed in `60d555d`, but a
      114-file backlog of mostly-4K sources still drains slowly. Decide
      deliberately: give it a lane of the existing two-lane parallelism
      (`SIMPLE_PHOTOS_CONVERSION_JOBS`) or keep it serial **and write down why**.
- [ ] **Securing a video removes its picker.** The sync feed is the only delivery
      path and secure photos are not in it, so the secure viewer shows a
      single-quality video where the main gallery showed a choice. Not a
      regression (there was no picker before) and arguably correct — decide
      whether the secure gallery's own item listing should carry the ladder too.
- [ ] **E2E: rendition serving + range requests.** The ladder arithmetic and the
      picker default-per-network-state are unit-tested; the *serving* path with a
      real `Range` header is not.

---

### B2 — E2E for the 3-strike conversion cap (#40 remainder)

- [ ] A fixture that always fails must be attempted **exactly 3 times across 5
      real scan passes**. The unit + DB tests pin the arithmetic and the SQL;
      nothing yet drives five actual autoscan passes end to end. **Use a no-row
      path** — a test built on an ordinary failing conversion passes vacuously
      via `existing_set`, which is the trap recorded in B2's correction block.

---

## 2. Verification debt

Everything below is code that is written, tested and merged but has never run
where it matters. It is grouped because it wants one session with a live box and
a phone, not ten separate ones.

### Deploy (CT132)

**Before any of this: confirm what is actually deployed.** On 2026-07-22 the box
was found 4 commits behind `dev` while we were "testing the deployed backfill".
Check the box's `git HEAD` **and** a log-format fingerprint — container uptime
proves nothing about which code is in it.

**Box state as of 2026-07-30:** checkout `b339a32` on `dev`, image built
2026-07-22 18:03Z, container restarted 2026-07-29 18:42Z, **DB wiped and
re-created 2026-07-22 18:06Z** (12,856 photos, fully encrypted). So the box runs
`b339a32` and every fix merged after it — E3, E3a, B5, B3a — has never run here.

- [ ] Redeploy `dev`. **Also unblocks B3a's live verification** (above), which is
      the only part of B3a still open.
- [ ] **A1/#42** — the 29 photos lost to page boundaries and the badge numbers do
      not change until the box runs the fix.
- [ ] **A2/#38** — migration `033` backfills on first boot against a ~15k-row
      library. Cheap, but it is the first migration here with a data backfill.
      Watch it.
- [ ] **B2/#40** — the 3-strike cap only starts counting after the redeploy, and
      the Takeout duplicate-transcode loop is worth measuring before/after: it is
      pure wasted CPU today.
- [ ] **B2/#40 ETA** — watch a real mixed pass and compare the reported ETA with
      the actual drain time. Whether the seeded video rate is within ~2× of this
      hardware has never been measured.
- [ ] **B3/#46** — re-verify the codec backfill actually **drains to zero**. It
      has never run on the live box, and "never settles to zero" is how the
      re-thrash was caught last time.
- [ ] **B4/#49** — let the 136-file rung backfill drain (126 × 3840x2160, 4 × 8K;
      the 8K files are a 2-rung decision each, not a background afterthought).
      **Nothing has generated a rung on the live box yet**, so no picker can
      appear on any client until this drains.
- [ ] **A3/#51 server half — still HYPOTHESIS.** Both clients are now shown
      virtualized and memory-bounded, so measure **server** memory during a long
      scroll against the full library. A thumbnail request storm is the remaining
      plausible mechanism; it can no longer be blamed on the client.

### Device (S21+ harness, `.device-test\dev.ps1`)

Every item here is Android UI that has only ever run in a compiler. Compose UI
tests live in `androidTest` and need a device — that is the whole reason this
list exists.

- [ ] **B4/#49** — the quality swap: playhead restore, pause-state restore, the
      picker actually appearing. Needs a >1080p video **with a generated rung**,
      so it is blocked on the deploy above.
- [ ] **E1/#43** — the selection → dialog → move gesture in a secure album.
- [ ] **F1/#41** — two real windows count as two, and killing one re-enables the
      menu entry.
- [ ] **D2/#50** — the video controls clear the nav bar on **both** 3-button and
      gesture nav (different inset heights).
- [ ] **E3** — the pager order against the grid order in a sorted album, with a
      secured member and a burst present. `AlbumPhotoResolverTest` pins the
      resolver; only a device proves the two screens actually render it.
- [ ] **E3a** — the whole point of the item, and the part no JVM test reaches.
      Open a photo from **each** of Search, People, Pets, Memories and Trips.
      Four of the five never worked at all (server ids were handed to a `localId`
      lookup), so this is a first run, not a regression check:
      1. The tapped photo opens — not "Photo not found", not a stranger's photo.
      2. Swiping follows **that grid's** order (relevance / cluster / trip), not
         the gallery's `takenAt DESC`.
      3. Rotate mid-pager: the order and the page survive (the handoff rides
         `SavedStateHandle`, and the `contains` guard is what keeps the copy
         one-shot).
      4. A secured member of a face cluster is absent from the pager, and tapping
         its tile says "Photo not found" rather than opening it.
- [ ] **B5** — the fail-closed gate, which is the one part of B5 a unit test
      cannot reach. Three states, and the middle one is the whole point:
      1. **Server up** — the grid renders as before, no new latency.
      2. **Server unreachable, app already run once** — the grid still renders
         and still hides the secured photos, from the persisted set. This is the
         offline-browsing property of #3/#8; if it spins here, B5 traded a leak
         for a regression.
      3. **Fresh install, secure endpoint failing (not 404)** — the grid holds
         its spinner instead of drawing the library unfiltered, and recovers
         within one 3s poll of the server returning.
      Also confirm the viewer shows the *filter* message, not "Photo not found".

### Browser

- [ ] **B4/#49 web** — the quality swap (playhead, pause state, no flash of the
      original). This repo has no jsdom; there is no substitute for opening it.
- [ ] **D2/#50 iOS/PWA** — the safe-area half on a **notched** device. Every
      `env()` value is 0 on the dev machine, so the desktop-identical rendering
      proves the no-op property and nothing else.
- [ ] **A3/#51** — observe the virtualization against a real large library, not a
      fixture.
- [ ] **E3 web** — still **believed correct by construction, never verified in a
      browser**, and the Android half is now fixed so this is the only unchecked
      claim left. The album views hand the viewer the *resolved* array
      ([RegularAlbumView.tsx:545-551](web/src/pages/albumDetail/RegularAlbumView.tsx#L545-L551),
      [SelectablePhotoGrid.tsx:177-183](web/src/components/gallery/SelectablePhotoGrid.tsx#L177-L183))
      and [Viewer.navigateToPhoto](web/src/pages/Viewer.tsx#L426-L442) pages
      exactly that, re-deriving nothing; `JustifiedGrid` hands `renderItem` a true
      global index so #51's virtualization does not offset it. Open a sorted album
      (date asc, then name), swipe, back-nav, re-check. Record the result here —
      this file has already been burned once by an unchecked `[ ]` and once by a
      comment.

---

## Cross-cutting risks (still live)

- **Burst collapse is the recurring trap.** Every count / sort / ETA / **pager**
  change must state explicitly whether it operates on raw rows or collapsed
  tiles. Most of the historical count bugs in this repo are exactly this
  confusion. E3's burst divergence was it again — and the *plan* for E3 got the
  direction backwards, because it reasoned about the two Android surfaces without
  checking which one web already agreed with.
- **Two derivations of one list will drift.** #42 (three definitions of "count"),
  #48(a) (two copies of one crop formula), #51 (two owners of one blob URL), and
  E3 (two derivations of one album list), and E3's own `setRawPhotos`
  (`allPhotos` re-derived from `allPhotosRaw` on every in-viewer edit). The fix is
  always the same shape: one function, called twice. **Five instances now — when a
  second surface needs the same list, wire it to the first one's resolver in the
  same commit, not later.**
- **Verify the *id space*, not just the call shape.** E3a's audit read five call
  sites, saw they all navigated identically, and filed the difference as
  "ordering". Four of them were in fact passing **server** photo ids to a lookup
  keyed on `localId` and had never once opened the right photo. Both are `String`,
  so nothing failed loudly and the screen still rendered *something*. The repo has
  three id spaces in constant traffic — `localId`, `serverPhotoId`, `serverBlobId`
  — and `src-` album ids as a fourth. **When two surfaces exchange an id, name
  which space it is in, in the signature or the doc.**
- **The `src-` album id formula lives in three codebases** (memory
  `takeout-album-phases-2-3`). Never tidy one copy alone.
- **Do not regress the idle disk-thrash fix** (memory
  `idle-disk-thrash-investigation`). The steady-state "zero downloads when
  nothing changed" property is a hard requirement, not a nice-to-have.
- **A comment that asserts the old intent is a defect.** D1/#44, B2/#40, and now
  E3 all shipped with a comment claiming the opposite of the code. Update the
  comment in the same commit as the behaviour.
