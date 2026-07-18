# Idle Disk-Thrash Fix — server never goes quiet after import

> **STATUS (2026-07-18): Phases 1–4 IMPLEMENTED on `dev`, uncommitted.**
> Automated tests all green — server `cargo test` 272 passing (6 new), web
> `vitest` 143 passing (6 new), both typechecks clean. Only the **Verification
> (live box)** section below is outstanding — it needs a deploy to CT132
> (`deploy-fresh.ps1 -NoWipe`). Notable decisions vs. the original plan:
> - **2.4 invalidation** uses two `AFTER DELETE` triggers (on `photos` and
>   `encrypted_gallery_items`) instead of hand-edited calls at ~10 delete sites —
>   the skip table is a pure cache, over-invalidation only costs a re-hash.
> - **3.3 BLOB_DL** kept but demoted to `debug!` + given `user_id`; skipped the
>   client-IP/periodic-counter option (root cause fixed upstream in Phase 1).
> - **4.2** left `auto_scan_interval_secs` at 300 — the scan is cheap now, so
>   raising it would only trade away import latency for no real gain.
> - **3.4 log rotation** added to the repo `server/docker-compose.yml`; the box
>   copy at `/opt/simple-photos/.../docker-compose.yml` still needs it by hand.


**Symptom (2026-07-17):** live box (CT132) up 5h, importing/encrypting/conversion all
finished, yet the disk is slammed indefinitely. Diagnostics page shows Storage
collection = 12.99s, CPU time 14,846s over 5h uptime (~0.8 cores sustained),
RSS 2.98 GB, load average 9–13 with 15% iowait inside the container.

**Investigated live** (Posh-SSH → PVE 192.168.86.87 → `pct exec 132`, docker logs +
`/proc/<pid>/io` + `/proc/fs/cifs/Stats`). Two root-cause bugs plus two aggravators.
None of this is speculation — every number below was measured on the box.

---

## Findings (evidence)

### Bug A — auto-scan re-processes the same 4,254 Takeout duplicates every 5 minutes, forever
- Every interval tick logs: `run_auto_scan: 4254 new candidate files to register`
  → ~130s of grinding → `Interval auto-scan complete: registered 0 new files`.
  Cycle repeats every 300s (`scan.auto_scan_interval_secs` default). That is a
  **43% duty cycle** of full-file reads over the CIFS mount (`//192.168.86.88/vault`).
