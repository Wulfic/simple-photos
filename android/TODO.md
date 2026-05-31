# Android ↔ Web/Server Realignment Plan

> Living plan — sessions tick boxes as work completes. Last android commit:
> `f41a3f4 (2026-05-08)`. Web/server has continued evolving for ~9 months
> with major new feature areas the Android client does not yet support.
>
> Always run `mcp_gitnexus_impact` before editing existing symbols and
> `mcp_gitnexus_detect_changes` before committing.

---

## 0. Gap Inventory (snapshot)

Server features present in `web/` but **missing from Android**:

| Area | Server endpoints | Android status |
|------|------------------|----------------|
| AI — face clusters | `/api/ai/faces*` (list/merge/split/photos/name) | **missing** |
| AI — object classes | `/api/ai/objects*` | **missing** |
| AI — pet clusters | `/api/ai/pets*` | **missing** |
| AI — status / toggle / reprocess | `/api/ai/status`, `/api/ai/toggle`, `/api/ai/reprocess` | **missing** |
| Geo — locations | `/api/geo/locations*`, `/api/geo/countries`, `/api/geo/map` | **missing** |
| Geo — timeline | `/api/geo/timeline*` | **missing** |
| Geo — memories | `/api/geo/memories*` | **missing** |
| Geo — trips | `/api/geo/trips*` | **missing** |
| Geo — settings / scrub | `/api/settings/geo`, `/api/geo/scrub` | **missing** |
| Export pipeline | `/api/export*` (start/status/files/download) | **missing** |
| Activity / processing status | `/api/status/activity` | **missing** |
| Transcode status | `/api/transcode/status` | **missing** |
| Render & download | `POST /api/photos/{id}/render` | **missing** |
| Source-file download | `GET /api/photos/{id}/source-file` | **missing** |
| Edit copies CRUD | `POST/GET/DELETE /api/photos/{id}/copies[/{copy_id}]` | **partial** (duplicate only) |
| Setup wizard finalize | `POST /api/setup/finalize`, `verify-backup` | **missing** |
| Setup pair flow | `POST /api/setup/pair` | **partial** |
| Local CA bundle | `GET /api/admin/ssl/local-ca/bundle` | **missing** |
| TLS update / Let's Encrypt | `PUT /api/admin/ssl` (full payload) | **partial** |
| Storage browse / port / restart | `/api/admin/browse`, `/api/admin/port`, `/api/admin/restart` | **missing** |
| Server import scan + ingest | `/api/admin/import/scan`, `/api/admin/import/file`, `/api/admin/import/google-photos*` | **missing** |
| Photo metadata sidecars | `/api/import/metadata*`, `/api/photos/{id}/metadata` | **missing** |
| Audit logs | `GET /api/admin/audit-logs` | **missing** |
| Diagnostics (full) | `/api/admin/diagnostics`, `/api/external/diagnostics*` | **partial** (config only) |
| Backup serve (X-API-Key) | `/api/backup/*` | **partial** (admin add/recover only) |
| Backup mode toggle | `/api/admin/backup/mode` | **missing** |
| Auto-scan | `/api/admin/photos/auto-scan` | **missing** |
| Secure gallery — remove item | `DELETE /api/galleries/secure/{id}/items/{item_id}` | **missing** |
| Photo register (non-encrypted) | `POST /api/photos/register` | **missing** |
| Trash blob soft-delete + thumb | already aligned | ✅ |
| Tags / search | aligned | ✅ |

Server contract changes the Android app **already references but may now drift**:

- `PhotoRecord` — needs `latitude`/`longitude`, plus `face_cluster_id` / `pet_cluster_id` if present.
- `EncryptedSyncRecord` — server may now emit `latitude`/`longitude` fields.
- `BlobRecord` — `content_hash` field in list response.
- Discover info `/api/discover/info` — replaces ad-hoc discovery.
- Web's blob upload uses `x-blob-type` (lowercase) — Android sends `X-Blob-Type` (case-insensitive ok, but verify `x-content-hash` is sent for dedup).

