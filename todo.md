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

### C1(d) — Pet cluster tiles have no representative bbox (#48 remainder) — DONE 2026-08-03

`PetCluster` carried no `rep_bbox_*` anywhere. Pet tiles were circular (shipped
in `5c4d776`) but centre-cropped, and no client-side framing could fix that.

**The plan said "mirroring `fetch_face_clusters`", which understated it by a
table.** `face_detections` has had `bbox_x/y/w/h` since migration 017;
**`pet_detections` had no bbox columns at all** (migration 020 never defined
them) and the processor threw the box away at insert. So this was never a query
change — it was migration + processor + query.

- [x] Server: resolve and emit a representative pet detection bbox. Migration
      `039` adds the four columns; the processor now stores the originating
      object detection's box.
- [x] **Backfill, and it is exact rather than a guess.** `pet_detections` rows
      are *derived* from `object_detections` rows — the processor maps
      `class_name` to a species, dedups by species per photo, and copies that
      detection's `confidence` **verbatim**. `object_detections` stored the box
      all along, so migration 039 recovers it by joining back on
      (photo, user, mapped species) with confidence as an exact-match preference.
      An existing library gets framed pet tiles with **no AI re-run** — hours of
      GPU on a 15k library. Two passes, not one: SQLite resolves the UPDATE
      target's columns inside a subquery's `WHERE` but **not inside its
      `ORDER BY`**, so the one-statement `ORDER BY (od.confidence =
      pet_detections.confidence)` form fails to prepare.
- [x] **The eligibility join — and the item named it as a risk when it was
      already a live defect.** `list_pet_clusters` had *no eligibility filter of
      any kind*: a bare `COALESCE(pc.representative, highest-confidence
      detection)`. A pet cluster whose best detection sat on a secure-album photo
      pointed its tile at a photo the client must not render — the #48(b) bug the
      face path fixed in `9046915`, still live here. Shipping the bbox on top of
      that would have made it **worse**, not merely unfixed: an unrenderable id
      only placeholders, but a bbox tells the tile to crop *into* the secured
      photo. `fetch_pet_clusters` is now extracted and unit-tested like its face
      twin, and both share one `eligible_representative_sql` /
      `representative_bbox_join` — a sixth copy of one derivation was the
      alternative.
- [x] Wired `clusterFaceCropStyle` (web: PetsView **and** the Albums-page row,
      which also needed `relative` on the tile — the crop style positions
      absolutely) and `FaceCrop.kt` (Android) to pet tiles. The nullable-DTO
      guard is now `tileFaceBoxOf`, shared by the People and Pets screens for
      the reason web's doc already records: inlining it per call site is exactly
      how the Albums row lost its framing in #48(d).
- [x] `API_REFERENCE.md` — **the faces row was stale too**, having never gained
      `rep_bbox_*` when `5c4d776` shipped them. Both rows fixed, plus the null
      contract.
- [ ] **Device/browser check, deferred to the verification session below.** Every
      test here is JVM/unit; that a pet tile *visually* frames the animal has
      only ever run in a compiler. Needs a library with a clustered pet — and the
      pre-039 case matters as much as the backfilled one, since a null box must
      draw the old plain crop rather than a degenerate window.

---

### B3 — `needs_web_preview` still guesses by file extension (#46 remainder) -- DONE 2026-08-03 `fa12624`

[server/src/photos/web_preview.rs](server/src/photos/web_preview.rs) has the same
extension-only blind spot the codec probe fixed in `conversion_target`.
**Deliberately deferred from `6663a4c`,** and the reasoning still holds: wiring
the probe in turns a pure `fn(&str) -> Option<&str>` into an async, path-taking
probe touching every caller in `server_migrate_encrypt.rs` — a separable refactor
with its own risk, for a path that only handles on-the-fly web previews. A fresh
install re-ingests everything through the already-fixed probe.

**Both halves of that deferral note were wrong, and the second one inverted the
stakes.** This is not "on-the-fly web previews": its only consumer is
`server_migrate_encrypt`, so whatever it picks is the payload that gets
*encrypted at rest* and handed to every client for the life of the photo. And
"a fresh install re-ingests everything through the already-fixed probe" does not
rescue the already-registered offenders: the #46 backfill (`6663a4c`) adds a
*rendition* and deliberately leaves the original blob alone, so those rows are
still `.mp4`-named HEVC. The ladder hides that in the main gallery; a secure
gallery item carries no renditions at all (`list_gallery_items` never joins
them -- B4 below records the same fact from the other side), so there the
unplayable payload is all there is.

The cost estimate was the load-bearing argument for deferring, and it did not
survive contact either: all three callers are in **one file**, already `async`,
and already holding the absolute path.

- [x] Probe video containers instead of trusting the extension.
      `resolve_web_preview(filename, stream_is_native)` is pure and holds the
      whole matrix; `plan_web_preview` spawns the one ffprobe, for
      `.mp4`/`.mov`/`.m4v` only. It reuses `transcode::probe` -- the same probe
      `ingest::opaque_container_needs_conversion` runs at scan time. A second
      derivation of "is this browser-native" was the alternative, and the
      cross-cutting risks below already count five instances of that mistake.
- [x] **There were two blind spots, opposite signs, and the item named only
      one.** False negative: HEVC / MPEG-4 Part 2 / 10-bit H.264 inside a `.mp4`
      was stored verbatim, unplayable. False positive: `.mov`/`.m4v` was
      *always* transcoded, so ordinary phone H.264 got a full re-encode baked
      permanently into the stored payload. Native streams in those containers
      are now **remuxed** (`-c:v copy`) -- lossless, and seconds instead of
      minutes. Audio is re-encoded to AAC rather than copied: a QuickTime
      container can legally carry PCM/ALAC, an MP4 wrapper will accept it, and
      no browser plays the result. A refused stream copy falls back to the full
      transcode rather than leaving the payload unplayable.
