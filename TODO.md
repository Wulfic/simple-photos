# TODO — Investigation: Android crashes, Takeout folders, per-client recount

Investigation date: 2026-07-03. Three reported issues, root-caused below with
concrete code references. Ordered by severity. **Nothing here is "done" until it
has a unit test AND a device/E2E verification — see AGENTS.md.**

---

## Issue 1 — Android app crashes consistently, over and over — ✅ FIXED & DEVICE-VERIFIED

### Root cause (confirmed via `adb logcat -b crash` on the S21+ / Android 15)
```
java.lang.IllegalArgumentException: foregroundServiceType 0x00000001 is not a
subset of foregroundServiceType attribute 0x00000000 in service element of
manifest file
   at androidx.work.impl.foreground.SystemForegroundService.startForeground
```
WorkManager's library-declared `SystemForegroundService` had **no**
`foregroundServiceType` in the merged manifest (0x0), but
[BackupWorker.buildForegroundInfo()](android/app/src/main/kotlin/com/simplephotos/sync/BackupWorker.kt#L107)
promotes to `FOREGROUND_SERVICE_TYPE_DATA_SYNC` (0x1) at runtime. On
targetSdk 34 / Android 14+ the runtime type must be a **subset** of the
manifest-declared type → **fatal throw**. It fires on the service's own Handler
(`SystemForegroundService$1.run`), *not* in the coroutine that called
`setForeground`, so `promoteToForeground`'s try/catch is **useless** against it.
`BackupWorker` is a 15-min **periodic** worker (+ reactive one-time), so it fires
on launch and every 15 min → the "over and over" crash loop. Two PIDs crashing
<1 s apart in the capture confirmed the loop.

> The earlier top suspect — the `runBlocking { dataStore.data.first() }` ANR at
> `MainActivity.kt:74` — was a **red herring**. It was not the crash.

### The fix (applied)
Merge the FGS type onto WorkManager's service element in
[AndroidManifest.xml](android/app/src/main/AndroidManifest.xml) (root now carries
`xmlns:tools`):
```xml
<service android:name="androidx.work.impl.foreground.SystemForegroundService"
         android:foregroundServiceType="dataSync" tools:node="merge" />
```
The `FOREGROUND_SERVICE_DATA_SYNC` permission was already present; only the
service-element type merge was missing.

### Verification (device)
- [x] Real trace captured (`adb logcat -b crash`).
- [x] Fix applied; debug APK reinstalled over v132 (`-r`, session preserved).
- [x] Cold start: process stays alive, **crash buffer empty**, no crash signature
      in any log buffer.
- [x] `dumpsys activity services` shows `SystemForegroundService isForeground=true
      foregroundId=4711 types=0x00000001` — the promotion that used to crash now
      **succeeds**; MainActivity is `topResumedActivity`; `BackupWorker` uploads
      photos live.

### Retained as defense-in-depth (built, not the fix)
- `CrashHandler.kt` — global `Thread.setDefaultUncaughtExceptionHandler` →
  `filesDir/crashlogs/` (synchronous), chains to previous handler, drains to the
  server on next boot via `uploadPendingCrashes`. Now future crashes are
  observable without needing a wired device. (Upload path present but untriggered
  — nothing to drain now the crash is gone.)
- `MainActivity` `produceState` — moved the biometric-pref read off the main
  thread (removes a latent ANR risk regardless).

### Follow-up (separate, non-blocking)
- [ ] One `BackupWorker: Upload failed: HTTP 500` was observed among many
      successes during verification — a **server-side** issue, unrelated to the
      crash. Investigate separately.

---

## Issue 2 — Google Photos folder/album structure not preserved on import

### Root cause (confirmed)
The **server-side Takeout import discards the album/folder structure entirely.**
[import/takeout.rs `import_takeout`](server/src/import/takeout.rs#L183) recursively
flattens every media file (`queue`/`read_dir`) and inserts each into `photos`
**with no record of its parent album folder**. For files outside the storage root
it even copies them flat into `uploads/{filename}`
([takeout.rs:319-328](server/src/import/takeout.rs#L319)), destroying the on-disk
folder too. Google Takeout's album = the parent directory name; that name is read
and thrown away.

Album re-creation exists but is a **separate, manual, WEB-ONLY, post-sync step**:
[TakeoutAlbumsImport.tsx](web/src/components/TakeoutAlbumsImport.tsx) +
[utils/takeoutAlbums.ts](web/src/utils/takeoutAlbums.ts). The user must, *after*
import + full sync, re-select the same `Google Photos` folder in a browser
folder-picker, and it matches album→photo **by filename** against already-synced
IndexedDB photos. Failure modes that make "folders still not maintained":
- **Manual + easily missed** — a normal import yields a flat gallery; nobody
  re-runs the second step.
- **Filename matching is fragile** — Takeout `IMG_1234(1).jpg` collision renames,
  `-edited` dedup (album folder lists the *original* name but
  `dedupeGooglePhotosEdits` keeps the *edited* file) → `albumsUnmatched`.
- **Web only** — no Android equivalent; Android users never get albums back.
- **Timing** — only matches photos already synced into IndexedDB; on a fresh
  device the whole gallery must finish syncing first.
- **Scale** — a 90 GB export through `webkitdirectory` enumerates every File in
  the browser; likely to choke.

### Tasks
- [ ] **Capture the album at import time, server-side.** In `import_takeout`,
      derive the album name from the media file's parent folder (skip `Photos
      from YYYY` date folders and the `Takeout`/`Google Photos` containers — mirror
      the regex/allowlist already in
      [takeoutAlbums.ts:22-25](web/src/utils/takeoutAlbums.ts#L22)). Persist an
      authoritative `photo_id → album_name` mapping (new table or a
      `source_album` column), independent of filename.
- [ ] **Rebuild client manifests from the authoritative mapping**, keyed by
      photo id (not filename). Works cross-platform and survives renames/edits.
- [ ] **Add Android album recreation** (or make the server-driven mapping the
      single source so both clients converge).
- [ ] Handle a photo in **multiple** albums (Takeout duplicates it across folders).
- [ ] Unit test: nested Takeout tree (album folders + date folders + edited dupes)
      → correct album set. E2E: import fixture → albums appear on web AND Android
      with no manual second step.

---

## Issue 3 — Server "recounts" photos for every client; no shared cache

### Root cause (confirmed)
**There is no server-side aggregate/summary endpoint.** Every client rebuilds its
entire local cache and derives counts itself by paginating the full gallery:
- Server `GET /api/photos/encrypted-sync`
  ([gallery/sync.rs](server/src/gallery/sync.rs)) returns per-photo rows, 500 at a
  time, with **correlated `id NOT IN (SELECT …)` subqueries re-run on every page**
  ([sync.rs:87-89](server/src/gallery/sync.rs#L87)) — expensive on large libraries.
- Web [usePhotoSync.ts](web/src/gallery/hooks/usePhotoSync.ts) pages the whole
  endpoint **plus** re-fetches every blob-list page for photo/gif/video/audio, on
  a **2-second interval** ([usePhotoSync.ts:39](web/src/gallery/hooks/usePhotoSync.ts#L39)).
- Web album counts come from IndexedDB
  ([Albums.tsx:243](web/src/pages/Albums.tsx#L243)) — but gated behind
  `encryptedDataReady`, which only flips true **after a full network re-sync
  completes** ([usePhotoSync.ts:51,110](web/src/gallery/hooks/usePhotoSync.ts#L51)).
  So even though IndexedDB is already populated and persisted, the UI shows
  0/spinner until a full round-trip finishes — **on every page open**. This is the
  "wait for it to count every time" the user sees.
- Android is the same architecture: counts read Room
  ([AlbumViewModel.loadSmartAlbumCounts](android/app/src/main/kotlin/com/simplephotos/ui/screens/album/AlbumViewModel.kt#L147)),
  but Room is populated by paginating the same `encrypted-sync`
  ([PhotoRepository.syncFromServerEncrypted](android/app/src/main/kotlin/com/simplephotos/data/repository/PhotoRepository.kt#L888)).
  No shared/precomputed counts pass between devices.

### Tasks
- [ ] **Add `GET /api/photos/summary`** returning precomputed counts in one cheap
      round-trip: total, favorites, photos, gifs, videos, audio, recent-capped —
      via `COUNT(*) … GROUP BY media_type` + favorite/burst-collapse counts. Cache
      the result in `AppState` per user; invalidate on any photo insert/trash/
      favorite toggle. Clients render smart-album counts from this **instantly**,
      before/without a full gallery sync.
- [ ] **Web: stop gating cached data on a full re-sync.** Show `rawEncryptedPhotos`
      from IndexedDB immediately on mount; treat the network sync as a background
      refresh, not a precondition for display
      ([usePhotoSync.ts:51](web/src/gallery/hooks/usePhotoSync.ts#L51)).
- [ ] **Make re-sync incremental** — add `updated_since` / ETag support to
      `encrypted-sync` so steady-state polls transfer deltas, not the whole list.
      Reconsider the 2 s interval (move to SSE `sync_tx`, which already exists).
- [ ] Optimize the `encrypted-sync` query — replace the correlated `NOT IN`
      subqueries with a `LEFT JOIN … WHERE … IS NULL` (or precomputed exclusion),
      indexed on `(user_id, taken_at, id)`.
- [ ] Unit test the summary counts vs. the collapsed-grid counts (they must match,
      incl. burst collapse). E2E: open Albums on device A, then device B, then
      reopen — counts appear immediately from `/summary`, no visible recount.

---

## Cross-cutting
- [ ] Server `/summary` + Android crash handler both need logging on every error
      path (AGENTS.md non-negotiable).
- [ ] Persist this investigation to memory (done: see
      `memory/investigation-crash-takeout-recount-2026-07-03.md`).
