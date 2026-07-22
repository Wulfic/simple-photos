# TODO â€” Open GitHub Issues (#38â€“#52)

Pulled from `Wulfic/simple-photos`, 14 open issues, all filed 2026-07-20.
Investigated against the tree at `b3e48f0` (branch `dev`).

**Legend for confidence:**
- **CONFIRMED** â€” I read the code, the defect is visible in the source. No live repro needed to start.
- **HYPOTHESIS** â€” mechanism identified, but the specific number/symptom the user reported needs a live box to attribute.

**Ground rules for this batch (non-negotiable):**
- One commit per issue, conventional commit, referencing `(#NN)`.
- Every fix ships with a test that FAILS before the fix. No exceptions, especially the count/pagination work â€” those bugs survived precisely because nothing asserted them.
- Never commit red. `cargo test --bin simple-photos-server`, `npm test` in `web/`, and the `tests/` pytest E2E suite must be green.
- Known-red baseline before you start (see memory `e2e-preexisting-failures-2026-07-15`): `test_06` secure 401s (8), `test_20` dates (4), `test_58` Windows harness bug, and `test_18` audio 403s (6 — `audio_backup_enabled` defaults false; found during B1). Do not blame your diff for those; do not let them grow.

---

## Workstream A â€” Counts, performance, and the scroll crash