- [x] `.webm` is deliberately **excluded** from the probe. `is_browser_native`
      is an H.264-only allowlist, so a VP9 WebM -- which every target browser
      plays -- would come back "not native" and be re-encoded for nothing. WebM
      has never been previewed here and still is not.
- [x] An unprobeable file (ffprobe missing, timeout, no video stream) falls back
      to the extension verdict, so a broken environment degrades to the *old*
      behaviour rather than to an unplayable payload or a library-wide
      re-encode. An audio-only `.mp4` lands here and is correctly stored as-is.
      Both pinned by tests.
- [x] Tests: 12 new, 478 green (was 466). Four of them -- including both real
      FFmpeg fixtures -- verified RED by forcing `preview_needs_probe` to
      `false`, which reproduces the old extension-only path exactly; the eight
      preserved-behaviour tests stayed green in that same run.

---

### B3b -- `is_opaque_video_container` and its test assert something false -- DONE 2026-08-03

Found while fixing B3, in a surface that item never named.
`probe::is_opaque_video_container` documents itself as "exactly the extensions
`conversion_target` treats as already-native", and its test
`opaque_containers_are_the_ones_conversion_target_skips` asserts that for
`.mov`/`.m4v`. But `conversion_target` returns `Some(mp4)` for both, so at
[server/src/ingest.rs:1084](server/src/ingest.rs#L1084) the probe branch
(`None => opaque_container_needs_conversion`) is **unreachable** for them.

**The item named the harmless half and missed the harmful one.** The set was
`{mp4, mov, m4v, webm}`; `conversion_target` skips `{mp4, webm}`. So `.webm`'s
probe branch was **reachable**, and behind it sits `is_browser_native` -- an
**H.264-only allowlist**. Every newly scanned VP9/AV1 WebM was therefore judged
"not native" and queued for a **full re-encode into H.264 MP4**, replacing a file
every target browser plays. That is the identical false positive B3 had just
fixed one file over in `web_preview`, and B3's own doc even named the hazard
("a VP9 WebM would be reported not native and re-encoded") while deferring "the
blast radius at ingest" -- without stating that ingest was *already paying it*.

The proof it was a bug and not a policy is an internal contradiction:
`media::MEDIA_EXTENSIONS` already lists `.webm` as universally playable, and an
**already-registered** `.webm` is skipped by `existing_set` and served untouched.
The same file was treated two different ways depending only on when it arrived.

- [x] Narrowed `is_opaque_video_container` to **`mp4` only**, and rewrote the doc
      to state the real rule, which is **two** conditions the old one collapsed
      into one: `conversion_target` must skip it (else the probe never runs) *and*
      `is_browser_native` must be able to adjudicate it (else the probe can only
      return the wrong answer). `.mov`/`.m4v` fail the first, `.webm` the second.
- [x] **Rejected dropping `mov`/`m4v` from `conversion_target`**, which this item
      called "the better behaviour". It is not, as stated: the ingest path has no
      remux verdict (`OpaqueVerdict` is convert / leave / unplayable), so a native
      `.mov` would be **left** as `video/quicktime` -- which Chrome and Firefox
      refuse to play. The item's parenthetical ("would then be remuxed at ingest")
      assumed a code path that does not exist there. It would also relocate `.mov`
      playability onto the encryption stage, which **B3a just proved is not a
      guarantee** (2,500 rows parked as plaintext originals). Doing it properly
      means adding an `OpaqueVerdict::Remux` and wiring the upload path -- a real
      feature, not a list edit. Recorded here so it is not re-proposed.
- [x] **Third copy of the list, found while fixing this one.**
      `rung_queue::find_codec_backfill_candidates`' SQL filtered
      `%.mp4/%.mov/%.m4v/%.webm` under a comment claiming it "mirrors
      `is_opaque_video_container`". Its verdict is `source_rung_is_offerable` ->
      the same H.264-only allowlist, so every WebM row bought a source-resolution
      re-encode it never needed -- additive (a rendition, not a replacement),
      which is why it hid longer. `.webm` dropped there too; `mov`/`m4v` **kept**,
      because that query reads *registered filenames* where a `.mov` legitimately
      survives, unlike the ingest walk. The two lists answer different questions;
      the "mirrors" comment was rewritten to say so, since it was an open
      invitation to re-merge them.
- [x] Tests: 481 green (was 478). Four verified RED on the pre-fix tree -- the
      ingest one prints the defect verbatim
      (`got Convert { target: ... "mp4" ... }` for a real VP9 fixture). The
      rung_queue test **asserted the bug** (`p_webm` in the expected set), the
      same shape as the #48 face-centering suite. The probe test now cross-checks
      against `conversion_target` itself rather than a copied list, which is how
      it drifted in the first place.

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
  (`allPhotos` re-derived from `allPhotosRaw` on every in-viewer edit), and B3b
  (**three** copies of "which video containers are opaque" — ingest's
  `is_opaque_video_container`, the ladder backfill's SQL `LIKE` list, and
  `web_preview`'s `preview_needs_probe`; B3 fixed exactly one and left the other
  two disagreeing with it). The fix is usually the same shape: one function,
  called twice. **Six instances now — when a second surface needs the same list,
  wire it to the first one's resolver in the same commit, not later.**
  B3b is the exception that proves the rule's limit: those three lists
  *legitimately* differ, because they answer different questions (unregistered
  walk vs. registered rows vs. encryption-time filename). When they must differ,
  the comment has to say **why**, or the next tidy-up re-merges them — B3b's
  "mirrors `is_opaque_video_container`" comment was exactly that invitation.
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