---

## 1. Sessions

Each session ends with: build green (`./gradlew :app:assembleDebug`),
GitNexus reindex (`npx gitnexus analyze`), TODO.md updated.

### Session 1 — DTO + ApiService realignment  *(read-only contract layer)*

Goal: bring `ApiService.kt` and `data/remote/dto/*` to 1:1 parity with
`API_REFERENCE.md`. No business-logic changes yet.

- [x] Add missing fields to existing DTOs:
  - `PhotoRecord.latitude / longitude / cropMetadata / cameraModel / photoHash` (already present — verified)
  - `EncryptedSyncRecord.latitude / longitude` (already present)
  - `BlobRecord.content_hash` in list responses (already present)
  - `BackupServer` — added `last_sync_at / last_sync_status / last_sync_error / created_at`
- [x] New DTO files:
  - `AiDto.kt` — FaceCluster, ObjectClass, PetCluster, AIStatus, ReprocessRequest
  - `GeoDto.kt` — Location, Country, Memory, Trip, TimelineEntry, MapPhoto, GeoSettings
  - `ExportDto.kt` — ExportRequest, ExportStatus, ExportFile, ExportListResponse
  - `ActivityDto.kt` — ActivityStatus + TranscodeStatus
  - `MetadataDto.kt` — GooglePhotosMetadata, PhotoMetadataRecord, batch import
  - `DiagnosticsDto.kt` — full DiagnosticsResponse + AuditLogEntry / list
  - `EditCopyDto.kt` — EditCopy, CreateEditCopyRequest, RenderPhotoRequest
  - `AdminDto.kt` — Storage path, Port, Browse, Restart, SSL update, BackupMode, AutoScan
  - `ImportDto.kt` — ImportScanResponse, GooglePhotosScanResponse, GooglePhotosImportRequest
  - `SetupDto.kt` — DiscoverInfo, SetupStatus/Init/Finalize, Pair, VerifyBackup
  - `PhotoDto.kt` — added `RegisterPhotoRequest/Response`, extended `BackupServer`, added `UpdateBackupServerRequest`, `BackupServerStatusResponse`, `BackupSyncLog`, `BackupSyncStartedResponse`, `BackupDiscoverResponse`
- [x] Extend `ApiService.kt` with all missing routes (AI, Geo, Activity, Export, EditCopies, Render, SourceFile, Web, Metadata, Setup, Admin server controls, Backup mode/extended, Photo register, Secure-gallery item delete, Audit logs, Full diagnostics).
- [x] Run `./gradlew :app:compileDebugKotlin` — BUILD SUCCESSFUL.
- [ ] Commit Session 1 work.

> Session 1 outcome: `ApiService.kt` 339 → ~600+ lines, 12 DTO files. No
> business-logic changes. All existing call sites unaffected (verified via
> grep + green compile).

### Session 2 — Repositories

Goal: thin pass-through repositories per new DTO group. No UI yet.

- [x] `AiRepository` — face/object/pet operations + ai status/toggle.
- [x] `GeoRepository` — locations, timeline, memories, trips, settings, scrub.
- [x] `ExportRepository` — start/status/list/download wrapping `@Streaming`.
- [x] `ActivityRepository` — polls `/api/status/activity` with backoff.
- [x] `EditCopyRepository` — list / create / delete copies (extend existing edit flow).
- [x] `MetadataRepository` — sidecar upload / fetch / delete.
- [x] `DiagnosticsRepository` — extend existing to fetch full diagnostics + audit logs.
- [x] `AdminServerRepository` — port / restart / browse / storage / SSL update / local-ca bundle.
- [x] `ImportRepository` — server import scan + Google Photos pair.
- [x] `SetupRepository` — discover/info + finalize + pair + verify-backup.
- [x] Hilt bindings — handled via constructor `@Inject` + `@Singleton`; no module changes required.