- Why: the walk in [autoscan.rs](server/src/backup/autoscan.rs#L459) only skips files whose
  `rel_path` is in `existing_set` (photos.file_path ∪ photos.source_path ∪ trash paths).
  Takeout stores the SAME bytes in "Photos from YYYY" AND in every album folder, so the
  album copies are on disk but not in any of those columns. Each pass they go through
  the full pipeline in [register.rs](server/src/photos/register.rs#L121):
  EXIF header read → XMP subtype scan → **full-file streaming SHA hash over SMB**
  → `INSERT OR IGNORE` collides on `idx_photos_user_hash (user_id, photo_hash)`
  ([002_data.sql:61](server/migrations/002_data.sql#L61)) → `rows_affected == 0` →
  album-membership backfill (`record_source_album`, a WAL write per file per pass)
  → `return false`. **The rejected path is remembered nowhere**, so next tick redoes
  all of it. Same hole for the secure-gallery hash skip at
  [register.rs:220-229](server/src/photos/register.rs#L220-L229).
- 8,490 `extract_media_metadata_async` calls in a 10-min window confirm the rate.

### Bug B — web client runs a FULL library sync every 2 seconds, unguarded
- [usePhotoSync.ts:41](web/src/gallery/hooks/usePhotoSync.ts#L41):
  `const SYNC_INTERVAL_MS = 2_000;` — the "periodic re-sync" `setInterval` fires a
  complete `loadEncryptedPhotos()` every 2 seconds. Each run pages the ENTIRE photo
  table from the server (~15 × 500-row requests for 7,371 photos) plus 4 blob-media
  page sweeps, then iterates every photo. In git since at least Apr 16 (644a4bb).
- `loadEncryptedPhotos` has **no re-entrancy guard** — a run takes far longer than 2s,
  so many overlapping syncs run concurrently, each seeing stale Dexie state and each
  re-downloading thumbnails the others haven't persisted yet.
- The Jul 16 thumbs refactor (bytes moved off photo rows into the Dexie `thumbs`
  table) emptied the browser-side cache, so overlapping syncs now hit the
  re-download branches ([usePhotoSync.ts:187](web/src/gallery/hooks/usePhotoSync.ts#L187) and
  [:210](web/src/gallery/hooks/usePhotoSync.ts#L210)) at full volume.
- Measured server-side: **16,266 thumbnail `BLOB_DL` requests within the last hour**
  (14,107 in 10 min ≈ 28/s sustained; 7,364 distinct ≈ the whole library; hottest
  blob served 17× in 10 min). Every one = SMB read + AES-GCM decrypt + HTTP response.
- This also explains the sustained ~0.8 core CPU and most of the **89.4 GB written
  to CT132's local disk in 5h** (`/proc/<pid>/io write_bytes`): SQLite WAL/checkpoint
  churn from per-request bookkeeping + per-pass `record_source_album` writes.

### Aggravator C — per-file INFO log spam
- Thousands of INFO lines per scan pass: `Skipping unedited Google Photos original`
  (one per shadowed file, [autoscan.rs:443-449](server/src/backup/autoscan.rs#L443-L449)),
  `[metadata] extract_media_metadata_async result` per file
  ([media.rs:431](server/src/photos/metadata/media.rs#L431)), `BLOB_DL` per request.
- Docker json log hit **151 MB in 5h** (no rotation configured) — constant local
  disk writes, and the signal (scan summaries, errors) drowns.
- `BLOB_DL` lines carry **no requester identity** (no user, no IP) — we could not
  tell WHICH client was hammering without inference. That's an observability bug.

### Aggravator D — diagnostics Storage collector walks the whole SMB tree per page load
- `collect_storage_stats` → `dir_usage` ([handlers.rs:160-203](server/src/diagnostics/handlers.rs#L160-L203))
  recursively collects **every DirEntry into a Vec** then stats each file: 12.99s per
  diagnostics fetch, all over CIFS. On-demand only, but it's the red bar in the
  screenshot and it's uncached.

---

## Phase 1 — Web: kill the 2-second sync storm  *(do first: biggest live relief, tiny diff)*
- [ ] 1.1 `SYNC_INTERVAL_MS` 2_000 → **300_000** (5 min). Realtime updates already
      arrive via SSE `/api/sync/events`; the poll is only a safety net.
- [ ] 1.2 Add re-entrancy guard to `loadEncryptedPhotos` (in-flight ref; skip the tick
      if a sync is still running). Without this, ANY interval value still stacks runs
      on slow links.
- [ ] 1.3 Unit test (vitest, `web/src/db/thumbs.test.ts` or new `usePhotoSync` test):
      a photo whose thumb resolves from the `thumbs` table must NOT trigger
      `api.blobs.download` on a subsequent sync pass (mock the api module).
- [ ] 1.4 `grep -rn "setInterval" web/src` — audit every other poller for sane
      intervals + guards (SecureGallery token freshness, processing store, banners).

## Phase 2 — Server: remember scan-rejected paths (kills the 4,254-file loop)
- [ ] 2.1 Migration **031_scan_skipped_paths.sql**:
      `scan_skipped_paths(user_id TEXT, rel_path TEXT, size_bytes INTEGER,
      mtime TEXT, reason TEXT, photo_hash TEXT, created_at TEXT,
      PRIMARY KEY (user_id, rel_path))`.
- [ ] 2.2 [register.rs](server/src/photos/register.rs): on BOTH terminal-skip paths —
      hash-duplicate (`rows_affected == 0`, after the album backfill) and
      gallery-hidden hash match — `INSERT OR REPLACE` the candidate's rel_path,
      size, mtime, reason (`hash_duplicate` / `gallery_hidden`), hash.
- [ ] 2.3 [autoscan.rs](server/src/backup/autoscan.rs) `run_auto_scan`: load the table into
      `HashMap<rel_path, (size, mtime)>` next to `existing_set`; during the walk,
      skip a candidate when present AND size+mtime unchanged. If changed: delete the
      row and process normally (file was replaced — must re-evaluate).
- [ ] 2.4 Invalidation: when a photo row is deleted (trash purge / secure-gallery
      original removal), `DELETE FROM scan_skipped_paths WHERE photo_hash = ?` so a
      re-dropped copy can register again. Find the delete/purge sites and add the
      one statement (photos delete path + egi removal path).
- [ ] 2.5 Album backfill only needs to run once per (photo, album) — with 2.3 the
      steady-state pass does zero `record_source_album` writes. Verify
      `record_source_album` is `INSERT OR IGNORE` (idempotent, no WAL churn on
      conflict) while in there.
- [ ] 2.6 Tests (`cargo test --bin simple-photos-server` — crate is a BINARY, see
      memory): seed a storage dir with a date-folder file + identical album copy →
      first scan registers 1 + records skip row for the copy → second scan finds
      **0 candidates** (assert via returned count and/or skip-table state). Add a
      case for gallery-hidden hash and for "skip row goes stale when file changes".

## Phase 3 — Log hygiene
- [ ] 3.1 Demote to `debug!`: per-file shadowed-original skip, per-file
      `extract_media_metadata_async result`, per-file duplicate/backfill lines.
- [ ] 3.2 Replace with one per-pass INFO summary:
      `scan pass: N walked, N shadowed, N known-dups skipped, N candidates, N registered, took Xs`.
- [ ] 3.3 `BLOB_DL` line: add `user_id` + client IP (extract from request extensions /
      `ConnectInfo`), demote the per-request line to `debug!` and keep an INFO-level
      periodic counter, OR keep INFO but with identity. Never again an anonymous
      28/s torrent.
- [ ] 3.4 Log rotation for the container: `logging: { driver: json-file, options:
      { max-size: "50m", max-file: "3" } }` in the compose file on the box
      (`/opt/simple-photos/docker-instances/simple-photos/docker-compose.yml`) and in
      the repo copy if one exists (check `server/` and `deploy-fresh.sh` templates).

## Phase 4 — Optional hardening (separate commits, only after 1–3 verified)
- [ ] 4.1 `dir_usage`: stream the walk (running sum/count, no `Vec<DirEntry>` of the
      whole tree) + cache `StorageStats` with a ~5-min TTL so diagnostics loads stop
      re-walking SMB.
- [ ] 4.2 Consider raising `scan.auto_scan_interval_secs` default (300 → 900) now that
      SSE covers realtime; scan is a janitor, not a hot path.

## Verification (live box, after deploy)
- [ ] Deploy with `deploy-fresh.ps1 -NoWipe` (do NOT wipe — library is populated).
- [ ] `docker logs -f` for 15+ min: scan passes report **0 candidates** and finish in
      seconds; with the web app open+idle, `BLOB_DL` is near zero.
- [ ] `/proc/<server pid>/io`: `write_bytes` growth ≈ 0 at idle; `rchar` flat between
      scan ticks.
- [ ] Load average back under ~1; `/proc/fs/cifs/Stats` reads flat between ticks.
- [ ] Diagnostics page: Storage collection time drops (post-4.1) and total collection
      well under a second on cache hits.

## Measured baseline (for before/after comparison)
| Metric | Before |
|---|---|
| Scan pass | 4,254 candidates, ~130s, every 300s |
| Thumbnail downloads | ~28/s sustained (16,266/h) |
| Server local-disk writes | 89.4 GB in 5h |
| Docker json log | 151 MB in 5h |
| Load avg (CT132) | 9–13, 15% iowait |
| CPU | ~0.8 cores sustained |
