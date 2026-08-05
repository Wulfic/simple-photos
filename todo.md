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

> **✅ CI debt cleared 2026-08-04 — and "Actions is broken" was the wrong
> diagnosis.** Actions did not *stop* on `dev`; it was **switched off, on
> purpose, in two steps**:
> - `5379845` (2026-06-02) removed `dev` from `ci.yml`'s push triggers and
>   handed dev validation to `sync-dev-to-main.yml` ("Gate & Sync"), which ran
>   the full suite on every `dev` push.
> - `2f31460` (2026-06-11) **deleted that orchestrator** as part of the unified
>   release pipeline, and `58f1d6c` stripped `ci.yml` down to
>   `on: workflow_call`. [pipeline.yml](.github/workflows/pipeline.yml) states
>   it in its own header: "Regular commits to `dev` or `main` do NOT trigger
>   anything."
>
> So nothing was failing, and there is no Actions outage to chase. The drift
> below is the *predicted consequence* of removing the gate, which is why it
> accumulated silently. All of it verified against CI's own pinned toolchain
> (`RUST_TOOLCHAIN: "1.88"`, installed here — `cargo +1.88 …`):
>
> - `cargo fmt --all -- --check`: **112 diffs across 20 files.** Fixed by
>   `cargo +1.88 fmt --all`. The tree now reads clean under **both** 1.88 and
>   1.96 — which is the proof that memory
>   `issue40-b2-rate-calibration-2026-08-04`'s "local 1.96 vs CI's 1.88 churned
>   20 untouched files" was wrong. They were the *same* 20 files and they were
>   genuinely unformatted; nobody's rustfmt was churning anything. Corrected in
>   that memory. Still use `cargo +1.88 fmt` — not because the two disagree
>   today, but because rustfmt ships with the toolchain and 1.88 is what the
>   gate runs.
> - `cargo clippy`: **four failures, not one.** This file counted only
>   `items_after_test_module` — real (`mod rendition_tests` had four
>   `pub async fn` handlers after it; moved to the end of
>   [serve.rs](server/src/photos/serve.rs)) — and stopped reading there. Behind
>   it sat three `uninlined_format_args`: the two `-enc-` ETag `format!`s in
>   `serve.rs`, and one in [setup/storage.rs](server/src/setup/storage.rs).
> - **One of those four is invisible to CI, and still is.** The `storage.rs`
>   lint sits under `#[cfg(target_os = "windows")]`; every runner here is
>   `ubuntu-24.04`, so no Linux clippy ever compiles it. It was found only
>   because this machine is Windows. The release pipeline *builds* the `.exe`
>   but does not clippy it, so a Windows-only lint has **no gate whatsoever**.
>   Fixed by hand. If it recurs, the fix is a `windows-latest` lint job; not
>   added now, to keep the new gate cheap.
>
> **The gate is back, deliberately smaller than the old one:**
> [.github/workflows/lint.yml](.github/workflows/lint.yml) runs `fmt --check` +
> `clippy -D warnings` on every push to `dev` and every PR — a couple of
> minutes, not the full chain. Build / unit tests / web / E2E stay in the
> tag-triggered release pipeline, which is slow on purpose. `ci.yml` was **not**
> given its push triggers back: it documents itself as having none, and
> re-adding them would make it run twice during a release. The two invocations
> in `lint.yml` are byte-identical to `ci.yml`'s on purpose — a gate that runs a
> *slightly* different check is worse than no gate, because it goes green while
> the release fails.
>
> **Residual, deliberately not closed here:** `cargo test`, `npm run build` and
> the pytest E2E suite still have no pre-tag gate. "Never commit red" is
> enforced on those by running them locally, exactly as before.
>
> **⚠ Correction, 2026-08-04 — the sentence above about the release pipeline was
> wrong, and in the reassuring direction.** This block claimed "Build / unit
> tests / web / E2E stay in the tag-triggered release pipeline." Read
> [ci.yml](.github/workflows/ci.yml) end to end: of the four suites the ground
> rules at the top of this file name, **the tag pipeline runs exactly one.**
> - `rust` — `fmt`, `clippy`, `build`, `cargo test`. Genuinely gated. ✅
> - `web` — `npm ci` and `npm run build`. **`npm test` is never invoked**, so the
>   vitest suite has no gate anywhere.
> - `python-tests` — named **"Python E2E (smoke)"**, and its two steps are
>   `pip install -r tests/requirements.txt` and a `grep` asserting the pins use
>   `==`. **It never runs pytest.** The job is green whenever the dependency
>   file parses.
>
> So the E2E suite has never been gated by anything, on any branch, at any
> point — which is the context for the run below. **A job named after a suite it
> does not run is worse than no job**, for the same reason `lint.yml` was made
> byte-identical to `ci.yml`: it reports green and buys nothing. Not fixed here
> (turning the E2E suite on in CI is its own workstream — it needs ffmpeg, the
> AI models, and a run budget), but the name is a lie today and this file should
> stop repeating it.

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

### A2 — Tombstone retention (#38 remainder) — DONE 2026-08-03 `ea45f9c`

`photo_change_log` rows for deleted photos accumulate without bound.

**The premise was half right, and the imprecise half matters.** `photo_change_log.photo_id`
is the **PRIMARY KEY** — one row per photo, `seq` bumped in place — so the log
does *not* grow per change, and for live photos it is bounded by the library
size. Migration `033` says so in a comment the item never reconciled with.
What actually accumulates is only the **tombstone**: one row per photo ever
deleted, surviving by design (no FK — evidence that cascades away is not
evidence). So the growth rate is "photos you have deleted", not "changes you
have made", which is why this was correctly filed as not urgent.

- [x] Pruning as a **policy**. `gallery/retention.rs`: prune tombstones older
      than `TOMBSTONE_RETENTION_DAYS` (90), record how far the prune reached as
      a floor in `server_settings`, and have `fetch_delta` refuse any `since`
      below that floor. One `VICTIM_PREDICATE` const feeds both the reach probe
      and the `DELETE`, so the two cannot drift. Hourly, in `spawn_housekeeping`
      — but in **its own transaction**, since a failure in the audit-log trim
      must not discard a floor whose rows are already gone.