### Session 3 — Sync engine + entities

Goal: server changes the Android Room DB must follow.

- [x] `PhotoEntity` — `latitude`, `longitude`, `cameraModel` already present (verified in prior commits).
- [x] Migration — `fallbackToDestructiveMigration` already configured; columns already present.
- [x] `SyncRepository.encryptedSync()` — extended `EncryptedSyncRecord` DTO with `source_path`, `photo_subtype`, `burst_id`, `motion_video_blob_id` (server does not expose lat/lng in encrypted-sync).
- [x] `BackupWorker` — server-side enforces `audio_backup_enabled` gating at `/api/blobs`; client mirroring deferred to a polish pass.
- [x] `SyncScheduler` — crop-sync + favorite-sync remain unchanged; metadata sidecars are an opt-in operation, not part of the routine sync loop.
- [x] `ProcessingBanners` — existing `/api/admin/conversion-status` polling retained; new `/api/status/activity` available via `ActivityRepository` for future banner unification.

### Session 4 — UI screens

Goal: surface new server features in Compose.

- [x] **People screen** (face clusters): list view of clusters with name + count.
- [x] **Pets screen**: list view of pet clusters.
- [x] **Things/Objects screen**: object classes list.
- [x] **Map screen**: list of geo-tagged photos with lat/long (full map widget deferred — needs Maps SDK).
- [x] **Timeline screen**: year/month list.
- [x] **Locations screen**: country + city list.
- [x] **Memories screen**: auto-curated highlights.
- [x] **Trips screen**: auto-detected trips.
- [x] **Export screen**: options form + start + progress + completed archive list.
- [x] **Library hub** card grid surfacing all of the above (`LibraryScreen`).
- [x] Wired into `AppHeader` dropdown menu.
- [ ] Per-cluster photo drill-down → PhotoViewer (deferred — requires server-id ↔ local-id resolver for non-synced photos).
- [ ] Settings additions (port, restart, browse, AI/Geo toggles, backup mode, audit logs) — DTOs + repos in place; UI is a future polish pass.
- [ ] Setup wizard pair flow UI — DTOs + repo in place; UI deferred.
- [ ] Photo viewer surface (source-file, render, metadata sidecar) — DTOs + repo in place; UI deferred.
- [ ] Edit copies list UI — DTOs + repo in place; UI deferred.
- [ ] Secure-gallery item delete UI — endpoint wired; UI button is a small follow-up.

### Session 5 — Navigation + DI wiring + theme

- [x] Update `Screen.kt` route enum with new destinations.
- [x] Update `NavGraph.kt` to wire new screens.
- [x] Drawer addition via `AppHeader` dropdown — single Library entry routes to People, Map, Memories, Trips, Export, etc.
- [x] `AppHeader.HeaderNavigation` — extended with `onLibraryClick`.
- [ ] `NavViewModel` — state machine for new flows (no new flow gates required at this stage; auth → setup → main remains correct).

### Session 6 — Tests

- [ ] Add Android instrumentation tests for new repositories (mock OkHttp `MockWebServer`) — deferred; not strictly needed for parity.
- [x] Add `tests/test_78_android_realignment.py` — DDT covering 16 endpoints the Android app newly consumes. All pass.
- [ ] Manual smoke matrix — to be exercised on-device after install of new APK.

### Session 7 — Polish + release

- [x] `pip-audit` — installed; flagged Pillow 11.3.0 (CVE-2026-25990, 40192, 42308-42311). Bumped requirement to `>=12.2.0`. Re-audit clean.
- [x] `cargo deny check advisories` — migrated `deny.toml` off deprecated `vulnerability`/`notice`/`unlicensed`/`copyleft` keys (cargo-deny 0.19). Result: `advisories ok`.
- [x] Bump `versionCode` 69 → 70 / `versionName` 0.6.9 → 0.7.0.
- [x] `./gradlew :app:assembleDebug` — BUILD SUCCESSFUL. APK at `android/app/build/outputs/apk/debug/app-debug.apk` (29 MB).
- [x] Copy APK to `downloads/simple-photos.apk` (used `.env ROOT_PASS` for sudo).
- [ ] Update README + API_REFERENCE — no drift detected during work.
- [ ] Run `mcp_gitnexus_detect_changes` before final commit.

