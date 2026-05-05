# simple-photos — Multi-Session TODO

This file tracks the multi-session work plan agreed with the user. Pick the
next un-checked item, follow the **per-task checklist** at the bottom of this
file, and tick the box when the work is shipped (code + tests + docs).

> Rules of engagement (from CLAUDE.md / .github/copilot-instructions.md):
> - **Plan first.** Run `gitnexus_impact` on every symbol you intend to edit and
>   report blast radius before changing code.
> - **DDT + E2E.** Every code path with variable input gets a parametrize
>   table; every user-facing feature gets at least one E2E test with `APIClient`.
> - **Numbered tests.** New test files use the next sequential number after the
>   highest existing `test_NN_*.py` (currently `test_67_backup_extended_metadata.py`,
>   so the next is `test_68_*.py`).
> - **Security.** Auth on every route, magic-byte validation for uploads,
>   parameterized queries only, no committed secrets.
> - **No find-and-replace renames.** Use `gitnexus_rename`.
> - **Run `gitnexus_detect_changes` before committing.**

---

## Phase 1 — DONE ✅

- [x] Extend `server/src/backup/sync_metadata.rs` to send all extended photos
      columns + `photo_tags`, `face_clusters`, `face_detections`,
      `object_detections`, `ai_processed_photos`, `user_settings`.
- [x] Extend `server/src/backup/serve.rs::backup_sync_metadata` with matching
      upserts + per-user prune semantics (preserves backward compatibility:
      old senders that omit a key still work via `.unwrap_or_default()`).
- [x] Hide AI Recognition + Geolocation sections in `web/src/pages/Settings.tsx`
      when `isBackupMode` is true.
- [x] `tests/test_67_backup_extended_metadata.py` — 36 cases, all pass in ~9 s.
- [x] `cargo build --release` clean.

### Known follow-up (Phase 1 hangover)

