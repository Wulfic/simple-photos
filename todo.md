# Post-STABLE-RELEASE Cleanup TODO

> Baseline: commit `cc8183b` ("STABLE RELEASE!"). 24 commits of feature-mash were piled on top
> by an LLM-driven workflow without proper end-to-end validation. This file is the single
> source of truth for fixing the resulting mess.
>
> **Working rule:** before checking off any P0/P1 item, the fix must be verified against the
> *real running server* (native `:8080`, not a mocked client) AND a regression test must be
> added that would have *failed before the fix*. No more green-test theatre.

---

## How to use this file

1. Pick the highest-priority unchecked item.
2. Run `gitnexus_impact` on every symbol you intend to touch. Paste the blast radius into the PR.
3. Do the work. Add or strengthen the regression test. Run `pytest tests/test_NN_*.py -v`.
4. Run `gitnexus_detect_changes` before committing.
5. Tick the box, add a one-line "fixed in <commit-sha>" note, move on.

Status legend: `[ ]` open · `[~]` in progress · `[x]` done · `[!]` blocked / needs decision

---

## P0 — Functional regressions (user-visible bugs)

### P0-1 `[ ]` Audio is uploaded even when `audio_backup_enabled = false`
- **Symptom:** user disables audio backup, then drag-drops an MP3 / FLAC into the web UI → it imports anyway.
- **Root cause:** the toggle is only enforced in three of the *four* import paths.
  - Enforced: [server/src/photos/scan.rs L154-164](server/src/photos/scan.rs#L154-L164),
    [server/src/ingest.rs L118](server/src/ingest.rs#L118),
    [server/src/backup/autoscan.rs L355](server/src/backup/autoscan.rs#L355).
  - **NOT enforced:** [server/src/photos/upload.rs L133-136, L290-300](server/src/photos/upload.rs#L133-L300) — the multipart upload endpoint never reads the setting.
  - **NOT enforced:** [server/src/backup/sync_engine.rs L364](server/src/backup/sync_engine.rs#L364) — comment literally says "Sync ALL registered media — including audio."
- **Fix:**
  1. Extract a single helper `audio_backup_enabled(&pool) -> bool` (one query, one place).
  2. Call it in `upload.rs` *before* the DB INSERT and reject with `400 Disabled` if media_type=audio.
  3. Decide policy for `sync_engine.rs`: either filter or document that backup mirrors source-of-truth regardless. **Default: filter.** Update the comment.
- **Verify:** new DDT row in `test_18_media_conversion.py` (or a new `test_63_audio_toggle_ddt.py`) toggling the setting and asserting upload + sync both honor it.

### P0-2 `[ ]` AI face/object detection silently runs in heuristic mode when models are missing
- **Symptom:** "Face Detection" toggle is on, photos process, tags appear — but nothing was detected by an actual ML model. SCRFD/MobileNetV2 weren't loaded; we're returning skin-tone-blob and color-histogram fakery.
- **Evidence:**
  - [server/src/ai/engine.rs L60-110](server/src/ai/engine.rs#L60-L110) — model load failures only emit `tracing::info!`; the engine keeps serving requests.
  - [server/src/ai/face.rs](server/src/ai/face.rs) — `detect_faces_from_image()` falls back to skin-tone heuristic (Peer et al.) when no ONNX session.
  - [server/src/ai/object.rs L240-290](server/src/ai/object.rs#L240-L290) — `detect_scenes_heuristic()` always runs, returns hand-rolled confidences (`0.3 * blue_ratio` etc.).
  - [server/src/ai/engine.rs L67-70](server/src/ai/engine.rs#L67-L70) — `gpu_available: false` is hard-coded, never updated.
- **Fix:**
  1. AI status endpoint must expose `face_model_loaded`, `object_model_loaded`, `models_path` and `degraded_mode: bool`.
  2. Front-end AI panel must show a red banner when `degraded_mode` is true.
  3. Heuristic fallbacks should be gated behind a config flag (`config.ai.allow_heuristic_fallback`, default `false` in production).
  4. On startup, if AI is enabled and no models are present, log `error!` (not `info!`) and surface in `/api/health`.
- **Verify:** new test `test_64_ai_model_required.py` — start server with empty `server/models/`, toggle AI on, upload photo, assert response shows `degraded_mode=true` and *zero* detections (not heuristic ghosts).

### P0-3 `[ ]` Face clustering does not actually use embeddings
- **Symptom:** face clusters group photos by detection-id timing or naive ID-based linking — *not* by face similarity. Same person across photos lands in different clusters.
- **Evidence:** [server/src/ai/processor.rs L315-330](server/src/ai/processor.rs#L315-L330) writes embedding bytes; `link_detection_to_cluster()` ([server/src/ai/clustering.rs](server/src/ai/clustering.rs)) needs an audit — confirm it computes cosine similarity on the stored embedding vectors and not a fallback.
- **Fix:** read clustering.rs end-to-end, instrument with a unit test that feeds two near-identical 512-d vectors and a third unrelated vector → asserts the first two cluster together, the third does not.
- **Verify:** unit test in Rust + an E2E test that uploads two known-same-person photos (use the `tests/test_data/ai_faces/face_01_woman.jpg` files) and asserts they share a cluster_id.

### P0-4 `[ ]` Geolocation reverse-geocoding is silently disabled in default install
- **Symptom:** user uploads photos with GPS EXIF; "Locations" / "Map" pages stay empty forever.
- **Root causes:**
  1. [server/src/geo/processor.rs L30-48](server/src/geo/processor.rs#L30-L48) — if cities500.txt is missing, falls back to `ReverseGeocoder::empty()` with **no error**.
  2. cities500.txt is ~200 MB and not bundled / not downloaded by `install.sh`.
  3. [server/src/geo/processor.rs L63-65](server/src/geo/processor.rs#L63-L65) — first run waits a full 5-minute interval tick; users see nothing for 5 min after their first upload.
- **Fix:**
  1. `install.sh` and Dockerfile must download cities500.txt to a known path at install time (it's freely re-distributable from geonames.org).
  2. Server start: if AI enabled but dataset absent → `error!` log + `/api/health` warning.
  3. Geo processor: run once immediately on startup, then every 5 min. Use `tokio::select!` so it doesn't block shutdown.
  4. After upload of a photo with GPS EXIF, kick the processor with a one-shot signal (don't wait the full interval).
- **Verify:** new test `test_65_geo_backfill.py` — upload photo with GPS, sleep ≤ 30 s, assert `geo_city` is populated. Skip cleanly *with a real `pytest.fail`*, not `pytest.skip`, if dataset is absent.

### P0-5 `[ ]` Object detection writes hardcoded fake confidences
- **Symptom:** `object:` tags appear with confidence values that have no relationship to reality.
- **Evidence:** [server/src/ai/object.rs L275-290](server/src/ai/object.rs#L275-L290) — boat = `0.3 * blue_ratio`, plant = `0.4 * green_ratio`, etc.
- **Fix:** delete the heuristic block entirely once P0-2 is done, OR fence it behind `cfg(test)` only. Production must use MobileNetV2 or no result.
- **Verify:** see P0-2's test.

### P0-6 `[ ]` Photo subtype detection is brittle (string-search XMP)
- **Symptom:** burst / motion / panorama / HDR detection misses files when XMP uses single quotes, different namespace prefix, or non-UTF-8.
- **Evidence:** [server/src/photos/metadata.rs L536-600](server/src/photos/metadata.rs#L536-L600) — uses raw substring match on bytes for `MicroVideo="1"` etc.
- **Fix:** swap the substring scanner for a tolerant XMP attribute parser (one of: small hand-rolled XML walker, `quick-xml` crate). Must handle:
  - `'` and `"` quoting
  - Arbitrary namespace prefixes (`gcamera:`, `GCamera:`, `xmlns` aliases)
  - Multiple XMP packets (extended XMP)
- **Verify:** add DDT rows to `test_58_subtype_scan_regression.py` covering the three quirks above.

### P0-7 `[ ]` Burst stacking — `burst_count` not surfaced; UI has no expand affordance
- **Symptom:** gallery collapses bursts to one tile but user sees no "3 frames" badge and no obvious way to expand.
- **Evidence:**
  - Server: [server/src/photos/handlers.rs L53-77](server/src/photos/handlers.rs#L53-L77) — `collapse_bursts` query exists, but `burst_count` is not part of the row shape returned (verify what the SELECT actually projects).
  - Web: [web/src/pages/Gallery.tsx L174-184](web/src/pages/Gallery.tsx#L174-L184) — collapse done client-side, server collapse may be dead code.
- **Fix:**
  1. Decide *one* place to do collapse: server (preferred) OR client. Delete the other.
  2. Surface `burst_count` and `burst_cover_id` in the photos list response.
  3. Add a corner badge + click-to-expand interaction in `Gallery.tsx`.
- **Verify:** extend `test_47_burst.py` — assert gallery list includes `burst_count >= 2` for a 3-frame burst, and `/api/photos/burst/{id}` returns ordered frames.

### P0-8 `[ ]` Burst grouping has two competing strategies that can disagree
- **Symptom:** a photo with both a `GCamera:BurstID` XMP tag AND timestamp-proximity to other photos can end up in inconsistent groups depending on which detector ran last.
- **Evidence:**
  - XMP-based: [server/src/photos/metadata.rs L610-618](server/src/photos/metadata.rs#L610-L618) — sets `burst_id` immediately on upload/scan.
  - Timestamp-based: [server/src/photos/burst.rs](server/src/photos/burst.rs) — `detect_bursts_for_user()` runs async later, can overwrite/reassign.
- **Fix:** if `burst_id` is already set from XMP, the timestamp detector must skip that photo (`WHERE burst_id IS NULL`). Document the precedence in `burst.rs`.
- **Verify:** test that uploads a 3-frame XMP burst then runs the timestamp grouper; asserts XMP burst_id is preserved unchanged.

---

## P1 — Tests that pass while features are broken (test theatre)

### P1-1 `[ ]` `test_50_ai_recognition_ddt.py` — only checks field existence
- Lines like `assert field in status` and `assert status[field] >= 0` (line 244). Verifies the *shape* of JSON, not that AI did anything.
- **Fix:** convert to behavioral assertions: upload a known-face photo, poll until `face_detections > 0`, fail otherwise. If models aren't present, fail loudly.

### P1-2 `[ ]` `test_51_ai_cpu_pipeline.py` — passes on heuristic-only output
- Uploads green/skin-tone PIL-generated rectangles and asserts heuristic results. The "green → plant" assertion proves the heuristic ran, NOT that ML works.
- **Fix:** require real model files (download in `conftest.py`). Use the `tests/test_data/ai_*` real photos. Drop the heuristic-targeted assertions.

### P1-3 `[ ]` `test_53_geo_pipeline.py` — never validates reverse-geocoding
- Comment on line 14 admits: *"NOTE: The background geo-processor … runs on a 5-minute cycle and requires the GeoNames dataset. These tests focus on the upload path …"*. Translation: we don't test the feature.
- **Fix:** see P0-4. Once startup-kick is in place, write the assertion.

### P1-4 `[ ]` `test_59_ai_accuracy.py` — `pytest.skip()` on the only path that matters
- [tests/test_59_ai_accuracy.py L76](tests/test_59_ai_accuracy.py#L76) skips when `E2E_PRIMARY_URL` is set (which it always is in our CI/dev workflow).
- **Fix:** delete the skip. If models are missing, fail with a clear "AI accuracy tests require model files in server/models/ — run `scripts/fetch_ai_models.sh`" message. Add that fetch script.

### P1-5 `[ ]` `test_58_subtype_scan_regression.py` — `pytest.skip` if release binary missing
- [tests/test_58_subtype_scan_regression.py L281](tests/test_58_subtype_scan_regression.py#L281) skips when `target/release/simple-photos-server` doesn't exist. CI debug builds → silent green.
- **Fix:** test should use the same `conftest.py` server fixture as every other test. If a separate process is genuinely needed, build it on demand or fail.

### P1-6 `[ ]` `test_61_geolocation_ddt.py` — `>= 0` assertions
- Line 87: `assert settings[field] >= 0`. Counter is unsigned, this is meaningless.
- **Fix:** assert known-good values after deterministic input.

### P1-7 `[ ]` Audit every `pytest.skip` in `tests/`
- Run: `grep -nE "pytest\.skip|pytest\.xfail" tests/test_*.py`. For each: justify it in a comment with an issue link, or remove it.

### P1-8 `[ ]` Audit every `>=\s*0` assertion
- `grep -nE "assert.*>=\s*0" tests/test_*.py`. Most are placeholders. Replace with real bounds.

---

## P2 — Code bloat & dead code

### P2-1 `[ ]` `server/src/ai/imagenet_labels.rs` — 1447 lines of static data
- 1000-class ImageNet label list as Rust source. Compile time + binary size hit. Move to a static text file under `server/models/` and `include_str!` if needed, or load at runtime.

### P2-2 `[ ]` Heuristic fallback functions in `ai/face.rs`, `ai/object.rs`
- Once P0-2 makes models mandatory: delete `detect_scenes_heuristic`, skin-tone face fallback, and any other "looks-like" code. They exist only to keep tests green.
- Estimated removal: ~600 lines across face.rs (1279 lines!) and object.rs (705 lines).

### P2-3 `[ ]` `server/src/ai/face.rs` is 1279 lines — split it
- Should be: `face/detector.rs` (model + inference), `face/embedding.rs`, `face/clustering_link.rs`. Single-file gigants are how this mess started.

### P2-4 `[ ]` `server/src/photos/handlers.rs` (591) and `metadata_edit.rs` (863)
- Both have grown by hundreds of lines since stable. Audit for duplicated SQL UPDATE blocks (the [server/src/photos/scan.rs L455](server/src/photos/scan.rs#L455) `UPDATE photos SET photo_subtype = ?, burst_id = COALESCE(...)` pattern is repeated in upload.rs — extract a helper).

### P2-5 `[ ]` Diagnostics "stub when disabled" path
- [server/src/diagnostics/handlers.rs L274](server/src/diagnostics/handlers.rs#L274) — verify the disabled path actually returns sensible 404/503, not stub data that masquerades as real telemetry.

### P2-6 `[ ]` Remove `web/dist/` from grep noise
- Build artifacts pollute every search. Add `web/dist/**` to `.git/info/exclude` for local greps OR commit a `.gitattributes` `linguist-generated`. Right now `grep_search` matches minified bundles.

---

## P3 — Process / hygiene

### P3-1 `[ ]` Add a "do these features actually work" smoke test
- One file: `tests/test_99_smoke_real_features.py`. Boots server, uploads:
  - one photo with GPS → asserts `geo_city` filled within 30 s
  - one photo with a face → asserts `face_detections > 0`
  - one photo with a recognizable object → asserts `object:` tag present
  - one motion photo → asserts `motion_video_blob_id` set
  - one burst (3 frames) → asserts collapse to 1 with `burst_count=3`
- Anything red here = release blocker.

### P3-2 `[ ]` Add a `scripts/fetch_ai_models.sh` and `scripts/fetch_geo_data.sh`
- Plus matching steps in `install.sh` and the Dockerfile. These features cannot be marked "supported" while their data is a manual download nobody knows about.

### P3-3 `[ ]` README / API_REFERENCE accuracy pass
- README brags about face detection, HDR, motion photos, panoramas, Cast support. Once P0-1..P0-8 are done, walk through the README claims one by one and either back them up with a working demo or remove them.

### P3-4 `[ ]` Re-run `gitnexus analyze` after every batch
- The 11 661-symbol index is stale relative to the post-stable churn. Re-index before doing impact analysis on any item above.

---

## Tracking

| Priority | Open | Total |
|---------:|-----:|------:|
| P0       |    8 |     8 |
| P1       |    8 |     8 |
| P2       |    6 |     6 |
| P3       |    4 |     4 |
| **Total**|   26 |    26 |

Last updated: 2026-05-03 (initial draft after audit of cc8183b..HEAD).