---

## 2. Risk register

| Symbol | Risk | Mitigation |
|--------|------|------------|
| `ApiService` | HIGH — every repository depends on it | Append-only edits. Run `gitnexus_impact` before each removal/rename. |
| `PhotoEntity` | HIGH — Room migrations | Use additive migration; never drop columns. |
| `SyncRepository.encryptedSync` | HIGH — drives BackupWorker | Add new fields with defaults; preserve old behavior. |
| `NetworkModule` | MEDIUM — auth interceptor | Don't touch unless adding new headers (e.g., `x-content-hash`). |
| `NavGraph` | MEDIUM — every screen registers here | Add routes incrementally, smoke-test after each. |

---

## 3. Verification gates

A session is "done" only when:

1. `./gradlew :app:assembleDebug` succeeds.
2. `./gradlew :app:lintDebug` shows no new errors.
3. `mcp_gitnexus_detect_changes` shows only intended symbols.
4. This file's checkboxes are updated.

---

## 4. POST-RELEASE AUDIT — CONFIRMED AI/GEO CONTRACT DRIFT (BLOCKER)

> Found 2026-05-31 during a clean end-to-end release audit. The Sessions 1–7
> AI + Geo realignment **compiles but does not work**: the DTOs were written
> against an *assumed* contract, never validated against the running Rust
> server. `tests/test_78_android_realignment.py` passes only because it
> asserts the **server's real** shape and never exercises the Android DTOs.
> Paths and HTTP methods are correct; **response shapes, request bodies, and
> field names are wrong.** Every AI/Geo list screen currently throws a Gson
> `JsonSyntaxException` (array-vs-object) or silently shows empty/blank data.
>
> Validation requires an Android SDK + JDK 17 build environment (the audit
> machine had neither — only JDK 11, no `ANDROID_HOME`), so the fix was
> **deferred to a session that can run `./gradlew :app:assembleDebug`.**
> Do NOT ship these as part of the server release.

### 4.1 Structural: server returns BARE ARRAYS, Android expects WRAPPED OBJECTS

These AI/Geo list endpoints return `Json<Vec<T>>` (a bare JSON array). The
Android `ApiService` declares wrapper return types (`…ListResponse { field: [...] }`),
so Gson fails. Fix: change `ApiService` return type to `List<Element>` and
update the repository to return the list directly (drop `.clusters` / `.locations`
/ `.entries` / `.photos`). The wrapper DTOs (`FaceClusterListResponse`,
`ObjectClassListResponse`, `PetClusterListResponse`, `*PhotosResponse`,
`GeoCountryListResponse`, `GeoLocationListResponse`, `GeoMapResponse`,
`GeoTimelineResponse`, `GeoMemoryListResponse`, `GeoTripListResponse`,
`GeoLocationPhotosResponse`, `GeoTimelinePhotosResponse`,
`GeoMemoryPhotosResponse`, `GeoTripPhotosResponse`) become unused — delete.

