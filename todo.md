# TODO — Open GitHub Issues (#38–#52)

Pulled from `Wulfic/simple-photos`, 14 open issues, all filed 2026-07-20.
Investigated against the tree at `b3e48f0` (branch `dev`).

**Legend for confidence:**
- **CONFIRMED** — I read the code, the defect is visible in the source. No live repro needed to start.
- **HYPOTHESIS** — mechanism identified, but the specific number/symptom the user reported needs a live box to attribute.

**Ground rules for this batch (non-negotiable):**
- One commit per issue, conventional commit, referencing `(#NN)`.
- Every fix ships with a test that FAILS before the fix. No exceptions, especially the count/pagination work — those bugs survived precisely because nothing asserted them.
- Never commit red. `cargo test --bin simple-photos-server`, `npm test` in `web/`, and the `tests/` pytest E2E suite must be green.
- Known-red baseline before you start (see memory `e2e-preexisting-failures-2026-07-15`): `test_06` secure 401s (8), `test_20` dates (4), `test_58` Windows harness bug. Do not blame your diff for those; do not let them grow.

---

## Workstream A — Counts, performance, and the scroll crash

These three issues (#42, #38, #51) are **one problem wearing three hats**: there is no single authoritative definition of "how many items are in this library," and the web client rebuilds the entire library on every sync while rendering every tile it has ever seen. Fix them together, in this order. Do not start anywhere else in this file until A is done.

### A1 — #42 Incorrect photo counts (High) — CONFIRMED

**Reported:** Android Photos album shows 10,211; web shows 7,822. "And many other instances."

**Root cause: there are three different, silently diverging definitions of the count.**

1. **Server** — [server/src/gallery/summary.rs:109-127](server/src/gallery/summary.rs#L109-L127) counts every eligible `photos` row, secure-excluded, **including rows with `encrypted_blob_id IS NULL`**, and reports both raw `total` and `collapsed_total`.
2. **Web** — [web/src/gallery/hooks/usePhotoSync.ts:205](web/src/gallery/hooks/usePhotoSync.ts#L205) does `if (!photo.encrypted_blob_id) continue;`, so every not-yet-encrypted row is **dropped from IndexedDB entirely**. Counts then come from that truncated mirror via [web/src/gallery/smartAlbums.ts:33-36](web/src/gallery/smartAlbums.ts#L33-L36).
3. **Android** — [PhotoRepository.kt:961](android/app/src/main/kotlin/com/simplephotos/data/repository/PhotoRepository.kt#L961) applies the *same* `?: continue` skip, **but** the count is taken from [AlbumViewModel.kt:250-269](android/app/src/main/kotlin/com/simplephotos/ui/screens/album/AlbumViewModel.kt#L250-L269) via `photoRepository.getAllPhotos()`, which is [`SELECT * FROM photos`](android/app/src/main/kotlin/com/simplephotos/data/local/dao/PhotoDao.kt#L16) — **the whole Room table, including device-captured local rows (`localPath` set, `syncStatus` PENDING/FAILED) that were never on the server and that the web mirror cannot possibly contain.**

So Android counts server-synced ∪ device-local; web counts server-synced-and-encrypted only. ~~**HYPOTHESIS:** the 2,389 delta is predominantly the device camera roll pending upload.~~

> **✅ MEASURED 2026-07-20 on live CT132 — the camera-roll hypothesis was WRONG.**
> The delta is the *server-side encryption backlog*, and it accounts for the gap
> exactly, with zero remainder:
> ```
> summary.total              14874
>   − NULL encrypted_blob_id  2494   ← web's `continue` drops these
>   − lost to the cursor bug     29   ← 29 page boundaries, 1 row each
>   = web-visible             12351   ← exactly what a full walk returns
> ```
> `encrypted_thumb_blob_id` is NULL on the same 2,494 rows, so they have no
> displayable ciphertext at all. Do not re-raise the camera-roll theory without
> device evidence. Repro: scratchpad `probe.ps1` (auth → summary → full walk).

**Second, independent defect — off-by-one in keyset pagination. CONFIRMED.**
[server/src/gallery/sync.rs:99-131](server/src/gallery/sync.rs#L99-L131) fetches `LIMIT limit + 1`, then builds `next_cursor` from `photos.last()` — which is the **peeked (limit+1)-th row** — and only afterwards truncates the response with `.take(limit)`. The next page's predicate is strict (`< ts OR (= ts AND id > id)`), so the peeked row is **never returned by any page**. One photo is silently lost per page boundary, on both clients, forever. At 500/page over a 10k library that is ~20 photos vanishing from every client. Nothing in `tests/` asserts round-trip completeness — [tests/helpers.py:1379-1390](tests/helpers.py#L1379-L1390) paginates the same way and only checks the data it *did* receive.

**Fix:**
- [x] `server/src/gallery/sync.rs` — build `next_cursor` from the **last returned** row, not the peeked one. Truncate first, then derive the cursor. — `568c282`
- [x] Rust unit test: paginate fully, assert the returned id set **equals** the seeded set. Verified RED first (`rows were never returned by ANY page: ["p03"]`). — `568c282`
- [x] **Same defect found in 3 more paginators** (`blobs`, `trash`, `photos`) — all fixed in `568c282`. `sync.rs` was the mildest: it has an `id` tiebreak, the others use timestamp-only cursors.
- [x] Decide and document the ONE canonical count definition. **DECIDED (Tyler, 2026-07-20): server-authoritative, count everything including unencrypted, grids unchanged.** Consequence accepted: the badge intentionally exceeds the tile count by the pending-encryption backlog. Corollary — no client may count its own local mirror. — `29e4d1f`
- [x] Extend `PhotoSummary` with per-smart-album collapsed counts — shipped as `smart_photos`/`smart_gifs`/`smart_videos`/`smart_audio`/`smart_favorites`/`smart_recent`. Note `smart_photos` counts photo **+ gif** because both clients define "Photos" that way; the raw `photos` column does not. — `29e4d1f`
- [x] Web: counts extracted to `gallery/smartAlbumCounts.ts` and precedence inverted from `local ?? summary` to `summary ?? local`. The real bug was not that web *dropped* rows — it was that the truncated mirror **outranked** the authoritative summary. — `29e4d1f`
- [x] Android: same inversion in `AlbumViewModel.loadSmartAlbumCounts`; every fallback category now collapses bursts (`total`/`gifs`/`videos`/`audio` were raw while `favorites`/`photos`/`recent` were collapsed, in one function). — `29e4d1f`
- [x] E2E (`tests/`): upload N photos, assert `/photos/summary`, a full `encrypted-sync` pagination, and the album badge all agree. — `21eae82` (`tests/test_89_count_agreement.py`, 11 tests). Verified it bites: with the pre-`568c282` cursor temporarily restored, 7/11 fail and `limit=1` returns **6 of 12 rows**. Small page limits are the whole trick — at limit=500 a 12-row library has no page boundary and the bug is invisible.
- [ ] **Android has no unit test source set** (`app/src/test/` does not exist). Its half of this fix is verified only by compilation + parity with the tested server logic. Decide whether to stand one up.
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

### A2 — #38 Photo libraries are slow (High) — CONFIRMED

**Reported:** slow on both web and Android; asks for a unified server-side cache.

**Root cause — the web sync is O(library) per pass with a sequential IndexedDB round-trip per photo.** [web/src/gallery/hooks/usePhotoSync.ts:155-405](web/src/gallery/hooks/usePhotoSync.ts#L155-L405):
- Phase 1 pages the **entire** library into memory (`limit: 500`, loop to exhaustion) — every pass, no delta/`since` parameter, no ETag.
- Phase 2 then enumerates **all four blob types** in full (`fetchAllPages` ×4).
- Phase 3 iterates every photo in a `for` loop with `await` inside: `await ensureThumbCached(...)` performs an IndexedDB read **per photo** ([line 51](web/src/gallery/hooks/usePhotoSync.ts#L51)), plus `await db.photos.update(...)` per changed row. For 10k photos that is 10k+ serialized IDB transactions on the main thread.
- Phase 4 adds another per-blob `await db.photos.get()`.

The 5-minute interval and re-entrancy guard (added in the idle-thrash fix) stop it *stacking*, but each individual pass is still a full-library rebuild. The server has no delta endpoint to make it cheap.

**The issue's own suggestion is the right fix.** Build a real server-side cache + delta protocol.

> **Client batching landed first — `f31b27b`.** Sequenced ahead of the protocol
> deliberately: one file, no schema, no Android, no wire change, and it removes
> the dominant client-side cost. Phase 3 is now staged per 200-row chunk (plan →
> one indexed key-scan + only-missing downloads → one `rw` transaction with a
> `bulkPut` per table) in `web/src/gallery/hooks/syncReconcile.ts`. Two further
> O(library) reads went with it: the second full `toArray()` after stale pruning,
> and the per-blob `db.photos.get()` in Phase 4.
>
> **Hard constraint discovered — do not undo it:** a blob download can NOT happen
> inside a Dexie `rw` transaction (awaiting a non-Dexie promise commits it early,
> next write throws `TransactionInactiveError`). That is what forces the staging;
> it is not stylistic. Presence is checked with `primaryKeys()`, not `bulkGet`,
> so testing for existence does not structured-clone megabytes of thumbnail bytes.
>
> **Bug found while testing** (fixed in the same commit): the blob-id binding path
> set `existing.serverPhotoId = photo.id` in memory, then asked
> `existing.serverPhotoId !== photo.id` to decide whether to persist it — false by
> construction. Locally-uploaded rows could keep a null `serverPhotoId`
> indefinitely, breaking favourite toggles, face-cluster lookup and duplicate.
>
> Tests assert operation **counts**, not correctness — the old code produced the
> right mirror, just slowly, so no correctness test could ever catch a regression.

Remaining (the protocol itself):

> **Server half landed — `31fc322`.** Clients untouched; they still full-walk.
> The plan below assumed a `change_seq` column on `photos`. That is NOT what
> shipped, and the difference is the whole safety argument.
>
> **`photo_change_log` is a HINT, not a source of truth.** Its triggers say only
> "photo X may have changed" — never that X was deleted or that X is eligible.
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
>   the first at a page boundary — **exactly #42's off-by-one, reintroduced
>   somewhere new.** Cursor is composite `"<seq>|<photo_id>"`. Verified RED:
>   bare-seq at `limit=1` returns `m1`, loses `m2`+`m3`.
> - A `UNIQUE` index on `seq` — the obvious "make the cursor simple" move —
>   would have made every multi-photo secure-add **fail outright**.
>
> **Confirmed by experiment, not assumed:** SQLite fires `AFTER DELETE` triggers
> for rows removed by `ON DELETE CASCADE`. Deleting a secure gallery cascades to
> its items, and that must un-hide its photos. The entire tombstone design rests
> on this, so a test pins it.
>
> Also verified RED by disabling the EGI insert trigger: three tests fail, and
> `applying_the_delta_matches_a_fresh_full_walk` fails by **retaining** the
> secured photo the full walk dropped — the ghost-row regression this file
> warned about, now under test rather than under discussion.

- [x] Monotonic change sequence maintained by trigger — shipped as `photo_change_log` (migration `033`), a keyed log rather than a column on `photos`. No FK on `photo_id`: a tombstone must outlive the row it describes. — `31fc322`
- [x] Tombstones covering all **9 delete sites across 7 files** + the eligibility subquery. Solved by the hint-not-truth design above rather than by exhaustively enumerating paths. — `31fc322`
- [x] Backstop. `photos_summary` now returns `head_seq`, deliberately **not** served from the TTL cache — a stale head would recreate exactly the busywork this removes. A client holding the current head skips `encrypted-sync` altogether. — `31fc322`
- [x] `GET /api/photos/encrypted-sync?since=<seq>` returning changed rows + `deleted[]` + `head_seq`. Migration backfills every existing photo, so `since=0` degenerates into a full sync and **cold-start needs no special branch**. — `31fc322`
- [x] Unified snapshot: counts **and** head sequence in one round trip. — `31fc322`
- [x] The eligibility predicate had been copy-pasted into 3 queries (delta adds 2 more) — now one const in `gallery/eligibility.rs`. A delta whose eligibility differs from the full walk's by one arm hands clients rows the grid will never show. — `31fc322`
- [x] Web: batch Phase 3 writes with `bulkPut` — already done in `f31b27b`.
> **Web `?since=` landed — `6a1b711`.** Android untouched.
>
> The pass moved out of `usePhotoSync` into `syncPass.ts` (skip / delta /
> full) with the cursor in `syncCursor.ts`. `usePhotoSync` is now only the
> React shell. **The full walk is kept as the recovery path, not as legacy** —
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
>   refuses a cursor over an empty mirror — the partial wipe co-location can't
>   catch. `clearAllUserData` wipes it (Dexie v11 `syncState`).
> - **A pre-#38 server ignores `since` and answers with a FULL walk**, whose
>   `photos` are indistinguishable from a delta's. Reading it as a delta prunes
>   nothing while believing it pruned, then persists a cursor making that
>   permanent. The handshake is `deleted`: present-possibly-empty on a delta,
>   absent on a full walk — which is exactly why the server author made it
>   "empty rather than absent". Absent now forces the full path.
> - **Persist the FIRST page's head, not the last** (the server doc says so;
>   it is easy to get backwards). A change committed mid-walk lands above the
>   first page's head — keep the first and it is re-delivered, keep the last
>   and it is lost.
> - **A tombstone names a photo id, which may be the row's primary key OR its
>   `serverPhotoId`** (rows bound to a local upload's blob id). Resolving only
>   by primary key strands locally-uploaded rows forever.
>
> **Deliberate, documented narrowing — do not "fix" it by restoring the walks:**
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

- [x] **Web: adopt `?since=`.** Persist the last-seen sequence; poll `summary.head_seq` first and skip sync entirely when unchanged. — `6a1b711`
- [x] **Perf gate** on the client where it is observable: a 10k-row fixture at an unchanged head calls `encrypted-sync` **0×**, `blobs.list` **0×**, `blobs.download` **0×**, `bulkPut`/`bulkDelete` **0×**, and `photos.toArray` **0×**. Verified RED by disabling *only* the skip fast-path — that test fails and nothing else does. 13 tests, 213 green (was 200). — `6a1b711`
- [ ] Android: same `?since=` adoption in `syncFromServerEncrypted`. **Read `web/src/gallery/hooks/syncPass.ts` first** — all four hazards above apply verbatim, and three of them (cursor lifetime, the `deleted` handshake, first-page head) are protocol-level, not web-specific. Android's equivalent of the co-location rule is: the cursor belongs in the Room DB that holds the mirror, cleared by whatever clears `photos` — NOT in `SharedPreferences`, which survives a database wipe.
- [ ] **Tombstone retention.** Rows for deleted photos accumulate without bound. Pruning needs a policy (e.g. drop after 90d) — a client offline longer than the retention window must be forced through a full reconcile, which is what `head_seq`/`total` are for. Not urgent at current library sizes; do not forget it.
- [ ] **Deploy required.** `033` backfills on first boot against the live 14,874-row library — cheap, but it is the first migration here with a data backfill, so watch it.

---

### A3 — #51 Crashing when scrolling a long list (High) — CONFIRMED

**Reported:** jitter and lag, then the app closes.

**Root cause: `JustifiedGrid` has no virtualization whatsoever.** [web/src/components/gallery/JustifiedGrid.tsx:166-211](web/src/components/gallery/JustifiedGrid.tsx#L166-L211) maps over **every** row and **every** item, mounting a DOM node per photo. At 10k photos that is 10k+ `<div>` + `<img>` nodes live simultaneously.

This interacts lethally with the thumbnail cache. [web/src/gallery/cache/thumbnailCache.ts:99](web/src/gallery/cache/thumbnailCache.ts#L99) caps at 500 entries and calls `URL.revokeObjectURL` on eviction — but the `<img>` elements are **never unmounted**, so scrolling past 500 tiles revokes blob URLs out from under mounted images. They blank, the loader re-fetches, which evicts more, which blanks more. That thrash is the "jittering," and the unbounded node + blob retention is the crash.

Secondary: `_evict()` ([line 83-95](web/src/gallery/cache/thumbnailCache.ts#L83-L95)) sorts the entire map on every insert past capacity — O(n log n) per insert where an LRU should be O(1).

**Fix:**

> **Web half landed — `20690ad`.** Android untouched.
>
> **The cache defect was worse than described above, and in a way that changes
> the symptom.** This file said eviction "revokes blob URLs out from under
> mounted images," implying the tile re-fetches and recovers. It cannot.
> `useThumbnailLoader` minted URLs via `blobUrlManager.acquire()` (ref-counted)
> while `thumbnailCache` revoked them with a raw `URL.revokeObjectURL` — two
> owners, and the one revoking was not the one counting refs. **Nothing in the
> tree ever called `blobUrlManager.release()`.** So after an eviction the
> manager still held a live entry pointing at a dead URL, and the next
> `acquire()` returned it. A cache *miss* is what re-entered the poisoned path,
> so the recovery mechanism was the failure mechanism. Tiles blanked
> **permanently**, for the rest of the session.
>
> Verified RED against the pre-fix code by reconstructing the old
> cache + manager collaboration: the reload returned `blob:mock/1` — the exact
> URL eviction had already revoked — and the "mounted" URL appeared in the
> revoked list. Both assertions fail on `HEAD~1`, both pass now.
>
> `blobUrlManager` is **deleted**, not left in place. It had no callers once the
> cache owned its own URLs, and leaving the second owner around is an invitation
> to reintroduce exactly this bug. It also installed a permanent 60s
> `setInterval` leak-detector in every session.

- [x] Virtualize `JustifiedGrid` — prefix-summed row offsets, rows intersecting the viewport plus a half-viewport overscan band, spacer-padded above/below. No new dependency. — `20690ad`
- [x] Cache capacity is now `max(base, pinned × 3)` — a function of what is actually mounted rather than a magic 500. — `20690ad`
- [x] O(1) LRU via `Map` insertion order (`delete` + re-`set` on access), replacing the full sort on every insert past capacity. — `20690ad`
- [x] Mounted tiles hold a **counted pin** (`pin`/`unpin` from `useThumbnailLoader`) and are skipped by eviction, so a revoke for a live `<img>` is structurally impossible rather than merely unlikely. Pins are counted because one blob can be mounted by several tiles at once. — `20690ad`
- [x] Test: bounded mount count over a 10k-item fixture, asserted on the **pure windowing math** (`gridWindow.ts`) rather than a rendered DOM — this repo has no jsdom/testing-library and the mounted count is decided entirely by that math. Also pins the invariant `padTop + rendered + padBottom == totalHeight` at every scroll offset, which is what keeps document height independent of what is mounted (and `useScrollMemory` therefore correct). 25 tests, 200 green. — `20690ad`
- [ ] **Android: still open.** Confirm the gallery uses `LazyVerticalGrid` with stable keys and that Coil's memory cache is bounded. **HYPOTHESIS** — the issue says "app/server crash," so also check server memory during a long scroll; a thumbnail request storm from an unvirtualized client can be the server-side half.
- [ ] **Deploy/observe.** The virtualization is client-only, so a redeploy of the web bundle is enough — but the fix is only *observable* against a large library. Verify on CT132 with the 14,874-row account, not a test fixture.

**Note for whoever does the Android half:** do not port the pin/unpin design
blindly. Coil owns its own bitmap cache and does not have the two-owner problem
that made the web bug permanent; the web fix is about *ownership*, and Android's
equivalent question is whether the memory cache is bounded at all.

---

## Workstream B — Media conversion and playback

### B1 — #45 Logs don't show failed conversions/imports/encryptions (Low, but do it FIRST) — CONFIRMED

**Do this before B2/B3.** The user currently cannot tell *which file* failed. Every other conversion fix in this workstream is guesswork without it, and it is the cheapest item here.

**Root cause:** the success path audits, the failure path does not. [server/src/ingest.rs:249-259](server/src/ingest.rs#L249-L259) calls `audit::log_background(AuditEvent::MediaConvert, …)` on success. The failure branch at [ingest.rs:437-452](server/src/ingest.rs#L437-L452) emits only `tracing::warn!` — which goes to the process log, **not** the `audit` table the Server Logs tab reads. `AuditEvent` ([server/src/audit.rs](server/src/audit.rs)) has no failure variants for convert/import/encrypt at all.

**Fix:**
- [ ] Add `AuditEvent::MediaConvertFailure`, `ImportFailure`, `EncryptionFailure` (+ `as_str()` arms + the existing `as_str` unit test).
- [ ] Emit `MediaConvertFailure` from `ingest.rs:437` with `filename`, `source_path`, `category`, `error`, `elapsed_ms`. Same treatment for the register failure at [ingest.rs:391](server/src/ingest.rs#L391) and the thumbnail failure at [ingest.rs:432](server/src/ingest.rs#L432).
- [ ] Audit the upload-path failure at [server/src/photos/upload.rs:280](server/src/photos/upload.rs#L280).
- [ ] Web: surface a "Failures" filter in the Server Logs tab (`web/src/components/diagnostics/ServerLogsTab.tsx`).
- [ ] Test: force a conversion failure (corrupt fixture), assert a row lands in `audit` **and** is reachable through the logs endpoint.

### B2 — #40 Conversion ETA is wrong + add a 3-failure cap (High) — CONFIRMED

**Root cause (ETA):** [server/src/status.rs:70-83](server/src/status.rs#L70-L83) `progress_math` is a naive cumulative mean — `per_item = elapsed / done`, `eta = remaining * per_item`. It treats every queue item as equal cost. The queue deliberately mixes categories and sorts images first ([`conversion_priority`, conversion.rs:584](server/src/conversion.rs#L584) orders videos last), so the estimator spends the whole image phase learning a per-item cost that is orders of magnitude too small, then hits the video tail and the ETA explodes. It is also cumulative (early samples bias it forever) and the denominator can move mid-batch.

**Fix (ETA):**
- [ ] Weight by work, not item count. Use `size_bytes` (already on `ConvertCandidate`) — and for video, duration where known — as the denominator. Track completed *weight* vs total *weight*.
- [ ] Estimate per-category throughput separately (image / video / audio). A mixed queue's ETA is then the sum of per-category remainders.
- [ ] Use a sliding-window / EWMA rate rather than the batch-lifetime mean so it adapts.
- [ ] Keep `progress_math` pure and unit-tested; add cases for a mixed image-then-video queue asserting the ETA does not swing by >2× across the category boundary.

**Root cause (repeat failures):** nothing anywhere persists a per-file failure count. On failure, `process_candidate` registers the ORIGINAL to avoid data loss ([ingest.rs:437-455](server/src/ingest.rs#L437-L455)), but several paths `return false` **without registering anything** — e.g. the register error at [ingest.rs:391-398](server/src/ingest.rs#L391-L398). A file that leaves no row is re-walked, re-converted and re-failed on every single autoscan pass, forever.

**Fix (3-strike cap):** extend the existing skip cache rather than inventing a new mechanism. `scan_skipped_paths` ([server/migrations/031_scan_skipped_paths.sql](server/migrations/031_scan_skipped_paths.sql)) already keys on `(user_id, rel_path)`, already stores `size_bytes` + `mtime`, and **already invalidates when either changes** — which is exactly the semantics you want ("if the file is replaced, try again").
- [ ] Migration `034_conversion_failure_count.sql`: add `attempt_count INTEGER NOT NULL DEFAULT 0` and allow `reason = 'conversion_failed'`.
- [ ] On conversion failure, upsert the row with `attempt_count = attempt_count + 1`.
- [ ] The autoscan walk skips a candidate whose `attempt_count >= 3`. Make the threshold a named constant, not a literal.
- [ ] Emit `MediaConvertFailure` (B1) on **every** attempt, and a distinct terminal audit event when a file is retired at 3 so it is visible rather than silently dropped.
- [ ] Admin escape hatch: an endpoint to clear `conversion_failed` rows and force a retry.
- [ ] Test: a fixture that always fails is attempted exactly 3 times across 5 scan passes, then never again — until its mtime changes, after which it is retried.

### B3 — #46 Video.play failure on a specific .mp4 (Medium) — CONFIRMED

**Reported:** `20210520212438-5a45c3d4.mp4` → "unable to play this video format."

**Root cause: format detection is purely by file extension.** [server/src/conversion.rs:487-560](server/src/conversion.rs#L487-L560) `conversion_target` matches on `ext` and `.mp4` is not in the video list, so an mp4 is assumed browser-native and **never transcoded**. But an MP4 *container* routinely carries codecs browsers cannot decode: H.265/HEVC, H.264 High 10 (10-bit), MPEG-4 Part 2 (DivX/Xvid), ProRes. [server/src/photos/web_preview.rs:13-30](server/src/photos/web_preview.rs#L13-L30) `needs_web_preview` has the identical extension-only blind spot.

This is the same class of bug as the GIF misdetection already fixed by magic-byte sniffing (see memory `c1-gif-detection-14`). Apply the same lesson: **probe, don't guess.**

**Fix:**
- [ ] Add an `ffprobe`-backed codec probe (`server/src/transcode/`) returning video codec, profile, pixel format, and resolution.
- [ ] Define the browser-native allowlist explicitly: H.264 (Baseline/Main/High, 8-bit, yuv420p) + AAC/MP3 in MP4. Anything else needs transcoding **regardless of extension**.
- [ ] Route `.mp4`/`.mov`/`.m4v` through the probe at ingest; enqueue for conversion when the codec is not on the allowlist.
- [ ] Backfill: an admin task to probe already-registered mp4s and queue the offenders. Existing libraries are full of these — a fix that only helps new imports does not fix the user's library.
- [ ] Get the actual failing file from the live box and `ffprobe` it before writing the fix. Confirm the codec; do not assume HEVC.
- [ ] Test: a fixture with HEVC-in-MP4 is detected as needing conversion; an H.264-in-MP4 fixture is left alone (no pointless re-encode).

### B4 — #49 Resolution ladder + player quality picker (High, largest item in this file)

**Reported:** >1080p sources should also produce a 1080p rendition; gear icon in the player for resolution choice; default highest on Wi-Fi, lower on cellular; Android needs a cellular data-saver toggle.

**Current state — CONFIRMED there is nothing to build on.** [server/src/transcode/ffmpeg_gpu.rs:15-158](server/src/transcode/ffmpeg_gpu.rs#L15-L158) `build_video_transcode_args` produces exactly **one** output at **source resolution** for every backend. The only scale filters present force even dimensions / pixel format (`scale=trunc(iw*sar/2)*2:trunc(ih/2)*2`); there is no downscale, no ladder, no rendition table, no variant serving.

**This is a feature, not a bug fix — scope it separately and do it LAST.** Do not let it block #38/#42/#51.

Phased plan:
- [ ] **Schema.** Migration `035_video_renditions.sql`: `video_renditions(photo_id, height, blob_id/path, codec, bitrate, size_bytes, created_at)`, unique on `(photo_id, height)`. A photo has 1..n renditions; the source is one of them.
- [ ] **Transcode.** `build_video_transcode_args` takes a target height; add `scale=-2:1080` (preserving the even-dimension and `format=yuv420p` guarantees already there) for the 1080p rung. Rules from the issue: source ≤1080p → single rendition; source >1080p → source rendition **and** a 1080p rendition.
- [ ] **Cost control.** This doubles encode work for every 4K video. Reuse the existing two-lane parallelism (`SIMPLE_PHOTOS_CONVERSION_JOBS`) and enqueue the 1080p rung at lower priority than first-pass conversions — a user must never wait on a secondary rendition to see their video at all.
- [ ] **Serving.** Extend the video endpoint with a rendition selector; keep range-request support intact (`server/src/http_utils.rs`).
- [ ] **API.** Expose available renditions per video so clients can populate the picker.
- [ ] **Web player.** Gear icon bottom-right of `web/src/components/viewer/VideoControls.tsx` → resolution menu. Default via the Network Information API where available (`navigator.connection.effectiveType` / `saveData`), falling back to highest.
- [ ] **Android player.** Same picker in `VideoPlayer.kt`; default from `ConnectivityManager` (`NET_CAPABILITY_NOT_METERED` → highest, metered → ≤1080p).
- [ ] **Android setting.** "Cellular data saver" toggle; when OFF, always serve highest regardless of network, per the issue.
- [ ] **Backfill.** Admin task to generate 1080p rungs for existing >1080p videos. Opt-in — do not silently start re-encoding someone's whole library.
- [ ] Tests: ladder selection logic (which rungs for which source height) as a pure unit test; rendition serving + range requests in E2E; picker default selection per network state.

---

## Workstream C — People / Pets

### C1 — #48 Face selection centering and missing thumbnails (High) — CONFIRMED

Four distinct defects in one issue. Treat them as four checkboxes.

**(a) Faces sit up and to the left — the zoom math is wrong, identically, on both platforms.**

Web: [web/src/utils/thumbnailCss.ts:285-303](web/src/utils/thumbnailCss.ts#L285-L303) `computeFaceCropStyle` sets `transformOrigin: cx cy` + `scale(zoom)`. **Scaling about a transform-origin holds that point stationary — it does not move it to the centre.** A face centred at (0.30, 0.25) stays at 30%/25% of the tile: up and to the left, exactly as reported. The accompanying `objectPosition: cx cy` has the same flaw (percentage object-position aligns the image's P% point with the *container's* P% point).

Android: [LibraryFeatureScreens.kt:177-192](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L177-L192) makes the **same** mistake — `graphicsLayer { scaleX = zoom; scaleY = zoom; transformOrigin = TransformOrigin(cx, cy) }`. Compounded by `contentScale = ContentScale.Crop` into a 1:1 tile: the bbox is normalised against the **full aspect-preserving thumbnail**, but it is applied after a centre-crop to square, so the coordinate spaces do not match. That is why Android is "far off" while web is merely offset.

The correct formula already exists in this repo — [PhotoInfoPanel.tsx:80-97](web/src/components/viewer/PhotoInfoPanel.tsx#L80-L97) `FaceCrop` uses the proper normalised sub-rectangle mapping (`(bbox_x / (1 - w)) * 100%`). Two implementations of the same operation, one right, one wrong.

- [ ] Fix `computeFaceCropStyle` to the correct mapping: visible fraction `z = 1/zoom`, position `p = clamp((c - z/2) / (1 - z), 0, 1)`. Verify it degenerates correctly when `z → 1`.
- [ ] Have `FaceCrop` and `computeFaceCropStyle` share **one** helper. Two copies is how they drifted.
- [ ] Android: scale about centre and translate the face centre to the middle, and correct for the centre-crop coordinate space before applying the bbox. See memory `android-crop-display-bug` — the `TopStart` scale+translate pattern is the one that works here.
- [ ] Unit-test the pure math on both platforms with known bboxes (corner, centre, edge-clamped). This is arithmetic; it must not need a device to verify.

**(b) Many People albums have no thumbnail.** [server/src/ai/handlers.rs:283-314](server/src/ai/handlers.rs#L283-L314) `fetch_face_clusters` LEFT-JOINs the representative detection, so `rep_bbox_*` is legitimately NULL when it cannot resolve — the client then renders the placeholder. **HYPOTHESIS, and it likely chains to A1:** the representative *photo* is resolved client-side against the local mirror, and the web mirror is missing every unencrypted row (`usePhotoSync.ts:205`). A cluster whose representative happens to be an unencrypted photo has no thumbnail on web but would on Android.
- [ ] Fix A1 first, then re-measure how many clusters are still thumbnail-less.
- [ ] For the genuine remainder: fall back to the next-highest-confidence detection whose photo *does* resolve, rather than giving up on the representative.
- [ ] Log (don't silently placeholder) when a cluster cannot resolve a thumbnail — otherwise this is invisible again.

**(c) Android uses square tiles where web uses circular portraits.** [LibraryFeatureScreens.kt:163-171](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L163-L171) uses `RoundedCornerShape(12.dp)`; web's People list uses `variant="avatar"`.
- [ ] Use `CircleShape` for person/pet cluster tiles on Android to match web.

**(d) The Albums page doesn't use face centering at all.** [web/src/pages/Albums.tsx:669-690](web/src/pages/Albums.tsx#L669-L690) renders the People row without applying `faceCropStyle` — that only happens in `PeopleView`.
- [ ] Apply the shared face-crop helper to the Albums-page People (and Pets) row tiles.

### C2 — #39 Cannot rename pets on Android (Low) — CONFIRMED

Clean, small, fully scoped. The whole backend path already exists: [ApiService.renamePetCluster:465](android/app/src/main/kotlin/com/simplephotos/data/remote/ApiService.kt#L465) and [AiRepository.renamePetCluster:69](android/app/src/main/kotlin/com/simplephotos/data/repository/AiRepository.kt#L69). Only the UI wiring is missing: `PersonDetailScreen` has the rename dialog ([LibraryFeatureScreens.kt:425-445](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L425-L445)) but `PetDetailViewModel`/`PetDetailScreen` ([line 528-569](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L528-L569)) has no `rename` function and no dialog. The shared dialog at [line 450](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt#L450) is already documented as "for a person/pet cluster."

- [ ] Add `rename()` to `PetDetailViewModel` calling `repo.renamePetCluster`, mirroring `PersonDetailViewModel.rename`.
- [ ] Wire the toolbar rename action + shared dialog into `PetDetailScreen`.
- [ ] Refresh the label optimistically on success; log and surface failures (no silent catch).

---

## Workstream D — Viewer UI (quick wins)

### D1 — #44 Info button still shows in the viewer (Low) — CONFIRMED

Not a bug so much as an unfinished decision from #30. Both platforms deliberately kept the standalone button, with a comment saying so:
- Web: [ViewerTopBar.tsx:121-125](web/src/components/viewer/ViewerTopBar.tsx#L121-L125) (button) and [:163](web/src/components/viewer/ViewerTopBar.tsx#L163) — *"Info lives here too (#30) — the standalone button stays up top."*
- Android: [PhotoViewerScreen.kt:1071-1075](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/PhotoViewerScreen.kt#L1071-L1075) and [:1125](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/PhotoViewerScreen.kt#L1125) — same comment.

- [ ] Remove the standalone Info button from both top bars; keep the overflow-menu entry.
- [ ] Update both comments — they currently assert the opposite of the new intent.
- [ ] Check the secure viewer (`SecurePhotoViewer.kt`) for the same duplication.
- [ ] Update any E2E selector that clicks the top-bar Info button.

### D2 — #50 Video controls collide with the phone navigation bar (Medium) — CONFIRMED

- Android: [VideoPlayer.kt:484-485](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/VideoPlayer.kt#L484-L485) uses a hardcoded `.padding(top = 32.dp, bottom = 8.dp)` with **no window-inset handling**. An 8dp bottom margin puts play/pause/mute directly under a 48dp 3-button nav bar. The correct pattern is already used elsewhere in this codebase — [SecurePhotoViewer.kt:362](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt#L362), [:649](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt#L649) and [ViewerEditPanel.kt:87](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/ViewerEditPanel.kt#L87) all apply `navigationBarsPadding()`. The main video player was simply missed.
- Web: [VideoControls.tsx:126](web/src/components/viewer/VideoControls.tsx#L126) uses `pb-3` with no safe-area inset. `grep` finds **zero** uses of `env(safe-area-inset-*)` anywhere in `web/src` — so the installed PWA overlaps the home indicator too.

- [ ] Android: add `.navigationBarsPadding()` to the video control bar, matching the secure viewer.
- [ ] Web: `padding-bottom: calc(0.75rem + env(safe-area-inset-bottom))`.
- [ ] Audit other bottom-anchored web surfaces (selection bar, banners) for the same missing inset while you are in there.
- [ ] Verify on a device with 3-button nav **and** gesture nav — they have different inset heights (see `.device-test/dev.ps1`, S21+ harness).

---

## Workstream E — Albums

### E1 — #43 Move selected items between secure albums (High) — CONFIRMED

**Reported:** no option in a secure album to add selected items to another secure album.

**Root cause: the existing feature only works in one direction.** #31 shipped a **pull** picker — from inside secure album A, browse *other* albums' items and bring them in ([web/src/gallery/secureMovePicker.ts](web/src/gallery/secureMovePicker.ts), wired at [SecureGallery.tsx:360-365](web/src/pages/SecureGallery.tsx#L360-L365)). There is no **push**: select items in the album you are looking at and send them elsewhere.

Good news — the hard parts are done. A photo lives in at most one secure album (server-enforced), so this is a **move**, and `resolveSecureMoves` + the server endpoint already implement exactly that operation. This is predominantly UI.

- [ ] Web: selection mode on the secure grid + a "Move to album" action in the selection bar → target-album picker → reuse `resolveSecureMoves`.
- [ ] Reuse `usePhotoSelection` / `SelectablePhotoGrid` rather than hand-rolling selection (see memory `selection-refactor`).
- [ ] Handle the smart-album case: in a secure smart view the "current gallery" is synthetic, so the source gallery must come from each item's own `gallery_id` — the removal path at [SecureGallery.tsx:329-334](web/src/pages/SecureGallery.tsx#L329-L334) already does this; follow that precedent.
- [ ] Batch the moves resiliently — partial failure must not lose items (see memory `a2-album-epic-27-16-25-20`, the resilient secure-add batch).
- [ ] Android parity in `SecureGallery`.
- [ ] E2E: move items A→B, assert membership on both sides and that no item ends up in two albums or none.

### E2 — #52 Sort button on the album header (Feature) — CONFIRMED

No user-facing sort exists anywhere. Ordering is hardcoded `takenAt` desc, with a single `sortBy: "addedAt"` special case for Recently Added ([smartAlbums.ts:17](web/src/gallery/smartAlbums.ts#L17), applied at [useAlbumPhotos.ts:76-77](web/src/hooks/useAlbumPhotos.ts#L76-L77)).

- [ ] Sort control on the right of the album-detail header: Date and Name, each ascending/descending.
- [ ] Implement in `useAlbumPhotos` so every album type inherits it; do not special-case per view.
- [ ] Persist the choice per album (localStorage keyed by album id), consistent with the existing `useScrollMemory` precedent.
- [ ] Interaction with burst collapse: sort **after** collapsing, and sort by the representative frame — otherwise burst tiles jump around.
- [ ] Name sort must be locale-aware (`Intl.Collator`, `numeric: true`) so `IMG_2` precedes `IMG_10`.
- [ ] Android parity.
- [ ] Unit-test the comparators, including ties and missing `takenAt`.

---

## Workstream F — Android windowing

### F1 — #41 Cap dual-window at two instances (Low) — CONFIRMED

[NewWindow.kt:108-125](android/app/src/main/kotlin/com/simplephotos/ui/navigation/NewWindow.kt#L108-L125) `openInNewWindow` has **no instance cap**. `FLAG_ACTIVITY_MULTIPLE_TASK` ([line 43-45](android/app/src/main/kotlin/com/simplephotos/ui/navigation/NewWindow.kt#L43-L45)) spawns a fresh task on every invocation, and "New Window" is offered from every `AppHeader` ([AppHeader.kt:382](android/app/src/main/kotlin/com/simplephotos/ui/components/AppHeader.kt#L382)) — so a user can open unbounded windows, all sharing one process, Room DB and Coil cache. Given A3, that is also a memory-pressure contributor.

- [ ] Track live `MainActivity` instances (static `AtomicInteger`, increment in `onCreate`, decrement in `onDestroy`) — or enumerate via `ActivityManager.appTasks`.
- [ ] `openInNewWindow` refuses at 2 and returns false; `rememberNewWindowLauncher` already surfaces a toast for false, so give it an accurate message ("Already using two windows").
- [ ] Grey out / hide the "New Window" menu item when at the cap rather than letting it fail — better UX than a toast after the fact.
- [ ] Verify the counter survives process death and configuration changes; a leaked counter permanently disables the feature, which is worse than the bug.
- [ ] Device-verify on the S21+ harness.

---

## Suggested execution order

Dependencies are real here — A1 gates the honest measurement of C1(b), and B1 gates B2/B3.

1. **A1** (#42) — counts + the pagination off-by-one. Everything downstream is measured against correct numbers.
2. **A3** (#51) — the crash. Highest user pain, self-contained once you accept virtualization.
3. **A2** (#38) — delta sync + server cache. Builds on A1's schema work.
4. **B1** (#45) — failure logging. Cheap, and makes B2/B3 diagnosable.
5. **D1** (#44), **D2** (#50), **C2** (#39), **F1** (#41) — quick wins, one commit each. Good palate cleansers between the heavy items.
6. **C1** (#48) — face centering. Do after A1 so (b) can be measured honestly.
7. **B3** (#46) — codec probing.
8. **B2** (#40) — ETA rework + 3-strike cap.
9. **E1** (#43), **E2** (#52) — album features.
10. **B4** (#49) — the resolution ladder. Largest scope; do not let it block anything above.

## Cross-cutting risks

- **A1 + A2 touch the same schema.** Sequence them or you will write two migrations that fight. Prefer one migration adding both `change_seq` and the failure-count column if they land together.
- **Burst collapse is the recurring trap.** Every count/sort/ETA change must state explicitly whether it operates on raw rows or collapsed tiles. Most of the historical count bugs in this repo are exactly this confusion.
- **The `src-` album id formula lives in three codebases** (memory `takeout-album-phases-2-3`). If E2's sort touches album identity, do not tidy one copy alone.
- **Device verification is outstanding from the previous batch** (#29–#37, per memory). Fold an S21+ pass into this batch rather than accruing more unverified Android work.
- **Do not regress the idle disk-thrash fix** (memory `idle-disk-thrash-investigation`). A2 rewrites the sync loop — the steady-state "zero downloads when nothing changed" property is a hard requirement, not a nice-to-have.