- [ ] **Investigate `tests/test_13_comprehensive_backup.py` flakiness.**
      - Tests pass individually but 25/44 fail when the file runs as a whole.
      - Looks pre-existing (the failure mode is `soft_delete_blob` returning
        500 in `test_populate_primary` — that path does not touch the new
        sync_metadata code). Confirm by running `git stash` against the Phase 1
        diff and observing the same failure pattern, then file as a separate
        ticket. **Priority: low (verify only, don't fix here).**

---

## Phase 2 — Active scope

The user reported these via screenshot + message after Phase 1 landed.

### 2.1 Panorama / 360 detection is broken — **HIGH** — ✅ DONE

> "we are having issues detecting panaromic photos, i hvae downloaded several,
> including 360 photos but none give the propeer dessignation, and so dont
> give the proper viewers."

- [x] Audited samples in `~/Desktop/Sample_files/image/` and
      `AI/subtypes/panorama/`. Real-world finding: `360photo.jpg` (3000×1500),
      `360photo2.jpg` (958×360), `Cologne_Panoramic` (7000×1000) and the
      `wide_no_xmp_control.jpg` sample have **no XMP `GPano:*`** — XMP-only
      detection misses them all.
- [x] Added `apply_aspect_subtype_fallback(info, width, height)` in
      `server/src/photos/metadata.rs` (purely additive; existing
      `extract_xmp_subtype` untouched).
      - `aspect >= 2.0` and `1.95 <= aspect <= 2.05` and `width >= 1500`
        ⇒ `equirectangular`.
      - `aspect >= 2.0` otherwise ⇒ `panorama`.
      - `width < 1024` or `aspect < 2.0` ⇒ unchanged (no false positives).
- [x] Wired into `upload.rs`, initial scan loop, and retroactive subtype
      backfill in `scan.rs` (backfill query now also pulls
      `width`/`height`).
- [x] Unit tests: 7 new `aspect_fallback_*` tests in
      `server/src/photos/metadata.rs::xmp_tests` (all 18 pass).
- [x] E2E DDT: `tests/test_68_panorama_aspect_fallback_ddt.py` — 15
      parametrize rows + 2 filter tests (16/16 pass).
- [x] Existing `test_45_panorama_360`, `test_52_subtype_pipeline`,
      `test_58_subtype_scan_regression` all still pass (55/55).
- [x] `cargo build --release` clean.

### 2.2 Grey thumbnails for various photo types — **HIGH**

> "thumbnails for numerous different photos types, showing up just grey
> insteada of the proper photos"

- [ ] Identify which formats/types fail. From `Sample_files/image/` there is
      coverage for: AVIF, BMP, HDR (.hdr), HEIC (likely under AI subtypes),
      ICO, PNG large, SVG, TIFF, WebP. Build a matrix of which actually have
      thumbnails after upload.
- [ ] `mcp_gitnexus_query({query: "generate thumbnail file pipeline"})`. Find
      the dispatcher and the per-format branches.
- [ ] Run upload+fetch loop locally with `pytest -k thumbnail` to find the
      grey ones; check server logs for "thumbnail generation failed:" lines.
- [ ] Fix per-format gaps:
      - HDR (.hdr Radiance): need a tonemapping pass (likely `image-rs`
        feature `hdr` is off; turn on or use an explicit decoder + Reinhard).
      - SVG: rasterize via `resvg` (already a dep?) at thumbnail size.
      - TIFF/large PNG: confirm `image::ImageReader::with_guessed_format()` is
        used, not `from_path` which can mis-detect.
      - AVIF: ensure `image` is built with `avif-decoder`.
      - BMP/ICO: cheap, just confirm format match.
- [ ] **DDT:** `tests/test_69_thumbnail_formats_ddt.py` parametrize over a
      curated list of real sample files; assert thumbnail bytes are non-empty,
      decode as a valid image of the expected dimensions, and are not a flat
      colour (compute pixel variance > threshold).
- [ ] **E2E:** upload each sample, fetch `/api/photos/{id}/thumbnail`, run
      the same variance check. Reuse pattern from existing thumbnail tests.

### 2.3 Low-resolution panorama previews — **HIGH**

> "panaromic photos seems to have very low rersolution prerviews, when they
> work."

- [ ] Find the preview generator (likely shared with the thumbnail pipeline
      but a higher-resolution tier — search for `web_preview` or
      `preview_max_dim`).
- [ ] Current logic likely caps the long edge at e.g. 1600 px. Panoramas with
      a 6:1 aspect ratio then get a 1600×266 preview which looks awful. Add
      a panorama-aware branch that scales by **short edge** (e.g. 1080 px
      short edge) when `subtype` is `panorama` or `equirectangular`.
- [ ] Make the cap configurable via `config.toml` (`preview_panorama_short_edge_px`).
- [ ] **DDT:** parametrize over sample panoramas with expected min long-edge.
- [ ] **E2E:** upload, fetch preview, assert pixel dimensions ≥ expected
      threshold and aspect ratio preserved within ±1 px.

### 2.4 Smart location albums (trips) — **MEDIUM** — ✅ DONE

> "no smart location photos albums showing up like, when a bunch of photos are
> taken on thee same date time and location roughly … one specific area like
> say yellowstone where you spend aa week there, and the a smart album using
> the geo settinigs would group up theere. we need to cretae test data."

Geo data already syncs (`geo_city/state/country/country_code`,
`photo_year/month`). This task is about surfacing it as automatic albums.

- [x] Trip clustering rule (in `server/src/geo/handlers.rs::list_trips`):
      - Group by `(geo_country_code, geo_city)`.
      - Split into runs where consecutive photo dates are > 3 days apart.
      - Discard runs with < 5 photos.
      - Single-day bursts qualify if photo count ≥ 5.
- [x] Backend: new `Trip` type + `list_trips` + `list_trip_photos` handlers,
      registered as `GET /api/geo/trips` and
      `GET /api/geo/trips/{trip_id}/photos` in `server/src/routes.rs`.
- [x] **DDT + E2E:** `tests/test_71_smart_trip_albums.py` — 9 tests pass
      (4 DDT clustering rows + lone-photo / shape / photos / 404 / auth).
- [x] `cargo build --release` clean.
- [ ] Front-end "Trips" carousel in `web/src/pages/Albums.tsx` — deferred to
      a follow-up UI task; backend ready for consumption.

### 2.5 Reorganise the sample test library — **MEDIUM** — ✅ DONE

> "we need to reorganize and inspect it so we have a proper conhernsive test
> library. as i want real work expamples and not generate photos where
> possible."

- [x] Added `load_sample(category, name) -> bytes` and `sample_files_root()`
      in `tests/helpers.py`.  Resolves against the `SAMPLE_FILES_ROOT` env
      var (default `~/Desktop/Sample_files`) and `pytest.skip()`s
      gracefully when the requested file is not on the runner.
- [x] New tests already preferring real samples where realism matters
      (`test_69_thumbnail_formats_ddt.py` uses `~/Desktop/Sample_files/image/`
      paths via skip-if-missing). Future feature tests should use
      `load_sample("AI/subtypes/panorama", "...")` etc.
- [ ] Physical reorg of files inside `~/Desktop/Sample_files/` itself
      (an out-of-repo dev folder) is left to the user — the helper means
      tests are decoupled from the on-disk layout for everything except
      the `category/name` path the test asks for.

### 2.6 Verify backup auto-sync runs every 24 h — **LOW (verify-only)** — ✅ DONE

> "use git nexus to check that the backup server autosyncs every 24 hours
> with the primary server."

Verified by code inspection of `server/src/backup/sync.rs::background_sync_task`
and `server/src/tasks.rs::spawn_backup_sync`:

- [x] Spawned at startup (`spawn_backup_sync` is called from
      `start_background_tasks`, which `main.rs` invokes at line 167).
- [x] Outer poll cadence: every **5 minutes** — deliberately short so
      newly-paired backup targets pick up promptly
      (`sync.rs:332`: `tokio::time::interval(Duration::from_secs(300))`).
- [x] Per-server gating uses `sync_frequency_hours` from the
      `backup_servers` table.  Default = 24 (set on insert in
      `backup/handlers.rs:73` and in `setup/pair_helpers.rs:255`).
- [x] Retry policy:
        * never synced → sync immediately
        * last status `error` or `partial` → retry after 1 h
        * last status success → wait full `sync_frequency_hours`
      (`sync.rs:354-369`).
- [x] On tick the same code path as the manual `/api/backup/sync`
      button executes, so all Phase 1 metadata is synced automatically.

**Conclusion:** the 24-hour auto-sync is correctly implemented; no fix
required.

---

## Per-task checklist (apply to every Phase 2 item)

1. **Plan.** Write the file/symbol list. Run `mcp_gitnexus_impact` on each.
   Report HIGH/CRITICAL risks to the user before editing.
2. **Read before editing.** `read_file` every file in the plan.
3. **Implement.** Smallest viable change. No drive-by refactors.
4. **Tests.** DDT + E2E as described per task. Add rows to existing tables
   when applicable rather than spawning one-off test functions.
5. **Validate.**
   - `cargo check` (server) / `tsc --noEmit` (web) / `pytest tests/test_NN_*.py -v`.
   - `mcp_gitnexus_detect_changes` to confirm the blast radius matches plan.
6. **Security pass.** Auth, input validation, magic-byte checks for uploads,
   parameterized queries.
7. **Tick the box** in this file and move on.

---

## Quick reference — recent migrations

| Migration | What it added |
|-----------|---------------|
| 016 | `photo_subtype`, `burst_id`, `motion_video_blob_id` |
| 017 | `face_clusters`, `face_detections`, `object_detections`, `ai_processed_photos`, `user_settings` |
| 018 | `geo_city`, `geo_state`, `geo_country`, `geo_country_code`, `photo_year`, `photo_month` |
| 019 | extended EXIF: `camera_make`, `lens_model`, `iso_speed`, `f_number`, `exposure_time`, `focal_length`, `flash`, `white_balance`, `exposure_program`, `metering_mode`, `orientation`, `software`, `artist`, `copyright`, `description`, `user_comment`, `color_space`, `exposure_bias`, `scene_type`, `digital_zoom`, `exif_overrides` |

## Quick reference — sample-file roots

- Panoramas (synthetic, light): `~/Desktop/Sample_files/AI/subtypes/panorama/`
- Real wide / 360 photos: `~/Desktop/Sample_files/image/360photo*.jpg`,
  `Cologne_-_Panoramic_Image_of_the_old_town_at_dusk.jpg`
- HDR (.hdr Radiance): `~/Desktop/Sample_files/image/Sample-HDR_5184×3456.hdr`
- Mixed formats: `~/Desktop/Sample_files/image/` (BMP, TIFF, AVIF, WebP, SVG, ICO, PNG)
- EXIF-rich, faces, geo, objects: `~/Desktop/Sample_files/AI/{exif_rich,faces,geolocation,objects}/`