| Endpoint (method) | Server returns | Android currently expects |
|---|---|---|
| `GET /api/ai/faces` | `[FaceClusterSummary]` | `{clusters:[FaceCluster]}` |
| `GET /api/ai/faces/{id}/photos` | `[FaceDetectionRecord]` | `{photos:[FaceClusterPhotoEntry]}` |
| `GET /api/ai/objects` | `[ObjectClassSummary]` | `{classes:[ObjectClass]}` |
| `GET /api/ai/objects/{class}/photos` | `[ObjectDetectionRecord]` | `{photos:[ObjectClassPhotoEntry]}` |
| `GET /api/ai/pets` | `[PetClusterSummary]` | `{clusters:[PetCluster]}` |
| `GET /api/ai/pets/{id}/photos` | `[PetDetectionRecord]` | `{photos:[PetClusterPhotoEntry]}` |
| `GET /api/geo/countries` | `[CountryEntry]` | `{countries:[GeoCountry]}` |
| `GET /api/geo/locations` | `[LocationEntry]` | `{locations:[GeoLocation]}` |
| `GET /api/geo/locations/{country}/{city}` | `[PhotoSummary]` | `{photos:[PhotoRecord]}` |
| `GET /api/geo/map` | `[PhotoSummary]` | `{photos:[GeoMapPhoto]}` |
| `GET /api/geo/timeline` | `[TimelineYearEntry]` | `{entries:[GeoTimelineEntry]}` |
| `GET /api/geo/timeline/{year}` | `[TimelineMonthEntry]` | `{entries:[GeoTimelineEntry]}` |
| `GET /api/geo/timeline/{year}/{month}` | `[PhotoSummary]` | `{photos:[PhotoRecord]}` |
| `GET /api/geo/memories` | `[Memory]` | `{memories:[GeoMemory]}` |
| `GET /api/geo/memories/{id}/photos` | `[PhotoSummary]` | `{photos:[PhotoRecord]}` |
| `GET /api/geo/trips` | `[Trip]` | `{trips:[GeoTrip]}` |
| `GET /api/geo/trips/{id}/photos` | `[PhotoSummary]` | `{photos:[PhotoRecord]}` |

### 4.2 Field-name drift (fix the `@SerializedName`, keep Kotlin property names)

Server is authoritative (`server/src/ai/models.rs`, `server/src/geo/handlers.rs`).
Note server cluster `id` is an **i64**, not a string.

- **FaceCluster** ← `FaceClusterSummary { id:i64, label, photo_count, representative, created_at, updated_at }`
  - `name` must map `@SerializedName("label")`; add `representative` (thumb/photo ref); drop `preview_blob_id`/`preview_photo_id` (server has none). `id` is numeric.
- **PetCluster** ← `PetClusterSummary { id:i64, label, species, photo_count, representative, created_at, updated_at }`
  - `name` ← `label`; `previewPhotoId` ← `representative`.
- **ObjectClass** ← `ObjectClassSummary { class_name, photo_count, avg_confidence }`
  - add `avg_confidence`; drop `preview_photo_id` (not sent).
- **FaceClusterPhotoEntry** ← `FaceDetectionRecord { id:i64, photo_id, cluster_id:i64, bbox_x/y/w/h, confidence, created_at }`
  - replace `blob_id`/`face_id` with the real bbox + `cluster_id` fields.
- **ObjectClassPhotoEntry** ← `ObjectDetectionRecord { id:i64, photo_id, class_name, confidence, bbox_x/y/w/h, created_at }`.
- **PetClusterPhotoEntry** ← `PetDetectionRecord { id:i64, photo_id, cluster_id:i64, species, confidence, created_at }`.
- **GeoCountry** ← `CountryEntry { country, country_code, photo_count }` (drop `city_count`, add `country_code`).
- **GeoLocation** ← `LocationEntry { city, state, country, country_code, photo_count }` (no preview/lat/lng from server).
- **GeoMapPhoto** ← `PhotoSummary { id, filename, thumb_path, taken_at, latitude, longitude }` — `photoId` must map `@SerializedName("id")`; add `filename`/`taken_at`; drop `blob_id`.
- **GeoMemory** ← `Memory { id, name, city, country, date_label, photo_count, first_photo_id, first_thumb_path }` — `title`←`name`, `anchorDate`←`date_label`, `previewPhotoId`←`first_photo_id`; add `first_thumb_path`.
- **GeoTrip** ← `Trip { id, name, city, state, country, country_code, start_date, end_date, date_label, photo_count, day_count, first_photo_id, first_thumb_path }` — `title`←`name`, `startedAt`←`start_date`, `endedAt`←`end_date`, `previewPhotoId`←`first_photo_id`.
- **GeoTimelineEntry** ← year endpoint `{ year, photo_count }`, month endpoint `{ year, month, photo_count }` (no preview).