These three issues (#42, #38, #51) are **one problem wearing three hats**: there is no single authoritative definition of "how many items are in this library," and the web client rebuilds the entire library on every sync while rendering every tile it has ever seen. Fix them together, in this order. Do not start anywhere else in this file until A is done.

### A1 â€” #42 Incorrect photo counts (High) â€” CONFIRMED

**Reported:** Android Photos album shows 10,211; web shows 7,822. "And many other instances."

**Root cause: there are three different, silently diverging definitions of the count.**

1. **Server** â€” [server/src/gallery/summary.rs:109-127](server/src/gallery/summary.rs#L109-L127) counts every eligible `photos` row, secure-excluded, **including rows with `encrypted_blob_id IS NULL`**, and reports both raw `total` and `collapsed_total`.
2. **Web** â€” [web/src/gallery/hooks/usePhotoSync.ts:205](web/src/gallery/hooks/usePhotoSync.ts#L205) does `if (!photo.encrypted_blob_id) continue;`, so every not-yet-encrypted row is **dropped from IndexedDB entirely**. Counts then come from that truncated mirror via [web/src/gallery/smartAlbums.ts:33-36](web/src/gallery/smartAlbums.ts#L33-L36).
3. **Android** â€” [PhotoRepository.kt:961](android/app/src/main/kotlin/com/simplephotos/data/repository/PhotoRepository.kt#L961) applies the *same* `?: continue` skip, **but** the count is taken from [AlbumViewModel.kt:250-269](android/app/src/main/kotlin/com/simplephotos/ui/screens/album/AlbumViewModel.kt#L250-L269) via `photoRepository.getAllPhotos()`, which is [`SELECT * FROM photos`](android/app/src/main/kotlin/com/simplephotos/data/local/dao/PhotoDao.kt#L16) â€” **the whole Room table, including device-captured local rows (`localPath` set, `syncStatus` PENDING/FAILED) that were never on the server and that the web mirror cannot possibly contain.**

So Android counts server-synced âˆª device-local; web counts server-synced-and-encrypted only. ~~**HYPOTHESIS:** the 2,389 delta is predominantly the device camera roll pending upload.~~

> **âœ… MEASURED 2026-07-20 on live CT132 â€” the camera-roll hypothesis was WRONG.**
> The delta is the *server-side encryption backlog*, and it accounts for the gap
> exactly, with zero remainder:
> ```
> summary.total              14874
>   âˆ’ NULL encrypted_blob_id  2494   â† web's `continue` drops these
>   âˆ’ lost to the cursor bug     29   â† 29 page boundaries, 1 row each
>   = web-visible             12351   â† exactly what a full walk returns
> ```
> `encrypted_thumb_blob_id` is NULL on the same 2,494 rows, so they have no
> displayable ciphertext at all. Do not re-raise the camera-roll theory without
> device evidence. Repro: scratchpad `probe.ps1` (auth â†’ summary â†’ full walk).

**Second, independent defect â€” off-by-one in keyset pagination. CONFIRMED.**
[server/src/gallery/sync.rs:99-131](server/src/gallery/sync.rs#L99-L131) fetches `LIMIT limit + 1`, then builds `next_cursor` from `photos.last()` â€” which is the **peeked (limit+1)-th row** â€” and only afterwards truncates the response with `.take(limit)`. The next page's predicate is strict (`< ts OR (= ts AND id > id)`), so the peeked row is **never returned by any page**. One photo is silently lost per page boundary, on both clients, forever. At 500/page over a 10k library that is ~20 photos vanishing from every client. Nothing in `tests/` asserts round-trip completeness â€” [tests/helpers.py:1379-1390](tests/helpers.py#L1379-L1390) paginates the same way and only checks the data it *did* receive.

**Fix:**
- [x] `server/src/gallery/sync.rs` â€” build `next_cursor` from the **last returned** row, not the peeked one. Truncate first, then derive the cursor. â€” `568c282`
- [x] Rust unit test: paginate fully, assert the returned id set **equals** the seeded set. Verified RED first (`rows were never returned by ANY page: ["p03"]`). â€” `568c282`
- [x] **Same defect found in 3 more paginators** (`blobs`, `trash`, `photos`) â€” all fixed in `568c282`. `sync.rs` was the mildest: it has an `id` tiebreak, the others use timestamp-only cursors.
- [x] Decide and document the ONE canonical count definition. **DECIDED (Tyler, 2026-07-20): server-authoritative, count everything including unencrypted, grids unchanged.** Consequence accepted: the badge intentionally exceeds the tile count by the pending-encryption backlog. Corollary â€” no client may count its own local mirror. â€” `29e4d1f`
- [x] Extend `PhotoSummary` with per-smart-album collapsed counts â€” shipped as `smart_photos`/`smart_gifs`/`smart_videos`/`smart_audio`/`smart_favorites`/`smart_recent`. Note `smart_photos` counts photo **+ gif** because both clients define "Photos" that way; the raw `photos` column does not. â€” `29e4d1f`
- [x] Web: counts extracted to `gallery/smartAlbumCounts.ts` and precedence inverted from `local ?? summary` to `summary ?? local`. The real bug was not that web *dropped* rows â€” it was that the truncated mirror **outranked** the authoritative summary. â€” `29e4d1f`
- [x] Android: same inversion in `AlbumViewModel.loadSmartAlbumCounts`; every fallback category now collapses bursts (`total`/`gifs`/`videos`/`audio` were raw while `favorites`/`photos`/`recent` were collapsed, in one function). â€” `29e4d1f`
- [x] E2E (`tests/`): upload N photos, assert `/photos/summary`, a full `encrypted-sync` pagination, and the album badge all agree. â€” `21eae82` (`tests/test_89_count_agreement.py`, 11 tests). Verified it bites: with the pre-`568c282` cursor temporarily restored, 7/11 fail and `limit=1` returns **6 of 12 rows**. Small page limits are the whole trick â€” at limit=500 a 12-row library has no page boundary and the bug is invisible.
- [x] ~~**Android has no unit test source set** (`app/src/test/` does not exist).~~ **WRONG â€” corrected 2026-07-21.** `android/app/src/test/` exists and holds 19 JVM test classes (`JustifiedGridLayoutTest`, `NewWindowTest`, `AlbumCountTest`, `SelectionStateTest`, `BurstCollapseTest`, â€¦), plus 4 Compose tests in `androidTest`. `.\gradlew.bat :app:testDebugUnitTest` runs 154 tests. **There is no excuse for shipping untested Android logic in this batch** â€” the real constraint is narrower: anything needing a rendered composable or a live Room DB is `androidTest` and needs a device, so pure logic must be extracted to be tested (as `#49` did with `RenditionChoice.kt`). Note `android.util.Log` throws in JVM tests unless `testOptions.unitTests.isReturnDefaultValues` is set â€” added 2026-07-21, and it was silently blocking every error-path test.
- [ ] **Deploy required to realise the fix.** The live box still runs the pre-fix binary; the 29 lost photos and the badge numbers do not change until CT132 is redeployed.
- [ ] Follow-up defect (own issue): `blobs`/`trash`/`photos` use timestamp-only cursors with no tiebreak, so rows sharing a timestamp at a page boundary are still dropped. Batch-encrypted blobs share `upload_time` in bulk, so this is not theoretical.

**Live verification query (run before and after):**
```sql
SELECT COUNT(*) FILTER (WHERE encrypted_blob_id IS NULL) AS unencrypted,
       COUNT(*)                                          AS eligible
FROM photos WHERE user_id = ?;
```
Compare against Android `SELECT COUNT(*) FROM photos WHERE serverPhotoId IS NULL` (device-local backlog).

---

### A2 â€” #38 Photo libraries are slow (High) â€” CONFIRMED

**Reported:** slow on both web and Android; asks for a unified server-side cache.

**Root cause â€” the web sync is O(library) per pass with a sequential IndexedDB round-trip per photo.** [web/src/gallery/hooks/usePhotoSync.ts:155-405](web/src/gallery/hooks/usePhotoSync.ts#L155-L405):
- Phase 1 pages the **entire** library into memory (`limit: 500`, loop to exhaustion) â€” every pass, no delta/`since` parameter, no ETag.
- Phase 2 then enumerates **all four blob types** in full (`fetchAllPages` Ã—4).
- Phase 3 iterates every photo in a `for` loop with `await` inside: `await ensureThumbCached(...)` performs an IndexedDB read **per photo** ([line 51](web/src/gallery/hooks/usePhotoSync.ts#L51)), plus `await db.photos.update(...)` per changed row. For 10k photos that is 10k+ serialized IDB transactions on the main thread.
- Phase 4 adds another per-blob `await db.photos.get()`.

The 5-minute interval and re-entrancy guard (added in the idle-thrash fix) stop it *stacking*, but each individual pass is still a full-library rebuild. The server has no delta endpoint to make it cheap.

**The issue's own suggestion is the right fix.** Build a real server-side cache + delta protocol.

> **Client batching landed first â€” `f31b27b`.** Sequenced ahead of the protocol
> deliberately: one file, no schema, no Android, no wire change, and it removes
> the dominant client-side cost. Phase 3 is now staged per 200-row chunk (plan â†’
> one indexed key-scan + only-missing downloads â†’ one `rw` transaction with a
> `bulkPut` per table) in `web/src/gallery/hooks/syncReconcile.ts`. Two further
> O(library) reads went with it: the second full `toArray()` after stale pruning,
> and the per-blob `db.photos.get()` in Phase 4.
>
> **Hard constraint discovered â€” do not undo it:** a blob download can NOT happen
> inside a Dexie `rw` transaction (awaiting a non-Dexie promise commits it early,
> next write throws `TransactionInactiveError`). That is what forces the staging;
> it is not stylistic. Presence is checked with `primaryKeys()`, not `bulkGet`,
> so testing for existence does not structured-clone megabytes of thumbnail bytes.
>
> **Bug found while testing** (fixed in the same commit): the blob-id binding path
> set `existing.serverPhotoId = photo.id` in memory, then asked
> `existing.serverPhotoId !== photo.id` to decide whether to persist it â€” false by
> construction. Locally-uploaded rows could keep a null `serverPhotoId`
> indefinitely, breaking favourite toggles, face-cluster lookup and duplicate.
>
> Tests assert operation **counts**, not correctness â€” the old code produced the
> right mirror, just slowly, so no correctness test could ever catch a regression.

Remaining (the protocol itself):

> **Server half landed â€” `31fc322`.** Clients untouched; they still full-walk.
> The plan below assumed a `change_seq` column on `photos`. That is NOT what
> shipped, and the difference is the whole safety argument.
>
> **`photo_change_log` is a HINT, not a source of truth.** Its triggers say only
> "photo X may have changed" â€” never that X was deleted or that X is eligible.
> `fetch_delta` re-derives both from the live tables using the *same*
> `ELIGIBLE_PREDICATE` the full walk uses. So a trigger that over-fires costs one
> redundant row in one page and **cannot** produce a wrong answer. That inverts
> the asymmetry this file worried about: there is no longer an under-fire
> failure mode to protect against, because the log never asserts anything a
> reader trusts. One trigger covers all 9 delete sites, and "deleted" and
> "secure-hidden" collapse into a single branch.
>
> **Two things measurement changed, neither of which was in this plan:**
> - **Sequences are NOT unique.** `MAX(seq)+1` is evaluated once per *statement*,
>   so one secure-gallery insert touching several photos lands them all on the
>   same seq. A bare `seq > last` cursor drops every member of the group after
>   the first at a page boundary â€” **exactly #42's off-by-one, reintroduced
>   somewhere new.** Cursor is composite `"<seq>|<photo_id>"`. Verified RED:
>   bare-seq at `limit=1` returns `m1`, loses `m2`+`m3`.
> - A `UNIQUE` index on `seq` â€” the obvious "make the cursor simple" move â€”
>   would have made every multi-photo secure-add **fail outright**.
>
> **Confirmed by experiment, not assumed:** SQLite fires `AFTER DELETE` triggers
> for rows removed by `ON DELETE CASCADE`. Deleting a secure gallery cascades to
> its items, and that must un-hide its photos. The entire tombstone design rests
> on this, so a test pins it.
>
> Also verified RED by disabling the EGI insert trigger: three tests fail, and
> `applying_the_delta_matches_a_fresh_full_walk` fails by **retaining** the
> secured photo the full walk dropped â€” the ghost-row regression this file
> warned about, now under test rather than under discussion.

- [x] Monotonic change sequence maintained by trigger â€” shipped as `photo_change_log` (migration `033`), a keyed log rather than a column on `photos`. No FK on `photo_id`: a tombstone must outlive the row it describes. â€” `31fc322`
- [x] Tombstones covering all **9 delete sites across 7 files** + the eligibility subquery. Solved by the hint-not-truth design above rather than by exhaustively enumerating paths. â€” `31fc322`
- [x] Backstop. `photos_summary` now returns `head_seq`, deliberately **not** served from the TTL cache â€” a stale head would recreate exactly the busywork this removes. A client holding the current head skips `encrypted-sync` altogether. â€” `31fc322`
- [x] `GET /api/photos/encrypted-sync?since=<seq>` returning changed rows + `deleted[]` + `head_seq`. Migration backfills every existing photo, so `since=0` degenerates into a full sync and **cold-start needs no special branch**. â€” `31fc322`
- [x] Unified snapshot: counts **and** head sequence in one round trip. â€” `31fc322`
- [x] The eligibility predicate had been copy-pasted into 3 queries (delta adds 2 more) â€” now one const in `gallery/eligibility.rs`. A delta whose eligibility differs from the full walk's by one arm hands clients rows the grid will never show. â€” `31fc322`
- [x] Web: batch Phase 3 writes with `bulkPut` â€” already done in `f31b27b`.
> **Web `?since=` landed â€” `6a1b711`.** Android untouched.
>
> The pass moved out of `usePhotoSync` into `syncPass.ts` (skip / delta /
> full) with the cursor in `syncCursor.ts`. `usePhotoSync` is now only the
> React shell. **The full walk is kept as the recovery path, not as legacy** â€”
> it is self-healing (re-sends everything, client set-differences), and a delta
> feed is not. Every uncertainty in the new code therefore resolves to "full
> walk", because a needless full walk costs one slow pass while a wrongly-
> trusted cursor costs rows no future response will ever mention again.
>
> **Four hazards the plan above did not anticipate, each verified RED:**
> - **The cursor must live in IndexedDB next to the mirror, not localStorage.**
>   Separate stores get wiped independently and the failure is silent *and*
>   permanent: eviction empties `photos`, the surviving cursor says "already
>   current", and the gallery is empty forever. `readSyncCursor` additionally
>   refuses a cursor over an empty mirror â€” the partial wipe co-location can't
>   catch. `clearAllUserData` wipes it (Dexie v11 `syncState`).
> - **A pre-#38 server ignores `since` and answers with a FULL walk**, whose
>   `photos` are indistinguishable from a delta's. Reading it as a delta prunes
>   nothing while believing it pruned, then persists a cursor making that
>   permanent. The handshake is `deleted`: present-possibly-empty on a delta,
>   absent on a full walk â€” which is exactly why the server author made it
>   "empty rather than absent". Absent now forces the full path.
> - **Persist the FIRST page's head, not the last** (the server doc says so;
>   it is easy to get backwards). A change committed mid-walk lands above the
>   first page's head â€” keep the first and it is re-delivered, keep the last
>   and it is lost.
> - **A tombstone names a photo id, which may be the row's primary key OR its
>   `serverPhotoId`** (rows bound to a local upload's blob id). Resolving only
>   by primary key strands locally-uploaded rows forever.
>
> **Deliberate, documented narrowing â€” do not "fix" it by restoring the walks:**
> delta passes skip the four `fetchAllPages` blob enumerations. The change-log
> triggers cover `photos` and `encrypted_gallery_items` only, so a blob with no
> `photos` row does not move `head_seq`. That state is a *failed registration*,
> not a normal one (web uploads only `album_manifest` directly; Android always
> follows `uploadBlob` with `registerEncryptedPhoto`, which inserts the row and
> fires the trigger), and the next cold start repairs it.
>
> Also fixed in passing: deleting a photo now deletes its cached thumbnail
> bytes. The pre-#38 prune did not, so decrypted image content of deleted
> photos accumulated in IDB without bound.

- [x] **Web: adopt `?since=`.** Persist the last-seen sequence; poll `summary.head_seq` first and skip sync entirely when unchanged. â€” `6a1b711`
- [x] **Perf gate** on the client where it is observable: a 10k-row fixture at an unchanged head calls `encrypted-sync` **0Ã—**, `blobs.list` **0Ã—**, `blobs.download` **0Ã—**, `bulkPut`/`bulkDelete` **0Ã—**, and `photos.toArray` **0Ã—**. Verified RED by disabling *only* the skip fast-path â€” that test fails and nothing else does. 13 tests, 213 green (was 200). â€” `6a1b711`
- [ ] Android: same `?since=` adoption in `syncFromServerEncrypted`. **Read `web/src/gallery/hooks/syncPass.ts` first** â€” all four hazards above apply verbatim, and three of them (cursor lifetime, the `deleted` handshake, first-page head) are protocol-level, not web-specific. Android's equivalent of the co-location rule is: the cursor belongs in the Room DB that holds the mirror, cleared by whatever clears `photos` â€” NOT in `SharedPreferences`, which survives a database wipe.
- [ ] **Tombstone retention.** Rows for deleted photos accumulate without bound. Pruning needs a policy (e.g. drop after 90d) â€” a client offline longer than the retention window must be forced through a full reconcile, which is what `head_seq`/`total` are for. Not urgent at current library sizes; do not forget it.
- [ ] **Deploy required.** `033` backfills on first boot against the live 14,874-row library â€” cheap, but it is the first migration here with a data backfill, so watch it.

---

### A3 â€” #51 Crashing when scrolling a long list (High) â€” CONFIRMED

**Reported:** jitter and lag, then the app closes.

**Root cause: `JustifiedGrid` has no virtualization whatsoever.** [web/src/components/gallery/JustifiedGrid.tsx:166-211](web/src/components/gallery/JustifiedGrid.tsx#L166-L211) maps over **every** row and **every** item, mounting a DOM node per photo. At 10k photos that is 10k+ `<div>` + `<img>` nodes live simultaneously.

This interacts lethally with the thumbnail cache. [web/src/gallery/cache/thumbnailCache.ts:99](web/src/gallery/cache/thumbnailCache.ts#L99) caps at 500 entries and calls `URL.revokeObjectURL` on eviction â€” but the `<img>` elements are **never unmounted**, so scrolling past 500 tiles revokes blob URLs out from under mounted images. They blank, the loader re-fetches, which evicts more, which blanks more. That thrash is the "jittering," and the unbounded node + blob retention is the crash.

Secondary: `_evict()` ([line 83-95](web/src/gallery/cache/thumbnailCache.ts#L83-L95)) sorts the entire map on every insert past capacity â€” O(n log n) per insert where an LRU should be O(1).

**Fix:**

> **Web half landed â€” `20690ad`.** Android untouched.
>
> **The cache defect was worse than described above, and in a way that changes
> the symptom.** This file said eviction "revokes blob URLs out from under
> mounted images," implying the tile re-fetches and recovers. It cannot.
> `useThumbnailLoader` minted URLs via `blobUrlManager.acquire()` (ref-counted)
> while `thumbnailCache` revoked them with a raw `URL.revokeObjectURL` â€” two
> owners, and the one revoking was not the one counting refs. **Nothing in the
> tree ever called `blobUrlManager.release()`.** So after an eviction the
> manager still held a live entry pointing at a dead URL, and the next
> `acquire()` returned it. A cache *miss* is what re-entered the poisoned path,
> so the recovery mechanism was the failure mechanism. Tiles blanked
> **permanently**, for the rest of the session.
>
> Verified RED against the pre-fix code by reconstructing the old
> cache + manager collaboration: the reload returned `blob:mock/1` â€” the exact
> URL eviction had already revoked â€” and the "mounted" URL appeared in the
> revoked list. Both assertions fail on `HEAD~1`, both pass now.
>
> `blobUrlManager` is **deleted**, not left in place. It had no callers once the
> cache owned its own URLs, and leaving the second owner around is an invitation
> to reintroduce exactly this bug. It also installed a permanent 60s
> `setInterval` leak-detector in every session.

- [x] Virtualize `JustifiedGrid` â€” prefix-summed row offsets, rows intersecting the viewport plus a half-viewport overscan band, spacer-padded above/below. No new dependency. â€” `20690ad`
- [x] Cache capacity is now `max(base, pinned Ã— 3)` â€” a function of what is actually mounted rather than a magic 500. â€” `20690ad`
- [x] O(1) LRU via `Map` insertion order (`delete` + re-`set` on access), replacing the full sort on every insert past capacity. â€” `20690ad`
- [x] Mounted tiles hold a **counted pin** (`pin`/`unpin` from `useThumbnailLoader`) and are skipped by eviction, so a revoke for a live `<img>` is structurally impossible rather than merely unlikely. Pins are counted because one blob can be mounted by several tiles at once. â€” `20690ad`
- [x] Test: bounded mount count over a 10k-item fixture, asserted on the **pure windowing math** (`gridWindow.ts`) rather than a rendered DOM â€” this repo has no jsdom/testing-library and the mounted count is decided entirely by that math. Also pins the invariant `padTop + rendered + padBottom == totalHeight` at every scroll offset, which is what keeps document height independent of what is mounted (and `useScrollMemory` therefore correct). 25 tests, 200 green. â€” `20690ad`
- [x] **Android: both preconditions already hold â€” checked 2026-07-21.** The two things this item asks you to "confirm" are already true, so the client half is **not** the Android bug. `ui/components/JustifiedGrid.kt` is a port of the web compute-rows algorithm rendering into a **`LazyColumn`, one slot per row** ([JustifiedGrid.kt:204](android/app/src/main/kotlin/com/simplephotos/ui/components/JustifiedGrid.kt#L204), "collapses to O(rows + headers)"), and Coil's memory cache is explicitly bounded with a **128 MB ceiling** ([SimplePhotosApplication.kt:85-95](android/app/src/main/kotlin/com/simplephotos/SimplePhotosApplication.kt#L85-L95)). Android also never had web's two-owners-of-one-blob-URL defect, which is what made the web bug permanent. `JustifiedGridLayoutTest` already covers the row math.
- [ ] **So the remaining #51 Android work is the SERVER half, and it is still HYPOTHESIS.** The issue says "app/server crash"; with the client shown to be virtualized and bounded, check server memory during a long scroll. Do this **after** the redeploy, against the 14,874-row account â€” a thumbnail request storm is still the plausible mechanism, but it can no longer be blamed on an unvirtualized Android client.
- [ ] **Deploy/observe.** The virtualization is client-only, so a redeploy of the web bundle is enough â€” but the fix is only *observable* against a large library. Verify on CT132 with the 14,874-row account, not a test fixture.

**Note for whoever does the Android half:** do not port the pin/unpin design
blindly. Coil owns its own bitmap cache and does not have the two-owner problem
that made the web bug permanent; the web fix is about *ownership*, and Android's
equivalent question is whether the memory cache is bounded at all.

---

## Workstream B â€” Media conversion and playback

### B1 â€” #45 Logs don't show failed conversions/imports/encryptions (Low, but do it FIRST) â€” CONFIRMED

**Do this before B2/B3.** The user currently cannot tell *which file* failed. Every other conversion fix in this workstream is guesswork without it, and it is the cheapest item here.

**Root cause:** the success path audits, the failure path does not. [server/src/ingest.rs:249-259](server/src/ingest.rs#L249-L259) calls `audit::log_background(AuditEvent::MediaConvert, â€¦)` on success. The failure branch at [ingest.rs:437-452](server/src/ingest.rs#L437-L452) emits only `tracing::warn!` â€” which goes to the process log, **not** the `audit` table the Server Logs tab reads. `AuditEvent` ([server/src/audit.rs](server/src/audit.rs)) has no failure variants for convert/import/encrypt at all.

**Fix:** — **DONE**, commit `298fd99`
- [x] Added `AuditEvent::MediaConvertFailure`, `ImportFailure`, `EncryptionFailure` — **plus a fourth, `ThumbnailFailure`**, which the plan above missed even though it names the thumbnail site. Also added `audit::FAILURE_EVENTS`, the single list both the server filter and the web client read.
- [x] Emitted `MediaConvertFailure` / `ImportFailure` / `ThumbnailFailure` from the three `ingest.rs` failure branches with `filename`, `source_path`, `category`, `error`, `elapsed_ms`.
- [x] Audited the upload-path failure in `photos/upload.rs` (`origin: "upload"` in details distinguishes it from the ingest path).
- [x] `EncryptionFailure` from `photos/server_migrate.rs::record_encryption_failure`, on **every** attempt. Had to widen the existing `query_scalar` to `query_as` to pull `filename` alongside `encryption_attempts` — an audit row that names only a UUID is not actionable, which is the entire complaint in #45.
- [x] Web: "Failures only" checkbox in the Server Logs tab, + the 4 new event colours.
- [x] Test: `tests/test_90_pipeline_failure_audit.py` (4 tests) — corrupt `.mkv` upload, poll for the row, assert it names the file and carries the error. Verified RED against a rebuilt pre-fix binary: all 4 fail with "Saw 0 rows of that type". 6 server unit tests + 9 new web unit tests. 262 web green (was 253).

**Traps this exposed:**
1. **The filter had to be server-side.** Filtering the already-fetched 100-row page in the browser reports "no failures" whenever the newest 100 events happen to be logins — a *worse* answer than no filter, because it looks authoritative. `test_failures_only_finds_a_failure_buried_under_newer_successes` pins this with `limit=5`.
2. **`source_server` was never sent.** `ServerLogsTab` has always passed it to `listAuditLogs`, which never serialized it — the Source dropdown filtered nothing, silently, while the server implemented it fully (including `"local"` ⇒ `source_server IS NULL`). Fixed in the same pass; do not add a param next to a dropped one.
3. **The SSE stream ignores every filter.** The tab opens one `EventSource` for its lifetime and prepended *all* incoming events, so a `login_success` would land at the top of a "Failures only" list. Fixed with `matchesAuditFilters` + a `filtersRef` (a `useEffect` dep would reconnect on every dropdown change). This bug predates #45 — it also affected the event-type and IP filters.
4. `log_background` is a fire-and-forget `tokio::spawn`, so the audit row is **not** guaranteed to exist when the HTTP response lands. The E2E polls; a bare sleep either flakes or wastes time.

**New known-red discovered (NOT in the recorded baseline):** `test_18_media_conversion.py` has **6 audio failures** (`test_upload_aiff/m4a_converts_to_mp3`, `test_aiff_listed_as_audio`, 3 × `test_format_acceptance[aiff/aif/m4a-mp3]`) — all 403 Forbidden. Cause: `audio_backup_enabled` defaults **false**, and `upload.rs` correctly rejects audio outright under that policy; the tests never enable the toggle. Unrelated to any diff. Either enable the toggle in those tests or mark them skipped — do not "fix" the server.

### B2 â€” #40 Conversion ETA is wrong + add a 3-failure cap (High) â€” CONFIRMED

**Root cause (ETA):** [server/src/status.rs:70-83](server/src/status.rs#L70-L83) `progress_math` is a naive cumulative mean â€” `per_item = elapsed / done`, `eta = remaining * per_item`. It treats every queue item as equal cost. The queue deliberately mixes categories and sorts images first ([`conversion_priority`, conversion.rs:584](server/src/conversion.rs#L584) orders videos last), so the estimator spends the whole image phase learning a per-item cost that is orders of magnitude too small, then hits the video tail and the ETA explodes. It is also cumulative (early samples bias it forever) and the denominator can move mid-batch.

**Fix (ETA):** — **DONE**, commit `08bc838`. 416 server tests green (was 400), 275 web green.

> **Three corrections to the plan below, and one thing the plan did not mention
> at all that turned out to be the whole difficulty.**
>
> **1. "For video, duration where known" is a TRAP — rejected.** Duration is only
> known for the containers B3 routes through `transcode::probe`
> (`.mp4`/`.mov`/`.m4v`/`.webm`); `.mkv`/`.avi`/`.wmv` are matched by extension
> and never probed. Taking duration "where known" makes the weight unit
> inconsistent *within* the video category, and a rate estimator whose
> denominator silently changes units between samples is worse than one that is
> merely coarse. Weight is `size_bytes` for every category; because the rate is
> tracked **per category**, the unit only ever has to be comparable with itself
> and cancels before the per-category times are summed.
>
> **2. The plan had no answer for the unsampled category, which is the entire
> bug.** At the moment the last image finishes, **zero** videos have completed,
> so there is no measured video throughput to apply to the tail. An estimator
> built only from measurements must therefore return nothing for the whole image
> phase — i.e. stay silent for the first hour of a Takeout import, which is
> exactly when the old one was most wrong. So per-category **seed rates** are
> structurally required, not a nicety. The first real sample **replaces** a seed
> outright rather than blending: the seed carries no evidence about this machine.
>
> Idea considered and rejected: derive the video seed from the *measured* image
> rate via a fixed cost ratio, so it self-calibrates to the hardware. The ratio
> is not stable across boxes — the fast lane runs at full core width while video
> is GPU-session- or thread-capped, so a GPU box moves video's rate without
> moving image's at all. An absolute seed is wrong in a simpler, predictable way.
>
> **3. `EWMA(weight / secs)` is the obvious sliding rate and it is WRONG.** It is
> a time-unweighted mean of instantaneous rates, biased high by short deltas —
> and short deltas are precisely what a wide lane produces, because N concurrent
> encodes finish in a burst. The estimator keeps **two** EWMAs and divides:
> `EWMA(weight) / EWMA(secs)`, which converges to `Σweight / Σtime`. Verified:
> the average-of-ratios form reports **24.1s** where true throughput says ~100s.
>
> **Deltas are wall-clock between ticks in a category, seeded from a phase
> start** — never a sum of per-file durations, which would overstate elapsed by
> the lane width. What is measured is therefore throughput-at-current-lane-width,
> which is what the remaining queue will actually experience.
>
> **Known wart, accepted and documented in the source:** the *first* video sample
> underestimates the rate by roughly the video lane width (N videos start
> together, so sample 1 reads one file's weight over the whole phase). It
> overestimates the ETA for exactly one tick and samples 2..N correct it
> immediately. Overestimating is the safe direction.

- [x] Weight by work, not item count — `size_bytes`, uniformly (see correction 1). Completed weight vs total weight, per category.
- [x] Per-category throughput (image / audio / video); the ETA is the sum of per-category remainders. Summing is right because the pass drains the fast lane to exhaustion *before* the video lane — the phases are serial. Image and audio do overlap inside the fast lane, so those two remainders are summed when they partly run concurrently; that overestimates by a fraction of the short phase, which is the safe direction.
- [x] Sliding EWMA rather than the batch-lifetime mean — as a ratio of two EWMAs (correction 3).
- [x] `progress_math` kept pure and unit-tested, and **moved** to a new `server/src/progress.rs` alongside the weighted estimator, with its 4 tests. `conversion.rs` had been reaching into `crate::status::progress_math` — the conversion banner borrowing its ETA math out of a module whose docs are entirely about the *encryption* banner. The encryption banner deliberately **stays** on the count-based one: its queue items are one photo each, so there is no cost heterogeneity to correct and no reported bug.
- [x] Weighted ledger is **parallel to** `CONV_TOTAL`/`CONV_DONE`, not a replacement. Those are counts, they render the "3 / 4" banner text, and they carry #11's pinned-denominator fix — rewriting them in bytes reopens that. The client-declared upload batch (`POST /api/admin/conversion-batch/start`) carries a count and no sizes, so it keeps the count-based estimator as a **fallback**; #40's mechanism is the mixed autoscan queue, not the homogeneous upload path.
- [x] Weight is charged on the **failure** path too (`process_candidate` ticks in both arms). A failed transcode still burned the wall-clock the rate is measured against, so skipping it would make throughput climb silently as failures accumulate *and* leave that weight outstanding forever, so the ETA never drains.
- [x] Tests: 16 pure (`progress`) + 4 wiring (`conversion`, under the existing `global_state_lock`). **Verified RED against five plausible-but-wrong implementations**, each biting exactly the tests it should — see the trap list below.

**Traps this exposed:**
1. **A "sliding vs cumulative" test that asserts only `slower ⇒ bigger ETA` is
   vacuous.** A cumulative mean *also* goes up (10 MB/s → 4 MB/s ⇒ 20s → 40s).
   The RED check caught that my own first version of that test passed against
   the very implementation it existed to reject. It now asserts the estimate
   tracks the **recent** rate (>90s where cumulative lands at 40s).
2. **A ratio-stability assertion alone cannot catch a pooled rate.** Against a
   single global rate the mixed-queue ETA is perfectly *stable* across the
   boundary — and stably wrong (40s for a 2 GB video tail). The magnitude
   assertion added next to it is what bites.
3. **`eta_reset()` in `clear_start_clock` is NOT redundant with the one in
   `raw_start`**, and the first test written for it could not tell the
   difference because `raw_start` covers the common path. The interactive upload
   path (`progress_add`) re-arms the banner **without** going through
   `raw_start`, so an abandoned pass's outstanding weight is quoted to the next
   upload — verified RED at exactly `Some(2048.0)`, the 4 GB tail at the video
   seed.
4. **Enqueue must come AFTER `ConversionBatchGuard::start`.** The guard resets
   the ledger along with the counters, so enqueuing first is silently wiped.
5. Two web comments asserted the opposite of the new behaviour (`eta_seconds`
   "null until throughput is known"; "same throughput estimator as the
   encryption banner"). Both corrected — the D1/#44 precedent.

- [ ] **Follow-up: persist measured per-category rates across passes.** The seeds only govern a machine that has never converted anything, but today *every* server boot is that machine. Persisting the last measured rate per category would retire the seeds on any box that has run one pass. Not urgent; the seeds are conservative and decay on first sample.
- [ ] **Deploy + observe.** The estimator is pure and unit-tested; whether the seeded video rate is within ~2× on CT132's hardware has not been measured. Watch a real mixed Takeout pass and compare the reported ETA against the actual drain time.

**Root cause (repeat failures):** nothing anywhere persists a per-file failure count. On failure, `process_candidate` registers the ORIGINAL to avoid data loss ([ingest.rs:437-455](server/src/ingest.rs#L437-L455)), but several paths `return false` **without registering anything** â€” e.g. the register error at [ingest.rs:391-398](server/src/ingest.rs#L391-L398). A file that leaves no row is re-walked, re-converted and re-failed on every single autoscan pass, forever.

**Fix (3-strike cap):** extend the existing skip cache rather than inventing a new mechanism. `scan_skipped_paths` ([server/migrations/031_scan_skipped_paths.sql](server/migrations/031_scan_skipped_paths.sql)) already keys on `(user_id, rel_path)`, already stores `size_bytes` + `mtime`, and **already invalidates when either changes** â€” which is exactly the semantics you want ("if the file is replaced, try again").
> **3-strike cap DONE — `8bfe66a`. The ETA half is now done too, above.**
> 400 server tests green (was 382), 275 web green. **Four corrections to the plan
> below, the third of which changes what the fix actually is.**
>
> **1. The migration number was wrong.** `034` was never created — a numbering
> slip between `033` and `035`. Filling the gap would insert a version *below*
> three migrations already applied on every dev database. sqlx only validates
> applied-but-missing, not out-of-order, so it would probably work; "probably" is
> not a reason to reuse a number when the next one is free. Shipped as `038`.
>
> **2. There was no CHECK constraint to widen.** The plan said the migration must
> "allow `reason = 'conversion_failed'`". `031` declares plain `reason TEXT NOT
> NULL` and names the permitted values only in a comment, so the sole schema
> change is the counter.
>
> **3. The forever-loop is NOT the general conversion failure — and the worst
> case is a file that CONVERTS SUCCESSFULLY.** `process_candidate` registers the
> ORIGINAL on failure, so the file lands in `photos.file_path` and the next pass
> skips it via `existing_set`. The loop is the narrower set of paths that run a
> transcode and leave **no row at all**, and the dominant one is the *success*
> path's hash-dedup early return ([ingest.rs](server/src/ingest.rs)): on a Google
> Takeout library the same bytes sit in the date folder AND every album folder,
> the date copy registers first, so every album copy is fully transcoded and the
> output then discarded — **every pass, forever**. Migration 031 has skipped
> exactly this case for *native* files since the disk-thrash fix; the conversion
> walk never learned to.
>
> That verdict is **deterministic**, so it is recorded as a terminal
> `hash_duplicate` (carrying the content hash, so 031's delete-triggers re-admit
> the copy if the photo it deduped against is deleted) — **not** as three
> strikes. Spending three full transcodes to re-derive a known answer three times
> would have been its own bug, and it would then retire the file citing a reason
> that is not true. **A test that asserts "a failing conversion is retried 3×"
> without using a no-row path passes vacuously**, green because of `existing_set`
> rather than the cap.
>
> **4. One uniform "row exists ⇒ skip" rule is a silent ONE-strike cap.** The two
> 031 verdicts are terminal on sight; `conversion_failed` must fall through below
> the cap. That is the whole reason the verdict is a pure function
> (`photos/scan_skip.rs`) rather than an inline comparison in each of the two
> walks — the conversion walk had no skip check at all, and adding a third
> caller with *different* per-reason semantics is precisely how they drift.
>
> **Charged BEFORE the encode, not in the failure handler.** The files that most
> need retiring are the ones that never reach a failure handler — an ffmpeg that
> OOMs, a hard kill, a pass cancelled by the stuck-job watchdog. Same argument as
> `036`'s rendition cap.
>
> **`INSERT OR REPLACE` would have made the cap decorative.** It is what the rest
> of `register.rs` uses on this table, and it deletes + re-inserts, resetting
> `attempt_count` to DEFAULT 0 on every charge — an unbounded retry loop that
> reads as bounded. Only SQLite can answer which happened, so that is a DB test,
> not a reasoned-about one; `ON CONFLICT DO UPDATE` is load-bearing.

- [x] Migration `038_conversion_attempt_count.sql` (**not `034`** — see above): `attempt_count INTEGER NOT NULL DEFAULT 0` on `scan_skipped_paths`. No CHECK to relax.
- [x] Attempt charged before each encode via `register::charge_conversion_attempt` (`ON CONFLICT DO UPDATE`, returns the new count). Cleared on success so a file that converts on attempt 2 leaves no retirement waiting for a later delete + re-scan.
- [x] **Both** walks skip a retired candidate, through the shared `scan_skip::skip_verdict`. The conversion walk consults the cache for the first time; its check sits **before** the ffprobe (the stat it needs was hoisted above the probe), or a retired file still costs a process spawn every pass. `CONVERSION_MAX_ATTEMPTS` is a named constant, and `>=` not `==` so an overshot count stays retired (the #41 asymmetry again).
- [x] Deterministic dead ends (hash duplicate, on both the success and failure paths) record a **terminal** `hash_duplicate` instead of burning strikes.
- [x] `AuditEvent::ConversionRetired` on retirement, in `FAILURE_EVENTS`, amber in the Server Logs tab so "given up on" is distinguishable from "failed this pass". Carries a `retry_hint` — the escape hatch is not discoverable otherwise.
- [x] Admin escape hatch: `POST /api/admin/conversion/retry-failed`, scoped to `conversion_failed` rows only. The automatic hatch (change the file on disk) covers "the file was broken"; it does **not** cover "the *server* got better" — a new ffmpeg, a GPU driver fix — and telling an admin to touch 600 files is not an escape hatch.
- [x] Tests: 13 pure (`scan_skip`) + 5 DB-level (`register`). Verified RED twice — the naive uniform skip rule fails exactly the 2 tests asserting new behaviour, and `INSERT OR REPLACE` fails 3 of the 5 DB tests. The others pass in both states because they pin *preserved* behaviour; that is the honest result, not tests tuned to go red.
- [ ] **E2E**: a fixture that always fails is attempted exactly 3 times across 5 real scan passes. The unit + DB tests pin the arithmetic and the SQL; nothing yet drives five actual autoscan passes end to end.
- [ ] **Deploy required.** The cap only starts counting on the live box after a redeploy — and the Takeout duplicate-transcode loop is the one worth measuring before/after, since it is pure wasted CPU today.

### B3 â€” #46 Video.play failure on a specific .mp4 (Medium) â€” **root cause below was WRONG; corrected**

**Reported:** `20210520212438-5a45c3d4.mp4` â†’ "unable to play this video format."

> **âœ… MEASURED 2026-07-20 on live CT132 (all 742 mp4/mov/m4v ffprobed). The
> stated root cause was wrong, and the planned fix would have closed this issue
> without fixing the reported file.**
>
> The reported file probes as **`h264 / Main / yuv420p / 320Ã—240 + aac LC`** â€”
> dead centre of the browser-native allowlist proposed below. An allowlist
> passes it. Its actual defect is a **corrupt bitstream behind an intact
> container**: 3,331 `Invalid NAL unit size (-536345661 > 542)` errors on
> decode. It is a 2007 Apple MPEG-4 (ODSM/`mp4s` data tracks) that Google
> exported already broken, from Takeout's own **"Failed Videos"** folder.
>
> That folder is **not a reliable signal** â€” of its 5 files, 3 are corrupt
> (3,331 / 26,315 / 49,695 decode errors), 1 is entirely unreadable
> (`VIDEO0063.mp4` â€” no duration, no codec), and 1 is **perfectly healthy**
> (h264 High 1920Ã—1080, 0 errors).
>
> **Re-encoding is still the right fix, for a different reason:** ffmpeg is
> lenient where browser decoders are strict, so it salvages what it can read.
> Measured: 51.15s of corrupt input â†’ **28.03s of clean, 0-error, playable
> output**. The 23s tail is unrecoverable and the user must be told, not handed
> a silently shorter video.
>
> **The codec allowlist is still worth building â€” for the other 38 files.**
> Library codec totals: **704 h264, 28 hevc, 10 mpeg4**. The 38 non-native are
> 28 hevc (incl. one `High 10 / yuv420p10le`) + 10 mpeg4 Simple Profile, all in
> `.mp4`/`.mov`, none ever queued. So: **two independent defect classes, one
> symptom.** Probe for **decodability**, not codec identity.

**Root cause (as corrected):** format detection is purely by file extension. [server/src/conversion.rs:487-560](server/src/conversion.rs#L487-L560) `conversion_target` matches on `ext` and `.mp4` is not in the video list, so an mp4 is assumed browser-native and **never transcoded** â€” regardless of whether its codec is decodable *or whether its bitstream is real*. [server/src/photos/web_preview.rs:13-30](server/src/photos/web_preview.rs#L13-L30) `needs_web_preview` has the identical blind spot.

Same class as the GIF misdetection fixed by magic-byte sniffing (memory `c1-gif-detection-14`): **probe, don't guess.**

**Fix:**
- [x] `ffprobe`-backed probe at [server/src/transcode/probe.rs](server/src/transcode/probe.rs) returning codec, profile, pixel format, resolution â€” **plus `probe_decode_health`**, which the original plan had no equivalent of and without which the reported file is not fixed. â€” `65389a7`
- [x] Browser-native allowlist: H.264 Baseline/Constrained Baseline/Main/High, 8-bit `yuv420p`/`yuvj420p`. Matched on the **full** profile string â€” `High 10` shares a prefix with the native `High`, and prefix-matching would pass the library's one 10-bit file. â€” `65389a7`
- [x] Route `.mp4`/`.mov`/`.m4v`/`.webm` through the probe at ingest. `conversion_target` keeps its signature (load-bearing in sync contexts â€” MIME selection, upload gating); the probe is layered on top. â€” `65389a7`
- [x] Got the actual failing file off the live box and probed it before writing the fix. **This is the step that caught the wrong root cause** â€” do not skip it on B2/B4. â€” `65389a7`
- [x] Test: HEVC-in-MP4 and MPEG-4-in-MP4 fixtures are queued; an H.264-in-MP4 fixture is left alone. Real FFmpeg fixtures through the real probe, since the defect is that the old path never looked inside the file. Verified RED against a simulated extension-only path: the two new-behaviour tests fail, the two preserved-behaviour tests pass. 310 green (was 295). â€” `65389a7`
- [ ] **Backfill â€” this fix helps NEW imports only.** The `existing_set` check was deliberately moved *ahead* of the probe so idle autoscan passes spawn zero ffprobes (protecting the migration-031 disk-thrash fix), which means the 38 already-registered offenders are never re-examined. **Without this the user's library is unchanged.** Runs automatically â€” the project is in beta and breaking changes are expected. **It must be a one-shot pass, not a new autoscan responsibility:** persist a completion marker so it probes each registered `.mp4`/`.mov`/`.m4v`/`.webm` exactly once and never re-walks them on subsequent idle passes, or it re-introduces the disk thrash the `existing_set` ordering exists to prevent. 38 known offenders â€” bounded, so queue at the ladder's low priority and let it drain.
- [ ] **Corrupt-file honesty.** A salvage re-encode silently shortens the video (51s â†’ 28s). Surface the loss â€” this is the natural first consumer of B1/#45's failure audit events.
- [ ] **`VIDEO0063.mp4` has no decodable video stream at all.** Currently logged and left unregistered, because queueing it would guarantee a failure every pass forever. Needs the terminal "unplayable" state from B2/#40's 3-strike cap.
- [ ] `needs_web_preview` still has the same extension-only blind spot; the probe is not wired into it yet.

### B4 â€” #49 Resolution ladder + player quality picker (High, largest item in this file)

**Reported:** >1080p sources should also produce a 1080p rendition; gear icon in the player for resolution choice; default highest on Wi-Fi, lower on cellular; Android needs a cellular data-saver toggle.

**Current state â€” CONFIRMED there is nothing to build on.** [server/src/transcode/ffmpeg_gpu.rs:15-158](server/src/transcode/ffmpeg_gpu.rs#L15-L158) `build_video_transcode_args` produces exactly **one** output at **source resolution** for every backend. The only scale filters present force even dimensions / pixel format (`scale=trunc(iw*sar/2)*2:trunc(ih/2)*2`); there is no downscale, no ladder, no rendition table, no variant serving.

**This is a feature, not a bug fix â€” scope it separately and do it LAST.** Do not let it block #38/#42/#51.

> **âš ï¸ MEASURED 2026-07-20 â€” the rung rule below must key on the SHORT EDGE,
> not on height. Keying on height is wrong twice, and the live library contains
> both traps.**
>
> - **71 of 742 videos are portrait**, 14 of them exactly `1080x1920`. A
>   `height > 1080` test flags every one â€” but `1080x1920` **is already the
>   1080p tier**. A naive `scale=-2:1080` would *downscale* them to 608Ã—1080,
>   degrading files that needed no rung at all.
> - **4 videos are `2288x1088`.** 1088 is macroblock-padded 1080. A strict
>   `> 1080` test spends a full 4K-class re-encode to save 8 pixels. **The rule
>   needs a tolerance band, not `>`.**
>
> Correct sizing by short edge > 1080: **140 need a rung**, 602 do not.
> Of the 140: 126Ã—`3840x2160`, 6Ã—`1920x1440`, 4Ã—`7680x4320` (8K), and the
> 4Ã—`2288x1088` that must be excluded â‡’ **true demand 136**, dominated by 4K.
>
> Note the 8K files: a `7680x4320` source is a 2-rung decision (source + 1080),
> and re-encoding four of those is not a background afterthought.

Phased plan:

> **Server planning + storage + queue landed â€” `aee50f8`, `25385bb`, `f3cd439`,
> `7a12582`, `d1a3079`. Nothing generates a rung yet; that is the next commit.**
>
> **The DB's geometry is not the encode's input â€” measured 2026-07-21.**
> `photos.width`/`height` disagree with this file's ffprobe census twice, and
> both were found by measuring the live box before writing the candidate query:
> - **58 of 698 videos have no recorded geometry at all** (`width`/`height` â‰¤ 0).
>   A prefilter requiring `min(w,h) > threshold` cannot see them, so selecting
>   purely on stored geometry skips them **forever**. They are selected anyway
>   and resolved by a probe.
> - **Orientation is transposed.** The census says 126 Ã— `3840x2160`; the DB
>   holds 78 Ã— `2160x3840` **plus** 26 Ã— `3840x2160`, and the same swap shows on
>   `1440x1920` (census `1920x1440`) and `4320x7680` (census `7680x4320`).
>   Cause unconfirmed â€” most likely rotation side-data applied on one side only.
>
> The transposition is survivable **only at selection time**, because the rung
> rule keys on `min(width, height)`. That is the short-edge rule earning its keep
> a second time. It is **not** survivable in the transcode: `rung_dimensions`
> returns the orientation it was given, so a transposed pair fed to `scale=W:H`
> squashes a landscape frame into a portrait box. **The generation pass must take
> its geometry from probing the file it is about to encode.** The columns narrow
> 698 videos to ~114 and are good for nothing else.
>
> **Two trigger bugs in `035`, both verified against SQLite rather than reasoned
> about** (fixed in `036`):
> - An upsert taking the `DO UPDATE` branch fires **UPDATE** triggers, not INSERT
>   triggers. `upsert_rendition` is `INSERT ... ON CONFLICT DO UPDATE`, so the
>   moment a rung becomes playable is an UPDATE â€” and `035` has no UPDATE
>   trigger. The photo is never nominated and **the picker stays empty until a
>   full walk that, post-#38, may never come.** This already affected any
>   re-encode of an existing rung.
> - Claim rows carry no locator, so nominating on INSERT wakes every client once
>   per attempt to deliver a picker that has not changed.
>
> Both now nominate exactly when the set of *playable* renditions changes.

- [x] **Schema.** Migration `035_video_renditions.sql`. Keyed on `short_edge`, **not** height â€” 14 live videos are `1080x1920` and 4 are `2288x1088`, and keyed on height the first group collides with a genuine 1080p rung while the second invents one 8 pixels tall. `blob_id` is the load-bearing column, not `file_path`: neither client plays from `GET /photos/:id/file`. â€” `25385bb`
- [x] **Rung selection as a pure function**, keyed on `min(width, height)` with a 10% tolerance band, unit-tested against `1080x1920`, `2288x1088`, `3840x2160`, `7680x4320`, `1920x1440`. The census test reproduces the measured demand of 136 from the shape counts, pinning the arithmetic to the measurement rather than to an assertion about it. â€” `aee50f8`
- [x] **Transcode.** `build_video_transcode_args` takes a target rung; dimensions arrive precomputed from `ladder::rung_dimensions` rather than as an ffmpeg scale expression, so the orientation-preserving arithmetic is a unit test instead of a device test. Found in passing: `convert_video`'s **CPU fallback held its own hardcoded copy of the scale filter** â€” harmless at source resolution, a silent correctness bug under the ladder (GPU asks for a rung, hardware encode fails, fallback runs at full resolution, result is recorded as the 1080p rendition). â€” `f3cd439`
- [x] **The source rendition must actually be playable.** Gated on `is_browser_native` + decode health. A lone unplayable source yields *no* picker rather than a one-entry picker whose only option fails. â€” `7a12582`
- [x] **Candidate queue + 3-strike cap** (`036`). The candidate set is self-limiting on success â€” a produced rung leaves it forever â€” but **not on failure**, so without a cap every sweep re-attempts a 4K-class re-encode forever on 114 candidates. Attempts are charged **before** the encode: a file that OOMs or hard-kills ffmpeg never reaches an error handler, and that is precisely the file needing retirement. Cheapest-first ordering keeps the 4 8K sources off the head of the queue. `rung_threshold` is shared with the SQL prefilter rather than re-derived, and a test requires the two to agree on every live shape. â€” `d1a3079`
- [x] **Generation pass.** Decrypt â†’ probe â†’ plan â†’ transcode â†’ encrypt â†’ record, in `transcode/rung_generate.rs`, wired after `run_conversion_pass` at all three autoscan sites. Chunked writer, `content_hash` NULL, geometry from the probe. â€” `60d555d`

> **Generation pass landed â€” `60d555d`. The ladder now produces renditions;
> nothing serves them yet.**
>
> **The transposition trap is real and now under test.** A portrait `1440x2560`
> source registered with the live transposition (`2560x1440`) is encoded by
> driving real ffmpeg, and the assertion is on the **produced file's** probed
> dimensions, not the row written about it. Against a DB-geometry implementation
> it comes out `1920x1080` â€” a visibly squashed frame that nothing downstream can
> recover. Those are the two things that can disagree, so the test asserts the
> one that costs the user.
>
> **A third verdict was missing, and its absence turned a healthy library into a
> retired one.** The prefilter is deliberately wider than the ladder rule so it
> can see the 58 videos with no recorded geometry. Most of them need no rung â€”
> and with only "produced" and "failed" available, that answer had nowhere to go:
> the row keeps both locators NULL, the candidate query reads it as still owed,
> and the file is re-probed every sweep until the attempt cap retires it with a
> warning claiming it will never get a picker. `037` adds `not_needed`, which is
> terminal, costs no attempt, and stays invisible to every picker. Verified RED
> by dropping the query arm: the photo comes back.
>
> **`035` queued the user's original video for garbage collection.** The source
> rung points at the blob the *photo* already owns â€” a second reference, not a
> copy â€” and the orphan trigger could not tell that from a rendition that owns
> its bytes. The sweeper is required to re-check references, so this was not by
> itself data loss; but it made the safety of an original 4K video depend on a
> sweeper that does not exist yet knowing about a case nobody had written down.
> `037` guards the trigger on `is_source = 0` instead, so the queue can only ever
> name bytes a rendition owns. Verified RED against `035`.
>
> **Photos in the encryption backlog are deferred, not encoded.** Both clients
> play from blobs, so a `file_path` rendition produced for one of the 2,494 rows
> about to be encrypted is bytes nothing can play â€” and the encode would have to
> be repeated after the migration moved them anyway.
>
> Also: `convert_video`'s CPU fallback is reached through `conversion::
> transcode_to_rung`, which shares the batch thread budget rather than taking
> every core â€” a background rendition must never be why a first-pass conversion
> is slow.
- [ ] **Cost control.** This doubles encode work for every 4K video. Sequencing is done (`60d555d` runs the sweep after `run_conversion_pass`, one file at a time, under a wall-clock budget) and the encode shares the batch thread budget. Still open: the sweep is **serial**, so a 114-file backlog of mostly-4K sources drains slowly. Decide whether to give it a lane of the existing two-lane parallelism (`SIMPLE_PHOTOS_CONVERSION_JOBS`) or leave it serial deliberately.
- [ ] **Orphan sweeper.** `035`'s cascade queues unreferenced rendition blobs into `orphaned_rendition_blobs`, and **nothing drains it yet** â€” so deleted 4K videos leak their rendition bytes. The sweeper must re-check references before unlinking, and must remember that `blobs.content_hash` dedup means one blob can back several photos. **`037` removed the worst trap** â€” source rungs no longer reach this queue at all, so it can only name bytes a rendition owns.
> **Serving + API landed â€” `8564636`. The server half of #49 is now complete;
> everything remaining is client work.** 378 tests green, was 363.
>
> **This item was hiding a live secure-album bypass, and shipping the picker
> without noticing would have published it.** `is_secure_item` matched a photo's
> own id, its encrypted blob and its thumb blob â€” every id a secure-gallery row
> actually *records*. A rendition is derived content in its own blob, and
> `encrypted_gallery_items` has no column that will ever name it, so
> `GET /api/blobs/<rung>` needed **only an account session, no unlock token**.
> The ladder generates only for *eligible* photos, so the exposure is created by
> securing a video **after** its rung was produced â€” ordinary use â€” and what
> leaks is a full-quality copy of the video the album exists to hide. The new
> third arm resolves the other way: blob â†’ owning photo â†’ is that photo secure.
> Verified RED. **Consequence to keep in mind: any future derived-content blob
> (a preview, a sprite sheet, an audio extract) has this same hole by default.**
>
> **The change-log triggers decided the API shape, not preference.** `035`/`036`
> nominate the photo when its *playable* rendition set changes â€” machinery that
> only pays for itself if renditions ride the sync record. A per-video endpoint
> would have made those triggers dead weight *and* put a round trip in front of
> every video opened. So renditions are hydrated onto `EncryptedSyncRecord` by
> one batched query per page, shared by the full walk and the delta. Verified RED
> by hydrating only `fetch_page`: a client cannot tell which feed produced a row,
> and #38 makes the full walk the delta's *recovery* path â€” a ladder visible
> through one and not the other is a "repair" that strips the user's picker.
>
> **Encrypted mode needed no new route.** A rendition is a blob, which is how
> both clients already play video, so serving reduced to the authorisation fix.
> `?rendition=<short_edge>` on `/photos/:id/file` exists for **unencrypted
> installs only** (`store_plaintext`), and works by swapping the locator so
> renditions inherit range support, chunked streaming and conditional requests
> instead of reimplementing them.
>
> **Two silent defects found reviewing the swap:**
> - The rung was served under the **parent photo's** mime type. The ladder always
>   emits H.264 in MP4; 10 live videos are `.mov`, so their downscales would have
>   gone out as `video/quicktime`. `ServeTarget` is now destructured
>   exhaustively (**no `..`**) so a field added to it cannot compile until the
>   handler applies it â€” this is the one that was missed on the first pass, and a
>   unit test on the pure function provably cannot catch it.
> - Rungs shared the photo's ETag. Sizes differ between rungs *today*, so it
>   would usually work â€” and would return a cached 4K copy for a 1080p request
>   the moment two rungs ever coincided in size.
>
> `file_path` is deliberately **absent from the wire shape**: a client cannot
> fetch a storage path, so shipping one publishes the storage layout for nothing.
> `short_edge` doubles as the selector.

- [x] **Serving.** Encrypted mode: authorising the blob, via the `is_secure_item` arm above. Unencrypted mode: `?rendition=<short_edge>` on `/photos/:id/file`, range support intact. â€” `8564636`
- [x] **API.** `renditions` on the sync record, from `list_renditions_for_photos` (one query per page). `list_renditions` gained its first production caller â€” the `?rendition=` lookup goes through it so "unproduced rungs are not offerable" has one home. â€” `8564636`
- [ ] **`rung_queue::geometry_is_known` is still dead** (the last `dead_code` warning). Its doc claims "the generation pass branches on this" â€” `rung_generate` does not. Either use it to distinguish "narrowed by the prefilter" from "selected blind" in the sweep's log, or delete it and the doc. Do not leave a comment asserting a caller that does not exist.
- [ ] **Securing a video now removes its picker**, because the sync feed is the only delivery path and secure photos are not in it. Not a regression (there was no picker before) and arguably correct, but the secure viewer will show a single-quality video where the main gallery showed a choice. Decide whether the secure gallery's own item listing should carry the ladder too.
> **Both players read the same field â€” no new fetch.** `renditions[]` arrives on
> every sync record (highest first, `is_source` marking the original) and is
> already in the local mirror by the time the viewer opens. Per entry: use
> `blob_id` with the existing blob-download path when set; otherwise
> `GET /photos/:id/file?rendition=<short_edge>` (unencrypted installs only).
> An **empty array is the normal case** â€” it means one quality, so render no gear
> icon at all rather than a picker with a single entry.
>
> Web needs the mirror widened first: `syncReconcile.ts` persists a fixed column
> set into IDB, so `renditions` must be added there or the viewer will never see
> it however correct the server is. Android's Gson DTO (`PhotoDto.kt`) needs the
> same field; Gson ignores it silently today, which is why the server change did
> not break either client.

> **Web picker landed â€” `163dcc0`. Android is now the only client half left.**
> 244 web tests green, was 213.
>
> **The mirror had to be widened first, and that was the load-bearing part.**
> `renditions` was arriving on every sync record and being dropped on the floor
> by `syncReconcile`'s fixed column set. Verified RED: 4 of the 5 new reconcile
> tests fail with the writes removed. Note *which* 4 â€” the fifth is a negative
> test asserting no write happens, so it passes in both states, which is the
> honest result rather than a broken test.
>
> **Three traps, none of them visible from the server contract:**
> - **The source rung is not a separate download.** `is_source` points at the
>   blob the photo *already owns* â€” the same second-reference fact `037` needed
>   to stop the orphan trigger queueing the user's original. So "Original"
>   reuses the URL the viewer already holds and fetches nothing. Treating it as
>   just another rung re-pulls a full 4K video the browser has in hand.
> - **Rendition bytes must never enter `db.fullPhotos` or the preload cache.**
>   Both are keyed by the *route* blob id â€” the original. The obvious
>   implementation (reuse `loadEncryptedMedia` with the rendition's blob id)
>   caches the downscale under the original's key, so the next open serves it as
>   the original and "Original" in the picker plays the rendition. That reuse is
>   the bug, so the download path deliberately bypasses that hook.
> - **Only URLs the hook minted are revoked.** `mediaUrl` is owned by the
>   preload cache; revoking it from the picker would blank the video on the next
>   swipe back â€” the two-owners-of-one-blob-URL defect that made the thumbnail
>   cache bug *permanent* in #51.
>
> **Edit mode reverts the selection, not just the gear icon.** Hiding a control
> does not change what `mediaUrl` points at, and a crop or trim saved while a
> 1080p rung is on screen would re-encode the downscale over the 4K master.
>
> **`undefined` and `[]` renditions are the same state** ("one quality") and
> arrive from different places â€” a pre-#49 server yields the former, a #49
> server sends the latter for the ~600 videos needing no rung. Collapsing them
> keeps the first pass after a server upgrade from rewriting the whole library.
>
> Rungs with a null `blob_id` are filtered out. The viewer has no plaintext
> path, so on an unencrypted install the picker is genuinely absent rather than
> offering a menu entry that silently does nothing.

- [x] **Web player.** Gear icon in `VideoControls.tsx` â†’ resolution menu; choice logic is pure in `gallery/renditionChoice.ts`, URL/lifetime ownership in `hooks/useVideoRendition.ts`. Default via the Network Information API (`saveData` / `type === "cellular"` / slow `effectiveType`), falling back to highest â€” absent all three (Safari, Firefox) it defaults to highest, which is the right way to be wrong. The cellular cap is absolute (1080p), not one-rung-down: on an 8K source one rung down is 4K. â€” `163dcc0`
- [ ] **Web: device-verify against a real >1080p video on CT132.** The picker is unit-tested on pure logic and typechecks, but the quality *swap* (playhead restore, pause-state restore, no flash of the original) has never run in a browser â€” this repo has no jsdom. Do it with the 14,874-row account, not a fixture.
> **Android picker landed â€” `5b46040`. #49 is now client-complete; everything
> left in this section is server-side cleanup.** Android unit tests **154 green**
> (was 132). `.\gradlew.bat :app:testDebugUnitTest` + `:app:assembleDebug`.
>
> **The mirror had to be widened here too, and again it was the load-bearing
> part** â€” but the Android defect is worse than web's was. `syncFromServerEncrypted`
> takes an early `continue` for any photo already in Room, back-filling only
> subtype/burst/motion. Rungs are produced by a **background sweep long after the
> photo synced**, so that branch is not an edge case: it is the only case that
> ever matters. Every video already in the mirror â€” i.e. all of them â€” would
> have kept an empty ladder forever and the gear icon would never have appeared,
> no matter how correct the server was. The merge path had the same hole.
>
> **Two of web's three traps do not exist on Android, and the reason matters
> more than the fact.** Web downloads and decrypts a whole blob into an object
> URL, which is what forces it to worry about caching a rendition under the
> original's key and about revoking a URL it does not own. Android streams every
> quality as `spblob://<blobId>` through `MediaBlobDataSource` over range
> requests â€” no download, no cache keyed by blob id, no URL to own. A quality
> switch is *only* a change of URI. **Do not port web's machinery here**; this
> file previously guessed the opposite ("Android's equivalent is
> `MediaBlobDataSource`/Coil-adjacent caching keyed by blob id" â€” there is no
> such cache; the geometry cache is per-instance and explicitly re-resolves on a
> different blob id).
>
> **Trap 1 survives and pays off structurally.** The source rung's `blob_id` is
> the photo's own `encrypted_blob_id` ([rung_generate.rs:412-416](server/src/transcode/rung_generate.rs#L412-L416)),
> so "Original" resolves to the exact URI already loaded and
> `PhotoViewerScreen`'s existing `uri != activeVideoUri` guard turns it into a
> genuine no-op rather than a re-prepare.
>
> **Three defects found reviewing my own work, none caught by a test:**
> - **Re-picking "Original" while it is already playing left a stale resume
>   snapshot.** `effectiveUri` does not change, so the swap effect never fires,
>   and the snapshot sat until some *later* quality change consumed it â€” seeking
>   the video back to wherever the user was at the earlier tap. Web guards the
>   same case for the same reason; the symptom differs because the mechanisms do.
> - `playWhenReady = true` was unconditional on load, so a video the user had
>   **paused** would silently resume on every quality change. Both directions are
>   restored now, and the playhead goes through `setMediaItem(item, startPositionMs)`
>   rather than a `seekTo` after `prepare()` â€” seeking afterwards visibly starts
>   at zero and jumps.
> - `rememberQualityConstrained()` was read per page, registering a
>   `ConnectivityManager` callback for every page the pager keeps alive. Hoisted
>   to the screen.
>
> **`org.json.JSONObject.optString` returns `""` for an absent key, not null** â€”
> so the obvious converter turns an unencrypted install's null `blob_id` into an
> empty string, which survives the picker's `blobId != null` filter and then
> builds a hostless `spblob://` URI that throws at playback. Verified RED: the
> naive implementation fails **two** tests, the second being the downstream
> symptom (the unstreamable rung reaching the menu).
>
> Also required `testOptions.unitTests.isReturnDefaultValues` â€” `android.util.Log`
> is a stub that **throws** in JVM tests, so every recovery path in this codebase
> was untestable: a test exercising one died on the log line instead of asserting
> the recovery. `ConvertersTest` had not hit it only because its existing case
> returns early before the catch block.

- [x] **Android player.** Picker in `VideoControlsOverlay`; selection state and
  effective-URI resolution in `VideoPlayerPage`; choice arithmetic ported pure to
  `data/media/RenditionChoice.kt` with 22 unit tests. `renditions` added to
  `PhotoDto.kt` (`EncryptedSyncRecord` + `RenditionDto`), persisted on
  `PhotoEntity` via a `Converters` JSON round-trip (Room `v12`, destructive), and
  landed on already-synced rows by `PhotoDao.updateRenditions` guarded with
  `renditionsEqual`. Edit mode reverts the **selection**, not just the icon. â€” `5b46040`
- [x] **Android setting.** "Cellular data saver" in Settings â†’ Display, default
  **ON**. The asymmetry decides the default: off spends someone's mobile data on
  a 4K stream they never asked for, on costs a sharper picture until they find
  the switch. Metered state comes from `NET_CAPABILITY_NOT_METERED` (which also
  catches a metered *wifi* hotspot â€” a `TRANSPORT_CELLULAR` check would miss it
  and cost the user the same money), and `isConstrained` requires **both** the
  setting and a metered link, so with the saver off nothing is ever downgraded.
  The viewer reads DataStore directly, so the toggle reaches an open player. â€” `5b46040`
- [ ] **Android: device-verify on the S21+ harness.** Same gap as web: the
  arithmetic and the persistence are unit-tested, but the *swap itself* (playhead
  restore, pause-state restore, the picker actually appearing) has only ever run
  in a compiler. Compose UI tests live in `androidTest` and need a device. Do it
  against a real >1080p video on CT132 â€” and note that **nothing has generated a
  rung on the live box yet**, so the picker cannot appear there until the server
  is redeployed and the sweep drains.
- [ ] **Backfill.** Task to generate 1080p rungs for existing >1080p videos. It runs automatically â€” the project is in beta and breaking changes are expected. Measured cost is bounded and known â€” 136 files, 126 of them 3840x2160 and 4 of them 8K â€” so run it at the ladder's low priority and let it drain. The 8K files are a 2-rung decision each and are not a background afterthought.
- [ ] Tests: ladder selection logic (which rungs for which source height) as a pure unit test; rendition serving + range requests in E2E; picker default selection per network state.

---

## Workstream C â€” People / Pets

### C1 â€” #48 Face selection centering and missing thumbnails (High) â€” CONFIRMED

Four distinct defects in one issue. Treat them as four checkboxes.

**(a) Faces sit up and to the left â€” the zoom math is wrong, identically, on both platforms.**

Web: [web/src/utils/thumbnailCss.ts:285-303](web/src/utils/thumbnailCss.ts#L285-L303) `computeFaceCropStyle` sets `transformOrigin: cx cy` + `scale(zoom)`. **Scaling about a transform-origin holds that point stationary â€” it does not move it to the centre.** A face centred at (0.30, 0.25) stays at 30%/25% of the tile: up and to the left, exactly as reported. The accompanying `objectPosition: cx cy` has the same flaw (percentage object-position aligns the image's P% point with the *container's* P% point).

Android: [LibraryFeatureScreens.kt:177-192](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L177-L192) makes the **same** mistake â€” `graphicsLayer { scaleX = zoom; scaleY = zoom; transformOrigin = TransformOrigin(cx, cy) }`. Compounded by `contentScale = ContentScale.Crop` into a 1:1 tile: the bbox is normalised against the **full aspect-preserving thumbnail**, but it is applied after a centre-crop to square, so the coordinate spaces do not match. That is why Android is "far off" while web is merely offset.

The correct formula already exists in this repo â€” [PhotoInfoPanel.tsx:80-97](web/src/components/viewer/PhotoInfoPanel.tsx#L80-L97) `FaceCrop` uses the proper normalised sub-rectangle mapping (`(bbox_x / (1 - w)) * 100%`). Two implementations of the same operation, one right, one wrong.

**(a) — DONE**, commit `5c4d776`

- [x] `computeFaceCropStyle` rewritten. The position formula above was right — `p = (c - z/2)/(1 - z)` — but **the mechanism this file prescribed was wrong**, the third wrong plan in this document. `z = 1/zoom` holds only for a square source. Under `object-fit: cover` the visible fraction is per-axis and aspect-dependent, and along the **uncropped** axis the whole image is visible (`z = 1`), so `object-position` has zero freedom and cannot centre anything — no amount of correct arithmetic rescues a crop built on `cover`. Every formula starting from "where does the face land after a cover crop?" needs the image aspect ratio, which we do not have for a cached thumbnail. — `5c4d776`
- [x] **The fix places the window itself** rather than asking where the face landed. The window is the bbox scaled about its own centre by a single factor `k`, drawn with explicit width/height + `object-fit: fill` (the `getThumbnailStyle` rotated-crop precedent). A face square in *pixels* yields a square window: `zx`/`zy` differ by exactly the photo's aspect ratio, which is the term that cancels. **That is why it needs no aspect ratio at all** — the property the old comment claimed and the old code did not have. — `5c4d776`
- [x] `z → 1` degenerates correctly: the axis is fully visible, so there is no pan freedom and no solution; `facePosition` returns the midpoint instead of dividing by zero. — `5c4d776`
- [x] One helper: `faceCropRect` (web) / `FaceCrop.kt` (Android), parameterised by `targetFraction` + `minVisibleFraction`. `FaceCrop` in PhotoInfoPanel — the copy that was already correct — now calls it at `targetFraction: 1`. A third copy, the four-nullable-column bbox guard, was inlined at each cluster call site and is now `clusterFaceCropStyle`. — `5c4d776`
- [x] Android: `TopStart` scale+translate as predicted (memory `android-crop-display-bug` was right), **plus `ContentScale.Crop` → `FillBounds`** — which the plan above missed. Correcting "for the centre-crop coordinate space" is not possible without the aspect ratio; the centre-crop has to be *removed*, not corrected. Aspect is preserved by `zx`/`zy` differing, not by the scaler. — `5c4d776`
- [x] Unit-tested on both platforms — 14 web + 9 Android, no device required. — `5c4d776`

> **The existing web suite asserted the bug.** `thumbnailCss.faceCrop.test.ts` carried a test named *"centres the crop on the face centre"* whose body asserted `objectPosition: "75.00% 75.00%"` for a face centred at 0.75 — it pinned the face staying exactly where it was. The assertion and the defect agreed, so #48 shipped with a green suite. Both suites now assert the **property** — invert the produced geometry, check the face centre lands at 0.5 — rather than the formula's own output, so they cannot re-agree with a broken implementation. Verified RED against the old formula: it lands the face at 0.30 where the property demands 0.50, precisely the reported "up and to the left".

**(b) Many People albums have no thumbnail.** [server/src/ai/handlers.rs:283-314](server/src/ai/handlers.rs#L283-L314) `fetch_face_clusters` LEFT-JOINs the representative detection, so `rep_bbox_*` is legitimately NULL when it cannot resolve â€” the client then renders the placeholder. **HYPOTHESIS, and it likely chains to A1:** the representative *photo* is resolved client-side against the local mirror, and the web mirror is missing every unencrypted row (`usePhotoSync.ts:205`). A cluster whose representative happens to be an unencrypted photo has no thumbnail on web but would on Android.
- [ ] Fix A1 first, then re-measure how many clusters are still thumbnail-less.
- [ ] For the genuine remainder: fall back to the next-highest-confidence detection whose photo *does* resolve, rather than giving up on the representative.
- [ ] Log (don't silently placeholder) when a cluster cannot resolve a thumbnail â€” otherwise this is invisible again.

**(c) Android uses square tiles where web uses circular portraits.** [LibraryFeatureScreens.kt:163-171](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L163-L171) uses `RoundedCornerShape(12.dp)`; web's People list uses `variant="avatar"`.
- [x] `CircleShape` for person **and** pet cluster tiles on Android, threaded as a `circular` flag through `GridScaffold` so Trips and Memories stay rectangular. — `5c4d776`

**(d) The Albums page doesn't use face centering at all.** [web/src/pages/Albums.tsx:669-690](web/src/pages/Albums.tsx#L669-L690) renders the People row without applying `faceCropStyle` â€” that only happens in `PeopleView`.
- [x] Applied to the Albums-page **People** row via the shared `clusterFaceCropStyle`. It had no face framing at all, so the same person was framed one way on the People page and another in the Albums row. — `5c4d776`
- [ ] **Pets cannot be done here — this plan was wrong.** `PetCluster` carries no `rep_bbox_*` on the server, in `web/src/api/ai.ts`, or in Android's `AiDto.kt`; only `FaceCluster` does. There is nothing to frame until the server resolves and emits a representative pet detection bbox, which is its own server-side task, not a client wire-up. Pet tiles are circular (c) but centre-cropped.

### C2 â€” #39 Cannot rename pets on Android (Low) â€” CONFIRMED

Clean, small, fully scoped. The whole backend path already exists: [ApiService.renamePetCluster:465](android/app/src/main/kotlin/com/simplephotos/data/remote/ApiService.kt#L465) and [AiRepository.renamePetCluster:69](android/app/src/main/kotlin/com/simplephotos/data/repository/AiRepository.kt#L69). Only the UI wiring is missing: `PersonDetailScreen` has the rename dialog ([LibraryFeatureScreens.kt:425-445](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L425-L445)) but `PetDetailViewModel`/`PetDetailScreen` ([line 528-569](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L528-L569)) has no `rename` function and no dialog. The shared dialog at [line 450](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L450) is already documented as "for a person/pet cluster."

**Fix:** — **DONE**, commit `736a927`

- [x] `rename()` on `PetDetailViewModel`, calling `repo.renamePetCluster`. — `736a927`
- [x] Toolbar rename action + shared dialog wired into `PetDetailScreen`. **`RenameClusterDialog` hardcoded the title "Rename person"** despite already being documented as "for a person/pet cluster" — reusing it as-is would have shown a pet owner the word "person". Title is now a parameter. — `736a927`
- [x] Optimistic label on success; **log + surface on failure**. — `736a927`
- [x] The decision is extracted to `ClusterRename.kt` and shared by BOTH detail ViewModels. Neither ViewModel is unit-testable directly (both take `PhotoRepository`, a concrete class whose graph would have to be stood up in a JVM test), so the part that can be wrong takes a suspend lambda instead of a repository. Same move `RenditionChoice.kt` made for #49. — `736a927`
- [x] Test: `ClusterRenameTest` (6). Verified RED against a naive implementation (no blank guard, raw `e.message`): 3 of 6 fail. The other 3 pass in both states because they assert preserved behaviour — the honest result, not tests tuned to go red. 179 Android tests green. — `736a927`

**Two defects in the PERSON path, fixed by sharing the helper:**
1. `catch (e: Exception) { error = e.message }` assigned a possibly-null message
   straight to the error banner, so any exception without one surfaced as an
   error state containing **nothing**.
2. The failure path had **no log at all**, which AGENTS.md forbids outright.
   Logging lives in the ViewModel adapter so `ClusterRename.kt` stays
   Android-free and JVM-testable.

**Do NOT "optimise" this with a `trimmed == current` skip.** It reads like a
free saving and it is wrong for pets: `PetDetailViewModel.label` falls back to
the **species** when the cluster has no stored label, so a pet displayed as
"Dog" with a NULL label typed as "Dog" is a genuine rename that would be
silently dropped. The displayed label is not the stored one. A test pins this.

---

## Workstream D â€” Viewer UI (quick wins)

### D1 â€” #44 Info button still shows in the viewer (Low) â€” CONFIRMED

Not a bug so much as an unfinished decision from #30. Both platforms deliberately kept the standalone button, with a comment saying so:
- Web: [ViewerTopBar.tsx:121-125](web/src/components/viewer/ViewerTopBar.tsx#L121-L125) (button) and [:163](web/src/components/viewer/ViewerTopBar.tsx#L163) â€” *"Info lives here too (#30) â€” the standalone button stays up top."*
- Android: [PhotoViewerScreen.kt:1071-1075](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/PhotoViewerScreen.kt#L1071-L1075) and [:1125](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/PhotoViewerScreen.kt#L1125) â€” same comment.

**Fix:** — **DONE**, commit `0fb7bdb`

- [x] Removed the standalone Info button from both top bars; overflow entry kept. — `0fb7bdb`
- [x] Both comments now state the new intent instead of the opposite. — `0fb7bdb`
- [x] **Checked `SecurePhotoViewer.kt` — the duplication never existed there.** Info is in the overflow menu only ([SecurePhotoViewer.kt:283-289](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt#L283-L289)). No edit needed.
- [x] **No E2E or unit selector drives the top-bar Info button** — grepped the title, the aria-label and the contentDescription across the tree. Nothing to re-point.

No test ships with this one: it deletes a button whose behaviour is duplicated
by a menu entry that was already there, and this repo has no jsdom or rendered-
composable harness that could assert "the button is absent" without a device.
Saying so beats inventing a test that asserts a string is missing from a file.

### D2 â€” #50 Video controls collide with the phone navigation bar (Medium) â€” CONFIRMED

- Android: [VideoPlayer.kt:484-485](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/VideoPlayer.kt#L484-L485) uses a hardcoded `.padding(top = 32.dp, bottom = 8.dp)` with **no window-inset handling**. An 8dp bottom margin puts play/pause/mute directly under a 48dp 3-button nav bar. The correct pattern is already used elsewhere in this codebase â€” [SecurePhotoViewer.kt:362](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt#L362), [:649](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt#L649) and [ViewerEditPanel.kt:87](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/ViewerEditPanel.kt#L87) all apply `navigationBarsPadding()`. The main video player was simply missed.
- Web: [VideoControls.tsx:126](web/src/components/viewer/VideoControls.tsx#L126) uses `pb-3` with no safe-area inset. `grep` finds **zero** uses of `env(safe-area-inset-*)` anywhere in `web/src` â€” so the installed PWA overlaps the home indicator too.

> **DONE — `90aa0cd`. The web plan above was WRONG and is corrected below.**
>
> **The prescribed web fix would have shipped a no-op.** `padding-bottom:
> calc(0.75rem + env(safe-area-inset-bottom))` was justified by "grep finds zero
> uses of `env(safe-area-inset-*)`" — but `index.html`'s viewport meta had no
> `viewport-fit=cover`, so `viewport-fit` defaulted to `auto`, the viewport was
> constrained to the safe area, and **`env()` resolved to 0 in every browser**.
> The padding would have computed to exactly the value it replaced. Same shape
> as B1/#45's trap 2: a value passed in that silently does nothing. **The
> absence of `env()` in the tree was not evidence the fix was missing — the
> precondition was.**
>
> **The justification was wrong for the same reason.** Under `viewport-fit=auto`
> the browser insets the viewport for you, so the installed PWA did **not**
> overlap the home indicator.
>
> **What IS a live bug is the TOP edge.**
> `apple-mobile-web-app-status-bar-style=black-translucent` ([index.html:10](web/index.html#L10))
> already pushed content under the iOS status bar and no top bar ever
> compensated. So the app was inconsistent: top edge opted into edge-to-edge and
> unpadded, bottom edge not opted in at all.
>
> Resolution: opt in deliberately with `viewport-fit=cover`, then inset every
> edge-anchored surface — **bottom AND top, because opting in without the top
> padding trades one overlap for another on every non-iOS platform.**
>
> Low-risk by construction: every rule is `base + env(..., 0px)`, which computes
> to exactly the base value on any device with no inset. Provably a no-op except
> where the bug is. The `0px` fallback is load-bearing — without it a browser
> that does not know the keyword invalidates the whole `calc()` and drops the
> padding entirely, which is worse than the overlap.

- [x] Android: `.navigationBarsPadding()` on the video control bar. Placed **after** `.background` deliberately — the gradient is sized before the inset, so it keeps painting behind the nav bar instead of cutting off in a hard line above it. Only the controls move. — `90aa0cd`
- [x] Web: `viewport-fit=cover` **first** (without it the rest is inert), then `safe-*` utilities defined once in `index.css`. — `90aa0cd`
- [x] Audited every edge-anchored web surface, not just the reported one: VideoControls, Slideshow, ViewerEditPanel, PhotoInfoPanel, TagPanel, BannerHost, the Gallery FAB, ViewerTopBar, ServerOfflineBanner. Horizontal insets on the viewer surfaces too — landscape is the orientation video is actually watched in. — `90aa0cd`
- [x] Utilities live once in `index.css`, **not** as repeated `pb-[calc(...)]` at nine call sites. Two copies of one formula is exactly how `computeFaceCropStyle` and `FaceCrop` drifted apart — see C1(a), still open below. — `90aa0cd`
- [x] Test: `web/src/safeArea.test.ts` (4). There is no honest unit test for "the controls clear the nav bar" — that is CSS, there is no jsdom, and `env()` is not resolvable off-device. So these pin what would silently void the fix: `viewport-fit=cover` present; every `safe-*` class used in the tree actually defined (a CSS typo is not a type, build or runtime error); every `env()` carrying a fallback. All 4 verified RED by stashing the change. 266 web green (was 262). — `90aa0cd`
- [ ] **Verify on a device with 3-button nav AND gesture nav** — different inset heights (`.device-test/dev.ps1`, S21+ harness). Still the only way to confirm the actual clearance.
- [ ] **Verify the iOS/PWA half on a notched device.** Every `env()` value is 0 on the dev machine, so the desktop-identical rendering proves the no-op property and nothing else.

**Two traps for whoever touches web safe-area next:**
1. **`import.meta.glob(..., { query: "?raw" })` returns an EMPTY STRING for
   `.css`** — Vite's CSS plugin outranks `?raw`. Every CSS assertion in the test
   passed *vacuously* until this was caught, which is precisely the failure the
   file exists to detect. Hence `node:fs` and the `@types/node` devDependency
   (dev-only, never bundled).
2. **`import.meta.glob`'s options must be an inline object literal.** Vite
   analyses it statically; hoisting them to a shared `const` fails the build.

---

## Workstream E â€” Albums

### E1 â€” #43 Move selected items between secure albums (High) â€” CONFIRMED

**Reported:** no option in a secure album to add selected items to another secure album.

**Root cause: the existing feature only works in one direction.** #31 shipped a **pull** picker â€” from inside secure album A, browse *other* albums' items and bring them in ([web/src/gallery/secureMovePicker.ts](web/src/gallery/secureMovePicker.ts), wired at [SecureGallery.tsx:360-365](web/src/pages/SecureGallery.tsx#L360-L365)). There is no **push**: select items in the album you are looking at and send them elsewhere.

Good news â€” the hard parts are done. A photo lives in at most one secure album (server-enforced), so this is a **move**, and `resolveSecureMoves` + the server endpoint already implement exactly that operation. This is predominantly UI.

**DONE â€” `d1f439a`. web 284 green (was 275), android 196 green, e2e move suite green.
Two plan items were wrong and one whole hazard the plan never mentioned turned
out to be the interesting part.**

> **The server op is direction-agnostic, so "push" is entirely client framing.**
> `move_gallery_item` verifies ownership of BOTH galleries and reassigns
> membership scoped to `(item_id, source gallery_id)` â€” it does not care whether
> the caller is pulling in or pushing out. Confirmed by reading it before writing
> a line: there is **no server change here**, and the E2E pins that the exact same
> endpoint the #31 pull picker calls also satisfies #43.
>
> **`SelectablePhotoGrid` is NOT reusable â€” only `usePhotoSelection` is.** The
> plan said "reuse `usePhotoSelection` / `SelectablePhotoGrid`". The grid is
> welded to `CachedPhoto` + `AlbumTile` + trash/add-to-album and renders regular
> photos from IDB; secure items are `SecureGalleryItem`, rendered from encrypted
> blobs through `SecureGalleryItem`/`ThumbnailTile` with a gallery token. Reusing
> it was never possible. The *hook* is, and it needed one additive method
> (`enterEmpty`) so a toolbar "Select" button can enter selection mode with an
> empty set â€” the hook only had "enter seeded with one id" (long-press) before.
>
> **Android already had multi-select in the secure album detail â€” for delete.**
> The plan's "Android parity in SecureGallery" implied building selection from
> scratch. `GalleryDetailView` already has `selectionMode`/`selectedItemIds`
> (long-press to enter, tap to toggle, select-all, âœ“ overlay) driving the Remove
> action. Push was a "Move" entry added to that existing bar + a target-album
> dialog â€” not a new selection system.
>
> **Bursts were the hazard the plan didn't name.** The grid collapses a burst to
> one tile, so a naive move ships only the cover and strands the rest in the
> source album â€” the same split `removeItems`/secure-add already guard against.
> The move must expand a selected representative to every frame sharing its
> `burst_id`. That, plus routing each item from its **own** `gallery_id` (so a
> synthetic smart view works) and dropping items already in the target (a no-op
> move), is the entire content of the pure planners â€” `secureMovePicker.ts`
> (`expandSecureSelection`/`planSecureMovesToTarget`/`secureMoveTargets`) and
> Android's `SecureMovePlan.kt`. Both were RED-verified by neutering the burst
> expansion and the same-target drop: web 4/15 fail, android 3/8 fail, exactly
> the new-behaviour tests, the rest green in both states.

- [x] Web: selection via `usePhotoSelection` + a "Select" toolbar button, a selection bar with "Move to album", and a target-album picker modal (`secureMoveTargets`). â€” `d1f439a`
- [x] Reuse `usePhotoSelection` (added `enterEmpty`). **`SelectablePhotoGrid` was not reusable** â€” see correction above. â€” `d1f439a`
- [x] Smart-album case: each item routes from its own `gallery_id` via `planSecureMovesToTarget`; a synthetic open id offers every real album as a target. Follows the removal-path precedent. â€” `d1f439a`
- [x] Resilient batch: each `moveItem` is isolated (one failure never aborts the rest), success/failure surfaced; same shape as secure-add and the pull picker. â€” `d1f439a`
- [x] Android parity: "Move" added to the **existing** selection bar + target dialog; pure planning in `SecureMovePlan.kt` (8 JVM tests). â€” `d1f439a`
- [x] E2E: `tests/test_06 TestSecureGalleryMove` (4) â€” membership reassigned Aâ†’B, move scoped to the named source (wrong source is a no-op 404), IDOR-guarded target (another user's album rejected, item untouched), original stays hidden after a move. â€” `d1f439a`
- [ ] **Device-verify on the S21+ harness.** The move arithmetic + persistence are tested; the actual selection→dialog→move gesture on a real device has only run in a compiler. Same standing gap as every other Android UI item in this batch.

### E2 â€” #52 Sort button on the album header (Feature) â€” CONFIRMED

No user-facing sort exists anywhere. Ordering is hardcoded `takenAt` desc, with a single `sortBy: "addedAt"` special case for Recently Added ([smartAlbums.ts:17](web/src/gallery/smartAlbums.ts#L17), applied at [useAlbumPhotos.ts:76-77](web/src/hooks/useAlbumPhotos.ts#L76-L77)).

- [ ] Sort control on the right of the album-detail header: Date and Name, each ascending/descending.
- [ ] Implement in `useAlbumPhotos` so every album type inherits it; do not special-case per view.
- [ ] Persist the choice per album (localStorage keyed by album id), consistent with the existing `useScrollMemory` precedent.
- [ ] Interaction with burst collapse: sort **after** collapsing, and sort by the representative frame â€” otherwise burst tiles jump around.
- [ ] Name sort must be locale-aware (`Intl.Collator`, `numeric: true`) so `IMG_2` precedes `IMG_10`.
- [ ] Android parity.
- [ ] Unit-test the comparators, including ties and missing `takenAt`.

---

## Workstream F â€” Android windowing

### F1 â€” #41 Cap dual-window at two instances (Low) â€” CONFIRMED

[NewWindow.kt:108-125](android/app/src/main/kotlin/com/simplephotos/ui/navigation/NewWindow.kt#L108-L125) `openInNewWindow` has **no instance cap**. `FLAG_ACTIVITY_MULTIPLE_TASK` ([line 43-45](android/app/src/main/kotlin/com/simplephotos/ui/navigation/NewWindow.kt#L43-L45)) spawns a fresh task on every invocation, and "New Window" is offered from every `AppHeader` ([AppHeader.kt:382](android/app/src/main/kotlin/com/simplephotos/ui/components/AppHeader.kt#L382)) â€” so a user can open unbounded windows, all sharing one process, Room DB and Coil cache. Given A3, that is also a memory-pressure contributor.

**Fix:** — **DONE**, commit `d663da7`

> **Both options this file proposed were rejected, and the reason is the item's
> own warning.** The design question is not "how do I count windows" but "how do
> I count them such that the count cannot get stuck high", because a stuck count
> disables the feature permanently.
>
> - **`ActivityManager.appTasks` enumerates TASKS, not live activities.** A task
>   whose activity the system reclaimed still appears there, so a window the
>   user can no longer see would hold the cap engaged forever. Self-healing in
>   appearance only.
> - **Hand-rolled `onCreate`/`onDestroy` overrides** pair correctly today but are
>   one early-return from leaking, with nothing to catch it.
>
> Used instead: **`Application.ActivityLifecycleCallbacks`**, registered in
> `SimplePhotosApplication.onCreate` before any activity can exist. The framework
> pairs created/destroyed for every instance within a process lifetime, and the
> one case where destroy is never delivered — the process being killed — takes
> the counter with it, so the count resets to 0 alongside the activities it was
> counting. **The leak and its cure arrive together**, which is what makes the
> "survives process death" requirement answerable rather than merely tested.

- [x] Live-instance tracking via `ActivityLifecycleCallbacks` in `AppWindows.kt`, not a hand-rolled counter and not `appTasks`. `WindowCounter` holds no Activity reference so the logic is JVM-testable; `AppWindows` is only the glue. — `d663da7`
- [x] `openInNewWindow` refuses at the cap and returns false; the toast now says "Already using 2 windows" instead of "Couldn't open a second window", which implied a malfunction. — `d663da7`
- [x] Menu entry is `enabled = false` at the cap and reads "New Window (limit reached)". Enforcement stays in `openInNewWindow` — the menu is an affordance, and the launcher is reachable from more than one place. — `d663da7`
- [x] **Configuration changes cannot inflate the count**: MainActivity's extensive `android:configChanges` means the OS does not destroy and recreate it — the #17 biometric-lock fix paying off a second time. **Process death cannot leak it** per the argument above. — `d663da7`
- [x] Test: `WindowCounterTest` (6). Verified RED against two plausible-but-wrong implementations — an `!=` cap test and an unclamped `closed()` — exactly 2 of 6 fail, one per defect. 179 Android tests green (was 173). — `d663da7`
- [ ] **Device-verify on the S21+ harness.** The arithmetic is unit-tested; that two real windows are counted as two, and that killing one re-enables the menu entry, has only ever run in a compiler.

**Both guarded cases are asymmetric in the same direction, which is why they are
tested at all:** `<` rather than `!=` so a count that overshoots still refuses
instead of waving every window through forever, and a zero clamp so an
unbalanced close cannot go negative — each step below zero buys one extra window
above the cap.

---

## Suggested execution order

Dependencies are real here â€” A1 gates the honest measurement of C1(b), and B1 gates B2/B3.

1. **A1** (#42) â€” counts + the pagination off-by-one. Everything downstream is measured against correct numbers.
2. **A3** (#51) â€” the crash. Highest user pain, self-contained once you accept virtualization.
3. **A2** (#38) â€” delta sync + server cache. Builds on A1's schema work.
4. ~~**B1** (#45) â€” failure logging.~~ **DONE** `298fd99`.
5. ~~**D1** (#44), **D2** (#50), **C2** (#39), **F1** (#41) â€” quick wins.~~ **ALL DONE** 2026-07-21 — `0fb7bdb`, `90aa0cd`, `736a927`, `d663da7`. One commit each as planned. Two of the four had a wrong plan in this file (D2's web half was a no-op; F1's both suggested mechanisms leak) — **read the corrections in those sections before trusting any other plan here.**
6. ~~**C1** (#48) — face centering.~~ **(a), (c), (d-People) DONE** 2026-07-21 — `5c4d776`. **(b) still open** and still gated on A1's deploy. Read the (a) corrections before trusting any other plan in this file: the formula was right, the *mechanism* was not, and the existing test suite asserted the bug.
7. ~~**B3** (#46) — codec probing.~~ **Code landed `65389a7`; backfill + corrupt-file honesty still open.**
8. ~~**B2** (#40).~~ **BOTH HALVES DONE** — 3-strike cap `8bfe66a`, ETA rework `08bc838`. The plan was wrong 4× on the cap (the worst loop was a file that *converts fine*) and 3× on the ETA (duration weighting is a trap; the plan had no answer for the unsampled category; the obvious EWMA form is biased). Read both correction blocks before trusting anything else in this file.
   > **B2 was resequenced ahead of B3's remainder deliberately.** B3's own
   > `VIDEO0063.mp4` item says it needs "the terminal unplayable state from
   > B2/#40", and B3's backfill queues 38 files that, without a cap, re-attempt
   > forever. Shipping the backfill first would have shipped a loop.
9. ~~**E1** (#43)~~ **DONE** `d1f439a` â€” push move, client + E2E, no server change (the move endpoint is direction-agnostic). **E2** (#52) â€” album sort, still open.
10. **B4** (#49) â€” the resolution ladder. Largest scope; do not let it block anything above.

## Cross-cutting risks

- **A1 + A2 touch the same schema.** Sequence them or you will write two migrations that fight. Prefer one migration adding both `change_seq` and the failure-count column if they land together.
- **Burst collapse is the recurring trap.** Every count/sort/ETA change must state explicitly whether it operates on raw rows or collapsed tiles. Most of the historical count bugs in this repo are exactly this confusion.
- **The `src-` album id formula lives in three codebases** (memory `takeout-album-phases-2-3`). If E2's sort touches album identity, do not tidy one copy alone.
- **Device verification is outstanding from the previous batch** (#29â€“#37, per memory). Fold an S21+ pass into this batch rather than accruing more unverified Android work.
- **Do not regress the idle disk-thrash fix** (memory `idle-disk-thrash-investigation`). A2 rewrites the sync loop â€” the steady-state "zero downloads when nothing changed" property is a hard requirement, not a nice-to-have.