- [x] **The forced full reconcile needed zero client work, and not via the
      mechanism the item named.** It is not `head_seq`/`total`: it is the
      `deleted` handshake. Both clients already treat an **absent** `deleted`
      array as "this server did not honour `since` — restart as a full walk"
      ([syncPass.ts:151](web/src/gallery/hooks/syncPass.ts#L151),
      [PhotoRepository.kt:1118](android/app/src/main/kotlin/com/simplephotos/data/repository/PhotoRepository.kt#L1118)),
      built for the pre-#38-server case. A beyond-retention client wants the
      identical recovery, so it reuses the branch: the fallback just calls
      `fetch_page`, whose `deleted` is `None`. The `after` cursor is dropped —
      a `"<seq>|<id>"` delta cursor is meaningless to the full walk's
      `"<timestamp>|<id>"` keyset, and neither client resumes there anyway.
- [x] **Head monotonicity — a hazard this item never named, and the one that
      would have caused real data loss.** Every trigger in `033` computes
      `MAX(seq) + 1`. Prune the highest-seq row and the next change **reuses a
      sequence synced clients have already passed**, so that change is invisible
      to them forever. The victim predicate therefore excludes the current
      maximum unconditionally, age irrelevant; it is one row. The realistic
      trigger is mundane: delete a photo, then don't touch the library for three
      months.
- [x] Consequence worth stating, because it looks like a bug: **a tombstone
      only becomes prunable once some later change exists.** A deletion *is* the
      head at the instant it happens. This is also how three of this work's own
      tests were caught passing **vacuously** — "spared by the date window" and
      "spared because its photo row still exists" both passed while actually
      being spared by the head guard, and stayed green with the arm they claimed
      to test deleted outright. Every such test now moves the head first.
- [x] Secure-hidden photos are **not** tombstones — their `photos` row still
      exists, so the predicate spares them. Pruning one would leave a photo the
      user just secured visible on every not-yet-synced client, which is the
      privacy-shaped half of this bug rather than the correctness-shaped half.
- [x] An unparseable floor fails **closed** (`i64::MAX` ⇒ every client
      full-walks) rather than open. Reading it as 0 would serve exactly the
      deltas the prune has already invalidated.
- [x] Tests: 12 new, 493 green (was 481). Verified RED against three separate
      breakages: removing the floor check from `fetch_delta` (both fallback
      tests fail with `deleted: Some([])` — the ghost-row bug verbatim, a client
      told "nothing was removed" about a photo whose tombstone was pruned);
      dropping the `seq < MAX(seq)` arm (the head falls); and swapping the
      SQLite cutoff for a chrono RFC 3339 one.
- [x] **The date-format claim was overstated on the first pass and is now
      accurate.** `changed_at` is `datetime('now')` (`"… 12:00:00"`) compared as
      a *string* against the cutoff. An RFC 3339 cutoff (`"…T12:00:00+00:00"`)
      does **not** wipe the log as first written here — the date portion
      dominates, so the two agree on every row except those dated the *same day*
      as the cutoff, where `' '` (0x20) sorts under `'T'` (0x54) and an
      inside-the-window row reads as expired. A boundary error of up to a day,
      not a catastrophe — but the boundary is the only observable part of a
      retention policy, so the cutoff is computed by the same function that
      wrote the column. `a_tombstone_just_inside_the_window_survives` is built
      on that same-day boundary precisely so it *can* fail.
- [ ] **Live verify — folded into the redeploy below.** Nothing on CT132 is 90
      days old (the DB was re-created 2026-07-22), so the prune is a guaranteed
      no-op there and the floor stays 0. That is the *correct* reading, not a
      passing test: confirm `photo_change_log` still holds one row per photo and
      that `server_settings` has no `photo_change_log_pruned_through` key at
      all. Forcing the other branch means backdating a tombstone by hand.

---

### B2 — Persist measured per-category conversion rates (#40 remainder) — DONE 2026-08-04

The ETA's seed rates only govern a machine that has never converted anything —
but today **every server boot is that machine**.

**The item understated the scope by one word: it is not just every *boot*, it is
every *pass*.** `eta_reset` runs at both ends of every batch (`raw_start` and
`clear_start_clock`), and `ConversionEta::reset` was `*self = Self::new()` — so
the second conversion pass of a single uptime was already back on the
compiled-in seeds. Persisting to the DB without fixing that would have shipped a
feature that only worked once per restart. The split the fix rests on: **the
queue is batch state, what the machine can do is not.** `reset` now clears
`cats` and keeps `calibration`, with the reason written next to it, because
"simplify this back to `*self = Self::new()`" is exactly the tidy-up that would
silently undo it.

- [x] Persist the last measured rate per category so the seeds retire on any box
      that has run one pass. One `server_settings` key per category rather than
      a JSON blob, so a corrupt value in one cannot take the other two down.
      Written once per pass (`save_throughput_calibration` in `ingest.rs`, not
      per file), read once at boot (`main.rs`, **awaited** rather than spawned —
      a seed that lands after the auto-scan has already started a pass is a seed
      that did nothing).
- [x] **What gets persisted is the batch average `Σweight / Σsecs`, NOT the
      EWMA the live estimate uses**, and the difference is not cosmetic. A video
      lane of width N starts N encodes together and they finish in a burst: each
      near-zero delta decays `ewma_secs` by a further 0.65 while `ewma_weight`
      does not move, so the EWMA's end-of-burst answer is a function of **the
      lane width, not the machine** — under-reading a narrow lane and
      over-reading a wide one. Measured on the 8-wide trace in the test:
      **20.4 MB/s against a true 8.0 MB/s, 2.5× optimistic**, which is the bad
      direction to bake in (an ETA that opens short and grows). The running
      totals read the truth from the moment the burst lands. This also retires
      #40's documented "first video sample underestimates by the lane width"
      wart for the persisted figure — Σ/Σ never had it.
      That is not a defect in the EWMA: mid-burst the *recent* rate genuinely is
      high, which is what in-batch tracking wants. It is a reason not to store it.
- [x] The stored rate is a **better seed**, not a competing estimator. Precedence
      is `this batch's own sample > stored calibration > compiled-in seed`, so
      once a category has evidence of its own nothing about in-batch behaviour
      changes — the same "a measurement replaces a seed, never blends with it"
      rule #40 already follows. `is_empty()` is still keyed on enqueued weight
      alone: a calibrated ledger with an empty queue is empty, or the
      client-declared upload batch loses its count-based fallback.
- [x] **A persisted rate is dangerous in a way a constant is not** — it crosses a
      process boundary and it feeds a division. `plausible_rate` gates it in
      **both** directions (a value that would be refused on load is never
      written), at 1 KiB/s .. 10 GiB/s. `NaN` is the one that actually bites:
      `rate <= 0.0` is **false** for `NaN`, so it sails straight through
      `eta_seconds`' existing guard. Failure is **soft** here — unlike
      `retention.rs`'s deliberately fail-closed floor — because ignoring a
      corrupt rate costs one worse progress bar, not silent data loss. The
      asymmetry is stated in both modules so neither reads as an oversight.
- [x] A category with no sample publishes **nothing**, rather than a zero or a
      seed. Most passes are images-only; reporting all three would erase the
      video rate a mixed pass measured last week on every one of them.
- [x] Tests: 15 new, 508 green (was 493). Verified RED four ways —
      `reset` restored to `*self = Self::new()` (3 fail), `rate` ignoring its
      calibration argument (4 fail), exporting the EWMA instead of the batch
      average (1 fail, printing `expected ~7.99 MB/s … got 20.36 MB/s`), and
      `plausible_rate` made a no-op (3 fail, `NaN` first).
      The DB half is tested through `read_calibration`/`write_calibration`
      rather than the public wrappers: those hold the ledger's std `Mutex`, and
      an async test awaiting while holding it would trip
      `clippy::await_holding_lock` and serialise nothing useful anyway.

> **Correction made mid-flight, recorded because the doc was wrong before it was
> right:** the first version of the EWMA-vs-average test used a 4-wide burst and
> the comment beside it claimed "~1.3 MB/s, a 3× pessimism". The RED check
> reported 3.64 MB/s against 4.00 — a real 9% failure, but the *sign and
> magnitude in the comment were both wrong*, and the margin was thin enough that
> a later tweak could have made the test vacuous. Widened to 8 and the claim
> rewritten from the measurement. **A test that fails for a smaller reason than
> its comment claims is a test that has not been read.**

> **Not a code change, but it cost real time and will again:** `cargo fmt` on
> this machine (rustfmt 1.9.0 / Rust 1.96) reformatted **20 files nobody
> touched**. CI pins `RUST_TOOLCHAIN: "1.88"` and gates on
> `cargo fmt --all -- --check`, so local rustfmt output is not the thing CI
> checks. Every hunk was 1.96 splitting a line 1.88 left joined. **Do not run
> `cargo fmt` on this repo from a 1.96 toolchain** — revert the churn by hand,
> or install 1.88.

---

### B4 — Ladder loose ends (#49 remainder)

- [x] **Cost control — DONE 2026-08-04. Decided: give it the video lane.**
      **The serial sweep was already budgeting itself for a parallel one**, and
      that is what settled it rather than any throughput guess. Each encode is
      capped at `video_threads` = `usable / video_lane` — a *share* sized for
      `video_lane` concurrent encodes — while exactly one ran. On a 128-thread
      host that is 8 threads of 112 usable, measured by the new test: lane 14,
      GPU lane 3. There is no serial fix, because handing the one encode all 112
      threads is precisely what `CPU_VIDEO_THREADS_TARGET` exists to refuse.
      `video_lane` is **1** below ~24 threads, so this is a provable no-op on
      ordinary hardware — pinned by `the_sweep_stays_serial_on_an_ordinary_host`.
- [x] **Two defects found on the way, neither named by this item.**
      1. `transcode_to_rung` planned its own threads from
         `plan_parallelism(num_cpus::get(), ..)`, which **never reads
         `SIMPLE_PHOTOS_CONVERSION_JOBS`** — so the one knob this item proposed
         handing the ladder did not reach the ladder at all. It now takes the
         budget as a parameter; the sweep plans once with `detect_parallelism`
         and both halves travel together. Sixth-instance material for the
         "two derivations of one list" risk below, in its budget form.
      2. **Nothing stopped the sweep running beside a conversion pass.** The
         ordering was arranged only by the three autoscan sites awaiting
         `run_conversion_pass` first; `upload.rs` and `scan.rs` kick that pass
         with no such sequencing, so an upload landing mid-sweep put two
         `video_lane`-wide video lanes on one box — on the GPU path, literally
         double the hardware session cap. Now a policy (`should_defer_sweep`),
         checked at sweep start **and** before each file, keyed on the pass
         **lock** rather than on `CONV_ACTIVE` (a client `batch_start` pins that
         flag with no pass running). `transcode_to_rung`'s comment claiming a
         ladder encode "runs alongside first-pass conversions" was the tell.
- [x] Tests: 7 new, 515 green (was 508). Verified RED three ways — the serial
      driver restored (`peak_concurrency` comes back 1, and *only* that test
      fails, which is the point: the tallies cannot tell a parallel driver from
      a serial one, so `peak_concurrency` is carried in `SweepOutcome` and
      logged); the conversion arm dropped from `should_defer_sweep`; and the
      lane pinned back to 1 (`left: 1, right: 14`).
- [ ] **Not fixed, and stated here so the next slow-drain investigation looks in
      the right place.** The sweep is also bounded by `SWEEP_CANDIDATE_LIMIT`
      (16 files) and by autoscan's ~hourly idle cadence. Against the live
      114-file backlog **those two dominate**, and on a box where
      `video_lane == 1` the lane width changes nothing whatsoever — so this
      item's premise ("a 114-file backlog still drains slowly") is only
      *partly* addressed. Widening either bound is a disk-I/O decision
      (memory `idle-disk-thrash-investigation`), not a lane-width one. Measure
      on CT132 first: the redeploy below is what produces the evidence.
- [x] **Securing a video removes its picker — DONE 2026-08-04. Decided: yes,
      carry the ladder.** What settled it was that **the exposure was already
      paid for and doing nothing**: `access.rs`'s third arm exists *specifically*
      to gate a secured video's rung blobs behind the unlock token, and nothing
      in the product ever handed a client one of those ids, so that arm was dead
      code. The rungs themselves survive securing untouched — the `photos` row is
      only *hidden* (`ELIGIBLE_PREDICATE`), never deleted, so `video_renditions`
      does not cascade and `orphan_sweep` still counts the blobs as referenced.
      The bytes were on disk being paid for in storage with the benefit thrown
      away. Both secure feeds (`{id}/items` and the aggregate `/items`) now
      hydrate `renditions`.
- [x] **The correlation is shared with the gate, not re-derived.**
      `SECURE_ITEM_RENDITION_MATCH` is one const used by both
      `is_secure_item`'s rendition arm and the new listing. Seventh instance of
      the two-derivations risk below, and the only one so far where the drift
      would be a **confidentiality** bug rather than a counting one: a listing
      matching more broadly than the gate publishes an ungated blob id, i.e. a
      full-quality copy of a hidden video fetchable with any session.
      `every_offered_rung_is_gated_by_the_serve_path` asserts the containment
      directly, with an `offered.len() == 2` guard — because breaking the shared
      const breaks *both* sides symmetrically and the test would otherwise pass
      vacuously. It did exactly that in the RED run: `expected both rungs, got []`.
- [x] **Two ids, not one.** The plan said "the secure gallery's own item
      listing", which reads as a one-line join. It is not: `add_gallery_item`'s
      server-side path stores a **clone** photo in `gi.blob_id` and the real
      photo in `gi.original_blob_id`, and the ladder only ever ran on the latter.
      A lookup keyed on `blob_id` alone finds nothing for every genuinely secured
      video — it passes only the in-place shape used in test fixtures.
      `a_cloned_secure_item_resolves_the_ladder_through_its_original` is that
      case, verified RED against the naive key.
- [x] **The asymmetry is stated, not hidden.** Rung *generation* is gated on
      `ELIGIBLE_PREDICATE`, so a video secured **before** its rung existed never
      gets one. The picker therefore appears for some secure videos and not
      others, keyed on the order of two operations the user does not think about.
      **Rejected widening generation to secured photos** as a side effect of a
      picker item: it would run ffmpeg over hidden content on a schedule and mint
      new derived blobs from it, which is a privacy decision in its own right and
      not one this item asked for. An empty ladder correctly draws no gear icon,
      which is honest rather than papered over. Recorded so it is not re-proposed.
- [x] Clients: web reads the ladder off the secure item (the IDB row `db.photos`
      is a **guaranteed** miss for secured photos — the same reason
      `photo_subtype` and `crop_metadata` already ride the item). Android's
      `SecureVideoPage` cannot re-point a `MediaBlobDataSource` like the main
      viewer does — the whole secure path is decrypt-to-a-temp-file — so a switch
      is a second download, with the playhead/pause state carried across it, the
      previous file deleted **only after** the new one exists, and every
      decrypted rung wiped on dispose. A 1080p plaintext copy left in the cache
      dir defeats the album as thoroughly as a 4K one.
- [x] `API_REFERENCE.md` — **the secure-item row was stale before this touched
      it**, listing seven fields of thirteen and omitting the aggregate
      `/api/galleries/secure/items` endpoint entirely. Same shape as C1(d)'s
      stale faces row. Fixed, plus the `is_source`/token contract.
- [x] Tests: 7 server (522 green, was 515) + 4 Android. Server verified RED two
      ways — the correlation narrowed to `blob_id` (2 fail) and the reader gutted
      to simulate the pre-fix tree (4 fail). Android verified RED by renaming the
      wire key to `rungs` (2 fail), which is the *realistic* regression: Gson
      leaves an unknown field at its default, so a renamed key silently becomes
      "no picker" — indistinguishable from a video with no rungs, and exactly the
      bug `PhotoDto.renditions` records having already happened once.
- [ ] **Device/browser check, folded into the verification session below.** No
      JVM or vitest run can show a gear icon on a secure video. Needs a secured
      >1080p video **that had a rung before it was secured** — securing first is
      the no-picker case and would look like a failure while being correct.
- [x] **E2E: rendition serving + range requests — DONE 2026-08-04.**
      [tests/test_91_video_rendition_serving.py](tests/test_91_video_rendition_serving.py),
      18 cases, driving the real pipeline: upload a 2560x1440 H.264 source, kick
      `POST /api/admin/photos/auto-scan` (which awaits the conversion pass and
      *then* calls `generate_rungs_after_scan`), wait for `renditions` to appear
      on the sync feed. The fixture asserts the ladder produced a **1920x1080**
      rung before any test runs — "no rungs" would otherwise make all 18 pass
      vacuously, which is this file's most-repeated lesson.
      "The rung, not the original" is decided by **ffprobing the served bytes**
      rather than by comparing lengths, so it survives any refactor of how the
      locator is chosen. The three properties the locator swap could get wrong
      are covered separately: the bytes, the length (`Content-Range` totals,
      open-ended ranges, a 416 at the rung's own length, and a
      reassemble-the-whole-file pass that catches chunk-frame boundary errors a
      single mid-file range cannot), and the cache identity (the original's ETag
      must **not** validate the rung — a 304 there hands the client the 4K bytes
      it asked to avoid).
- [x] **Found by that E2E, in a surface B4 never named: every video response was
      being gzipped, and it cost `Accept-Ranges`.** `DefaultPredicate` excludes
      `image/` and nothing else, so `video/mp4` went through the global
      `CompressionLayer` while the JPEG beside it did not. The wasted CPU on
      incompressible H.264 is the boring half. The functional half: **a
      compressed body is a transformed body**, so the layer drops
      `Content-Length` and `Accept-Ranges: bytes` and switches to
      `Transfer-Encoding: chunked` — deleting the exact header `serve_photo` sets
      to advertise seeking, on the serving path of a feature whose whole purpose
      is swapping quality mid-playback.
      **`main.rs` already documented the contract and `photos/serve.rs` had never
      once honoured it:** "Binary blob endpoints explicitly set
      `Content-Encoding: identity` to bypass this layer." `blobs/download.rs`
      does, in four places. `serve.rs` does, in zero. Fixed **centrally**
      (`http_utils::media_compression_predicate`, excluding `video/` and
      `audio/`) rather than by adding a fifth hand-written copy — see the
      eighth cross-cutting instance below. `application/octet-stream` is
      deliberately **not** excluded: it is a genuine catch-all, some of it
      compresses, and the blob route already opts itself out.
      Tests: 4 unit (526 green, was 522) + 4 E2E. Verified RED both ways — the
      E2E failed on the pre-fix tree with `assert None == 'bytes'` for
      `Accept-Ranges`, and reverting the predicate to the stock default fails
      `video_and_audio_are_never_compressed` and nothing else.
      `the_stock_default_would_have_compressed_video_mp4` asserts the
      precondition directly, so if a future tower-http ever declines video by
      itself, that test says so instead of leaving the predicate as dead weight.
      `json_and_text_are_still_compressed` is the vacuity guard: without it
      `compress_when(|_| false)` passes every other assertion while silently
      un-compressing the JSON API.

---

### B2 — E2E for the 3-strike conversion cap (#40 remainder) — DONE 2026-08-04

[tests/test_92_conversion_attempt_cap.py](tests/test_92_conversion_attempt_cap.py),
7 cases across 6 real autoscan passes.

**The item asked for a test that cannot be built, and the measurement says so
rather than an argument.** The ask was "a fixture that always fails must be
attempted **exactly 3 times across 5 real scan passes**". Measured on the E2E
server, garbage `.mkv` in the storage root, five `POST
/api/admin/photos/auto-scan` passes:

```
pass 1..5: media_convert_failure=1  conversion_retired=0  photos row=1
```

**One attempt, not three, and the cap never fires.** `process_candidate`'s
failure arm registers the ORIGINAL to avoid data loss, so the file lands in
`photos.file_path` and every later pass skips it via `existing_set` — which is
consulted *before* the skip cache. One strike is charged and nothing ever
spends the other two.

- [x] **The item's own warning was right and still not sufficient.** It said an
      ordinary failing conversion "passes vacuously via `existing_set`" — true,
      but it framed that as a test-authoring hazard when it is actually a
      statement about the *cap's reachability*. Enumerated against the code:
      both hash-dedup arms (success and failure side) record a terminal
      `hash_duplicate` **deliberately**, so they are not strikes either. That
      leaves the cap exactly two live consumers — the DB-error path at
      `ingest.rs`'s "Failed to register converted photo", and a pass interrupted
      between the charge and the registration. Neither is reachable over HTTP.
- [x] **Rejected adding a fault-injection seam** to force the third strike. It
      is the only way to reach `attempt_count = 3` end to end, and it means a
      release binary carrying a knob that makes it drop photos on demand. That
      is a worse thing to own than an untested branch whose arithmetic and SQL
      are already pinned by `photos/scan_skip.rs` and `photos/register.rs`.
      Recorded here so it is not re-proposed as "just an env var".
- [x] **What the E2E pins instead is the property the cap exists to deliver** —
      *no file is transcoded on every pass forever* — for both paths that
      actually reach it, over five real passes each.
- [x] **The no-row path is real, and it is the expensive one.** Same bytes at a
      second path (the Takeout shape: date folder + every album folder) fails
      conversion, hits the dedup arm, and returns **without registering
      anything** — verified `photos row = 0`. Pre-#40 that file was fully
      transcoded and the output discarded on every pass forever; it is now one
      transcode total. This is the first end-to-end coverage of that loop.
- [x] **Two vacuity traps, pointing opposite ways, and the file names both.**
      "Attempted only once" passes for the *wrong reason* on the registered path
      — it stops because a row exists, not because anything capped it — so
      `test_it_stops_because_it_registered_not_because_of_the_cap` asserts the
      row is present and that no `conversion_retired` was announced for a file
      that is sitting in the library. On the no-row path the count means nothing
      unless the row is genuinely absent, so
      `test_the_duplicate_leaves_no_photos_row` runs first as the precondition.
- [x] `retry-failed` scoping is now pinned end to end. It deletes
      `conversion_failed` rows only; widening it re-admits the whole duplicate
      set and re-hashes the library on the next pass — the disk thrash migration
      031 removed. The `WHERE` clause is one line and the consequence lands a
      full scan pass later, which is why no unit test has ever seen it.
- [x] Verified RED two ways, each biting exactly the tests it should. Gutting
      the conversion walk's skip-cache consultation (the pre-#40 tree) takes the
      no-row duplicate to **5 transcodes instead of 1** and fails 2 of 7 — and
      notably **not** `test_the_duplicate_leaves_no_photos_row`, which is
      correct, since the row is still absent. Widening `retry-failed` past its
      reason scope fails **1 of 7** (`assert 2 == 1`) and nothing else.
      526 server unit tests still green; `cargo +1.88 fmt --check` clean.

---

### B6 — `no-store` on every `/api/` response makes all media caching dead code — DONE 2026-08-04

Found 2026-08-04 while writing B4's serving E2E, and **deliberately not fixed
there**: it is a privacy decision, not a bug fix, and it does not belong inside a
#49 commit.

[security.rs](server/src/security.rs#L99-L105) stamps
`Cache-Control: no-store, no-cache, must-revalidate` on **every** response whose
path starts with `/api/`. That is a blanket `insert`, so it overwrites whatever
the handler set. Measured on the wire, not inferred:

| route | what the handler sets | what the client receives |
|---|---|---|
| `/api/photos/{id}/file` | `private, max-age=86400` | `no-store, no-cache, must-revalidate` |
| `/api/photos/{id}/thumb` | `private, max-age=86400` | same |
| `/api/blobs/{id}` | `private, max-age=31536000, immutable` | same |

The three rows above are **measured on the wire**, not inferred from the code.
The same blanket `insert` reaches every other handler that sets the header:
`photos/serve.rs` sets it at **10** sites, `blobs/download.rs` at **5**,
`trash/handlers.rs` and `backup/proxy.rs` at one each — 17 in total, all of them
dead. (`setup/import.rs:220` is the 18th and the only one that survives, because
it sets `no-store` itself and therefore agrees with the middleware.) Dead with
them is the ETag machinery behind them: `no-store` forbids storing the response
at all, so there is nothing left to revalidate with. The #49 picker's whole premise
— swap quality, keep playing — assumes a client can hold onto bytes it already
fetched. Today every thumbnail in a scrolled grid is re-fetched and re-decrypted
on every visit.

The header is not obviously wrong, which is why this is an item and not a patch.
`no-store` on `/api/` was chosen so responses that may carry user data or tokens
are never persisted, and media *is* user data: relaxing it means a browser
writes **decrypted** photos and videos into its on-disk cache, which is
precisely what the secure-gallery threat model exists to prevent. The plausible
shape is a split — token/JSON endpoints keep `no-store`, media routes keep the
handler's `private` value, and secure-gallery media stays `no-store`
unconditionally — but "which routes are media" is a list, and this file already
tracks what happens to those.

**DONE 2026-08-04. Decided: split by route, and the split is an allowlist.**

- [x] Policy: media routes keep the handler's value; everything else under
      `/api/` keeps `no-store`; **secure-gallery media is `no-store`
      unconditionally**. The handler headers are now authoritative rather than
      deleted.
- [x] **The obvious implementation of "split by route" does not work, and the
      reason is the whole shape of this item.** Secure media is served by the
      *same* routes as ordinary media — `/api/photos/{id}/file`,
      `/api/blobs/{id}` — and is distinguished only by an unlock token. A path
      list therefore cannot classify it. Nor can "did the request carry a
      gallery token": [core.ts:44](web/src/api/core.ts#L44) attaches
      `X-Gallery-Token` to **every** request once a vault is unlocked, so that
      rule would silently disable media caching library-wide for exactly the
      users who have secure albums — fail-safe, but it would have quietly
      undone the fix and left the tests green.
      The verdict has to be **per item**, so `require_secure_access` now returns
      `Confidentiality` (`Cacheable` / `Secure`) instead of `()`, and the six
      call sites spend the `is_secure_item` query they were already paying for
      twice: once to enforce the token, once to pick the header.
- [x] The middleware grants caching only when **both** hold: the route is on
      `http_utils::is_cacheable_media_route` **and** the handler actually set a
      value. Both fail towards `no-store`, so a new route, an error response, or
      an early return all land back on the default. An allowlist rather than
      "don't stomp what the handler set" precisely because the latter fails
      **open** — a handler that forgets gets no protection and nothing says so.
- [x] `/api/trash/{id}/thumb` and the backup-proxy thumb are **deliberately
      excluded** and still stomped to `no-store`. Both set a `private, max-age`
      header today, and neither calls `require_secure_access` — a route that
      cannot classify its own content must not be granted a cache. Pinned by
      `routes_that_cannot_classify_their_content_are_excluded` so the exclusion
      reads as a decision rather than an oversight. Adding them means gating
      them first.
- [x] **Found while classifying the routes, and it is a confidentiality bug, not
      a caching one: `/api/photos/{id}/source-file` had no secure-album gate at
      all.** Every sibling projection — `file`, `web`, `thumb`, `motion-video` —
      took a `GalleryToken` and called `require_secure_access`.
      `serve_source_file` took neither, so an account session alone downloaded
      the **original, unconverted** source (the HEIC, the `.mkv`) of a photo
      sitting in a secure album. Securing hides the `photos` row via
      `ELIGIBLE_PREDICATE` but never deletes it and never clears `source_path`,
      so the row this handler reads survives securing untouched. The route had
      **zero** E2E coverage before this. Gated, and the E2E verified RED against
      the pre-fix tree: `expected 401, got 200`.
      Worth stating because it compounds: on that same RED run the secured
      original also came back `private, max-age=86400`, i.e. B6's own fix would
      have told the browser to keep a plaintext copy of it on disk for a day.
      **Fixing the caching without fixing the gate would have made the leak
      durable.**
- [x] Two more sites the item never counted. `serve_photo`'s v1-monolithic
      **206** exit set no `Cache-Control` at all — invisible while everything was
      stomped, but a cacheable 200 beside an uncacheable 206 means seeking a
      small encrypted video re-decrypts it every time. And `check_etag`'s **304**
      hardcoded `max-age=86400`, which for a secure item would have *extended*
      the life of a cache entry the 200 refused to create — the one response
      where the mistake has no body to notice it.
- [x] Tests: 13 unit (539 green, was 526) + 15 E2E
      ([tests/test_93_media_cache_headers.py](tests/test_93_media_cache_headers.py)).
      The unit tests drive the **real middleware** through `tower::oneshot`,
      because every one of the 17 handler sites was already "correct" when read
      in its handler — the defect only ever existed after the middleware ran.
      Verified RED four ways, each biting exactly the tests it should:
      the middleware restored to its unconditional `insert` (2 unit + 5 E2E,
      each printing `no-store…` where the handler set `private, max-age=86400`);
      `media_cache_control` ignoring its `Confidentiality` argument (2 unit,
      printing `left: "private, max-age=86400"` for a secure item); and the
      `serve_source_file` gate removed (2 E2E). The JSON and secure-media tests
      stayed green in the middleware RED run, which is the point — they are not
      coupled to the thing being fixed.
      `TestJsonIsStillNeverStored` is the vacuity guard with teeth: deleting the
      middleware's insert outright satisfies every "media is cacheable"
      assertion while writing refresh tokens to a browser's disk cache.
- [x] `test_the_etag_handshake_is_alive_again` is the one that proves the
      *payoff* rather than the header: fetch, keep the ETag, re-fetch with
      `If-None-Match`, require a 304 whose own `Cache-Control` still permits
      storage. `no-store` made the ETag machinery unreachable by construction;
      this is the first test that shows a conditional hit actually completing.
- [x] `security.rs`'s comment recorded this lesson already being learned once,
      for static assets ("previously we stomped them with no-store, forcing
      browsers to re-download the entire frontend on every page load"). The same
      mistake was live for media one `if` below it. The comment now describes
      the media case too, in the same block as the code that implements it.

> **Fixture lesson, recorded because it cost two runs and will recur:** ingest
> deduplicates on content hash, so two uploads of identical bytes collapse into
> **one** `photos` row. The first draft of `test_93`'s fixture called
> `generate_test_jpeg(64, 64)` and `generate_test_tiff()` twice each; the
> "ordinary" and "secured" photos came back as the same id, so securing one
> secured the other and the caching tests failed with 401. **Every fixture photo
> in a test that treats photos differently must be byte-distinct**, and the
> fixture now asserts that directly rather than hoping.
> Second trap in the same file: `/source-file` serves `photos.source_path`, and
> **the upload endpoint never writes that column** — an ordinary upload of a
> convertible file takes `upload.rs`'s *inline* branch, which converts and
> registers but records no source. Only `run_conversion_pass` sets it, and the
> only way to reach that over HTTP is a deferred upload, which is gated on
> `X-Defer-Conversion` **and admin**. A regular-user upload 404s there, and a
> 404 would have made the secure-gate test pass while proving nothing.

---

### Z1 — Album headers diverge; adding to a second secure album MOVED instead of adding

> Original report: *"when selecting items in an album there is a remove option,
> but no + button to add those items to another album. And in secure albums when
> trying to do so, it removes the item from the current album it's in rather than
> showing in both. This also means we have diverging options for album headers, so
> the remove button can be changed to a trashcan icon, and a popup box letting the
> user know what's happening with a yes and no option shows."*

Server (`8bf2f06`, `14606b9`), the regular-album web view (`56f995c`), the web
secure removal (`50464c2`) and Android (Z1e below) are all done. **Every code
half of the original report is now closed; what is left is a device check and
the server suspicion Z1f raises.**

**⚠ `56f995c` shipped Z1 HALF-WIRED, and the green suite is what hid it.** That
commit wrote `web/src/gallery/albumRemoval.ts` — `secureRemovalPrompt`, fully
unit-tested, documenting at length why "returns to your regular gallery" is a
privacy-shaped lie after Z1 — and **wired it into nothing**. Meanwhile
`SecureGallery.tsx` still shipped a raw `confirm()` containing *that exact
sentence*, plus the same claim in the success toast and the tile tooltip. Its
twin `albumRemovalPrompt` *was* wired, which is what made the omission read as
done. **Check the call site, not the export.**

#### Z1d — web secure removal — DONE 2026-08-04

- [x] Three-way verdict, not two. `otherSecureAlbumCount` resolves the feed's
      `galleries` array to `0` / `>0` / **`undefined`**, and
      `secureRemovalPrompt` returns a discriminated `confirm` | `blocked`.
      **Empty means UNKNOWN, not zero** — the server's own comment says a miss is
      unreachable by construction, so an empty array can only be an older server.
      Reading it as 0 is precisely how the UI came to promise a photo would
      return to the regular gallery when it would stay secured.
- [x] **The `otherSecureAlbums = 0` default was removed, deliberately.** The
      parameter is now required and has no default: the old signature handed the
      most dangerous of the three answers to every call site that had not thought
      about membership, *by omission*. An argument you must pass is the only
      version of this function that cannot be misused by accident.
- [x] **Decided: block rather than hedge** when membership is unknown. A prompt
      that hedges makes the user adjudicate a fact only the server holds; a
      prompt that guesses is the original bug restated. The refusal offers a
      **Refresh** that re-fetches the feeds, because a fail-closed guard with no
      recovery path is one the next person deletes.
- [x] A list that does not contain the owning album is **unknown**, not
      off-by-one. Counting the owner as an "other" flips a last-membership
      removal into the "stays secured" branch — wrong in the direction that
      surprises the user.
- [x] All three false claims killed: the `confirm()`, the success toast
      (now conditional), and the tile tooltip (now names the action and claims no
      outcome — a tooltip cannot be conditional on a per-item lookup).
- [x] In a smart view the dialog names the **owning** album, not the open one.
      `selectedGallery.name` there is "Videos", and naming that as the thing being
      removed from is wrong in the one dialog whose entire job is accuracy.
- [x] `SecureGalleryItem.galleries` added to the web DTO — **the type never had
      it**, so no client could have computed this even if it had wanted to.
- [x] The album-delete `confirm()` was ported to `ConfirmDialog` too. Leaving a
      native confirm beside a real dialog is the same divergence this item is
      about. It is the one prompt here that earns `tone="danger"`: unlike removing
      an item, deleting an album can drop the last reference to a clone and take
      its bytes with it.
- [x] Tests: 10 new, **322 green** (was 312). Verified RED two ways, and the
      first is the one that matters: reverting **only the component** fails the 4
      wiring tests while **all 14 helper tests stay green** — the exact signature
      of how `56f995c` shipped. Reverting only the helper fails 8.
- [x] **A wiring guard now exists**, reading source with `node:fs` after
      `safeArea.test.ts`'s precedent. This repo has no jsdom, so the dialog cannot
      be rendered and asserted; what *is* checkable is that the call site exists
      and the false sentences are gone. **A tested helper with no call site is
      worse than no helper** — the green suite is what stops anyone looking.
- [ ] **Browser check** — folded into the verification session below. No vitest
      run can show a dialog. Needs a photo in **two** secure albums (the `>0`
      branch) and one in a single album (the `0` branch); confirm the two bodies
      differ and that neither promises the wrong outcome.

#### Z1e — Android — DONE 2026-08-04

- [x] **`SecureGalleryViewModel.pushItemsTo` called `moveItem`** — the reported
      bug itself, and the last place it was still live. Now `addItem` against the
      clone blob, with a 409 counted as **"already in that album", not a
      failure**: that is the server answering the membership question
      authoritatively, and the alternative — a client-side "is it already there"
      filter — would have to guess it from a per-album feed that cannot see the
      target. `planMovesToTarget` is **deleted** rather than left beside its
      replacement; it had exactly one caller and this was it.
- [x] `isConflict` reads the **status**, not the message text. The message is
      authored in Rust and would have been compared in Kotlin — one fact derived
      in two languages with nothing keeping them in step. Mirrors web's `core.ts`.
- [x] **The false sentence is dead in all three places it lived**, and one of
      them was a surface Z1d never named: `GalleryDetailView`'s multi-select
      dialog, **`SecurePhotoViewer`'s own separate confirmation** (a second copy
      of the same conditional outcome, which is a second thing to get wrong), and
      the "Move" verb in the header. The viewer's dialog is not restated
      accurately — it is **deleted**, and the overflow item now raises the one
      shared dialog. An `AlertDialog` is its own window, so it renders over the
      pager and a cancel leaves the page exactly where it was.
- [x] `AlbumRemoval.kt` mirrors `albumRemoval.ts`, with one deliberate divergence:
      **the parameter is a list of per-item counts, not a scalar.** This screen
      removes a whole multi-select at once, so the answer is genuinely per item;
      a scalar rule plus a batch rule would be two derivations of one question.
      One unknown in a batch blocks the whole batch, and **an empty list is
      unknown too** — a caller that resolved nothing has not answered the
      question, and the safe reading of "no information" is never "no other
      album". `SecureRemovalVerdict` is a sealed hierarchy rather than web's
      `kind` string: a call site that renders only the confirm arm **fails to
      compile** instead of silently dropping the refusal.
- [x] `SecureGalleryItem.galleries` added to the Android DTO — it never had it,
      so no client could have computed this. Null and empty both mean UNKNOWN;
      the realistic Android failure is not an old server but **Gson leaving a
      renamed wire key at its default**, which is the same regression shape
      `PhotoDto.renditions` already suffered once (B4). Fail-closed either way.
- [x] The dialog counts the **same batch the removal acts on**.
      `SecureMovePlan.expandForRemoval` was extracted from `removeItems` and is
      now called by both — asking one derivation what will be removed and another
      what to say about it is how a prompt ends up accurate about the wrong set.
- [x] Regular `AlbumDetailScreen.kt`: **Close icon → trash icon**, a confirmation
      that did not exist at all, and the "+ Add to album" the report asked for.
      `AlbumPickerDialog` was `private` in `GalleryScreen.kt`; it is **moved to
      `ui/components` and shared** rather than copied — eleventh instance of the
      two-derivations risk, in its mildest form.
- [x] Tests: 33 new, **285 green** (was 252). Verified RED three ways, each
      biting exactly the tests it should:
      - **Reverting only the component** fails the 4 wiring tests while **all 18
        `AlbumRemoval` tests stay green** — the exact signature of how `56f995c`
        shipped Z1 half-wired, reproduced deliberately.
      - `pushItemsTo` restored to `moveItem` fails **1** test, printing
        `pushItemsTo must call addItem — a '+' that moves is the reported bug`.
      - `otherSecureAlbumCount` reading empty as 0 fails **1**
        (`expected:<null> but was:<0>`) — the single misreading this field exists
        to prevent.
- [x] **A wiring guard now exists on Android too**, reading source with
      `java.io.File` after web's `safeArea.test.ts` precedent. Compose UI needs a
      device, which is *why* this class of bug outlives its web twin here. The
      path lookup walks up from `user.dir` and **throws** if it cannot find the
      file, because a wrong guess would make every assertion in it pass
      vacuously — which is the exact failure the file is about.
- [ ] **Device check** — folded into the verification session below. Needs a
      photo in **two** secure albums (the `>0` branch), one in a single album
      (the `0` branch), and a server that publishes no `galleries` (the blocked
      branch, reachable by pointing at an older build). Confirm the three bodies
      differ, that the push genuinely leaves the photo in *both* albums, and that
      the regular album's trash icon asks before un-filing.

#### Z1f — the secure feed publishes an id the add path may not correlate — OPEN, UNVERIFIED

Found while wiring Z1e, in the server, and **stated as a suspicion because it has
not been proven yet** — the next session's first job is a unit test against
`existing_memberships`, not a fix.

`list_gallery_items` publishes `blob_id` as
`COALESCE(gi.encrypted_blob_id, p.encrypted_blob_id, gi.blob_id)` — the *clone's
encrypted* blob for a server-side photo whose clone has since been encrypted, not
the raw `gi.blob_id`. Both clients' push-add sends that value back to
`add_gallery_item`. There, `candidate_original_id` resolves it to the clone's own
`photos` row (`= gi.blob_id`), and `SECURE_MEMBERSHIP_MATCH` is
`(gi.original_blob_id = ?1 OR gi.blob_id = ?2)` — canonical is never matched
against `gi.blob_id`. If that reading is right, the add finds **no** membership
and therefore neither adopts the clone (a second full clone: double storage, a
second decrypt+encrypt of a possibly multi-gigabyte video, and two physically
different files so an edit in one album cannot reach the other) **nor 409s on a
same-album duplicate**, which would break the one half of the invariant Z1 kept.

Why it is plausible that this is live and unnoticed:
- `test_94` adds with the **original** client-uploaded blob id, where
  `gi.encrypted_blob_id == gi.blob_id`, so the published id *is* the clone id and
  adoption works. The tested path and the UI path are different id spaces —
  todo.md's own "verify the *id space*, not just the call shape" risk, again.
- CT132 is ~13k **autoscanned** photos, i.e. entirely the untested shape.

The suspected fix is one clause (`gi.blob_id = ?1` as well), since a clone id is a
fresh UUID and cannot collide with an unrelated photo id — but **do not apply it
before a RED test reproduces the miss.**

Also found, and smaller: **web's `planSecureMovesToTarget` is dead** — exported
and tested, called by nothing since `56f995c` switched the push to adds. Same
shape as Z1's half-wiring one level down. Android's twin was deleted in Z1e; web's
was left alone to keep that commit Android-only.

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
      hardware has never been measured. **Now also the readout for the
      calibration above**, and it is cheap: after one pass,
      `SELECT * FROM server_settings WHERE key LIKE 'conversion_rate_%'` shows
      what CT132 actually measured, so the seeds can be re-based on evidence
      instead of the order-of-magnitude guesses in `progress.rs`. The *second*
      pass is the one that proves it — its ETA must open near the first pass's
      measured rate rather than at the 2 MB/s video seed.
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
- [ ] **Z1e** — the whole point of the item, and no JVM test reaches it. A photo
      in **two** secure albums, one in a single album, and (by pointing at an
      older build) one whose feed publishes no `galleries`: the three dialog
      bodies must differ and none may promise the wrong outcome. Then the push
      itself — file a secure photo into a second album and confirm it is still in
      the first, which is the reported bug and the only check that settles it.
      Also the regular album header: trash icon, confirmation, "+ Add to album".
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
  **Seventh instance: B4's secure ladder** — "which renditions belong to this
  secure item" is asked by the serve-path gate *and* by the listing that offers
  them. Shared as `SECURE_ITEM_RENDITION_MATCH` in the same commit. This is the
  first one where drift would be a **confidentiality** bug, and it is also the
  first where a naive equality test passes vacuously: breaking the shared
  expression breaks both sides at once, so the test needs a non-emptiness guard
  on top of the containment assertion.
  **Eighth instance, and the first where the "one function" was never written at
  all: the compression bypass** (B4's E2E finding). `main.rs` states the rule in
  a comment — "binary blob endpoints explicitly set `Content-Encoding: identity`
  to bypass this layer" — and then leaves every endpoint to remember it by hand.
  `blobs/download.rs` remembered four times; `photos/serve.rs` remembered zero.
  This is the failure mode one step earlier than usual: not two derivations that
  drifted, but **a rule that only ever existed in prose**, so a whole route could
  omit it without contradicting anything. A comment describing a convention is
  not an implementation of it. Fixed as a predicate the router applies once, so
  the next media route cannot forget. Watch for the same shape in
  `Cache-Control` — B6 above is that rule stated 17 times and enforced zero.
  **Ninth instance, and it is the `Cache-Control` case that sentence predicted**
  (B6, now fixed). Two things were being derived twice. The header value itself
  was 17 hand-written copies of two strings, now `media_cache_control` plus two
  consts. More importantly, **"is this item secure" was about to become a second
  derivation**: the handler already knew, via `require_secure_access`, and the
  tempting shape was to re-ask at header-building time. It returns the verdict
  instead. This is the second instance after B4's `SECURE_ITEM_RENDITION_MATCH`
  where drift would be a **confidentiality** bug rather than a counting one —
  and unlike B4's, the two sides here would not have broken symmetrically: the
  gate would have kept working while the header quietly authorised a browser to
  store a decrypted secure photo for a year. **When one query answers both "may
  I serve this" and "may they keep it", return the answer; do not ask twice.**
  **Tenth and eleventh instances, both in Z1e and both mild — which is the
  point.** `SecureMovePlan.expandForRemoval` (the removal's burst expansion, now
  asked by the dialog *and* the removal instead of inlined in one and re-derived
  in the other) and `AlbumPickerDialog` (`private` in `GalleryScreen.kt`, needed
  verbatim by the album detail screen). Neither would have caused a
  confidentiality bug; both would have drifted. The cheap time to share is the
  commit that needs the second copy — every entry above is what happens when it
  is not.
- **A tested helper with no call site is worse than no helper.** Z1 shipped
  `secureRemovalPrompt` fully unit-tested and wired into nothing, while the
  component kept the sentence the helper existed to kill; the green suite is what
  stopped anyone looking. Both clients now carry a source-reading wiring guard
  (web `node:fs`, Android `java.io.File`) for the prompts specifically, because
  neither repo can render a dialog in a unit test. The general form: **when a
  helper's whole value is that a component calls it, assert the call site.**
- **A route that cannot classify its own content must not be granted a
  capability.** B6's allowlist excludes `/api/trash/{id}/thumb` and the
  backup-proxy thumb for exactly this reason — they set cache headers but never
  call `require_secure_access`, so they cannot tell a secured photo from an
  ordinary one. The general form: before extending a permission (caching,
  compression bypass, range serving) to a route, check the route can *answer the
  question the permission depends on*. `serve_source_file` is what happens when
  it cannot and is granted anyway — it had no gate at all, and B6 nearly handed
  it a one-day cache.
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