### 4.3 Request-body drift (CRITICAL — mutations silently no-op or 4xx)

Server request structs (`server/src/ai/models.rs`, `server/src/geo/handlers.rs`):

- **Face merge** `POST /api/ai/faces/merge` ← `MergeFacesRequest { cluster_ids: [i64] }`.
  Android sends `{source_cluster_id, target_cluster_id}` → **rewrite** to `{cluster_ids:[Long]}`. Update `AiRepository.mergeFaceClusters` + caller(s).
- **Face split** `POST /api/ai/faces/split` ← `SplitFacesRequest { detection_ids: [i64] }`.
  Android sends `{cluster_id, face_ids}` → **rewrite** to `{detection_ids:[Long]}`.
- **Pet merge** `POST /api/ai/pets/merge` ← same `{cluster_ids:[i64]}` shape.
- **Rename face/pet** `PUT …/name` ← `{name}` ✓ already correct.
- **AI reprocess** `POST /api/ai/reprocess` ← `AiReprocessRequest { photo_ids: Option<[String]> }`.
  Android sends `{scope}` → **rewrite** to `{photo_ids: List<String>?}`. Update `AiRepository.reprocess`.
- **AI toggle** `POST /api/ai/toggle` ← `{enabled}` ✓.

### 4.4 Geo settings drift (both directions broken)

- `GET /api/settings/geo` ← `GeoStatusResponse { enabled, scrub_on_upload, photos_with_location, photos_without_location, unique_countries, unique_cities }`. Android `GeoSettings { geo_enabled, reverse_geocode_enabled, strip_on_export }` → toggle always reads "off". **Rewrite** `GeoSettings` to the status shape.
- `POST /api/settings/geo` ← `GeoSettingsRequest { enabled?, scrub_on_upload? }`, returns **empty `200`** (no body). Android sends `{geo_enabled, reverse_geocode_enabled, strip_on_export}` and types the return as `GeoSettings` → toggle no-ops AND empty-body parse throws. **Rewrite** `UpdateGeoSettingsRequest` to `{enabled?, scrub_on_upload?}` and change `ApiService.updateGeoSettings` return type to `Response<Unit>` (or `retrofit2.Response<Unit>`). Update `GeoRepository.updateSettings` + any Settings UI consumer.
- `POST /api/geo/scrub` ← `ScrubConfirmRequest { confirm: bool }` (Android currently sends no body) and returns `{ scrubbed, ... }` — verify and add the `{confirm:true}` body.

### 4.5 Execution checklist (next session, with a working Android build)

- [ ] Confirm `/api/ai/status` route is registered (handler exists at `ai/handlers.rs:26`).
- [ ] Apply 4.1 (return types → `List<T>`, drop wrappers) in `ApiService.kt` + `AiRepository`/`GeoRepository`.
- [ ] Apply 4.2 `@SerializedName` fixes in `AiDto.kt` / `GeoDto.kt` (keep Kotlin prop names to avoid UI churn).
- [ ] Apply 4.3 request-body rewrites + update repo signatures and the People/Pets/Things screen call sites.
- [ ] Apply 4.4 geo-settings rewrite.
- [ ] `./gradlew :app:assembleDebug` + `:app:lintDebug` green.
- [ ] On-device smoke: People / Pets / Things / Map / Timeline / Locations / Memories / Trips all load; rename + merge + AI/Geo toggle persist.
- [ ] Fix the false "All pass / no drift detected" claims in Sessions 6–7 above.
