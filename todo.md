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

### P0-1 `[x]` Audio is uploaded even when `audio_backup_enabled = false` — fixed in 44b4e33
- Shared helper `crate::photos::utils::audio_backup_enabled` introduced.
- `upload.rs` now returns 403 for audio when toggle is off.
- `sync_engine.rs` filters `media_type='audio'` from the candidate query.
- All four pre-existing call sites refactored onto the helper.
- Regression `tests/test_63_audio_toggle_ddt.py` (5 DDT rows) verified to *fail* on pre-fix binary and *pass* after — i.e. it actually catches the bug.

### P0-2 `[x]` AI face/object detection silently runs in heuristic mode — fixed in 3d3ad89
- New `AiConfig.allow_heuristic_fallback` (default **false**): production deployments must install ONNX models; heuristic fallbacks no longer emit fake AI output unless an operator explicitly opts in.
- `face::detect_faces_from_image` and `object::detect_objects_with_quality` now take that flag; when no model is loaded and the flag is false they return `Ok(vec![])` instead of synthesising skin-tone / colour-histogram detections.
- `processor.rs` startup now logs `error!` (was `info!`) when no models are present and the flag is off, with explicit instructions to run `scripts/fetch_ai_models.sh`.
- `GET /api/ai/status` now exposes `face_model_loaded`, `object_model_loaded`, `degraded_mode`, and `allow_heuristic_fallback` so dashboards / admins can detect the degraded state.
- Regression `tests/test_64_ai_models_required.py` (6 cases) locks the contract; `tests/test_51_ai_cpu_pipeline.py` heuristic-dependent assertions now skip cleanly in degraded mode rather than passing on synthesised fake detections.

### P0-3 `[x]` Face clustering does not actually use embeddings — verified, not actually broken (7c4a105)
- Audited `server/src/ai/clustering.rs` end-to-end: `cluster_faces` already builds a pairwise cosine-similarity matrix from the stored embedding vectors, sorts by similarity descending, and merges via union-find when similarity ≥ threshold.  No detection-id timing, no naïve linking — the embeddings ARE the gate.
- Added two regression tests:
  - `test_cluster_uses_cosine_similarity_512d` builds a realistic L2-normalised 512-d ArcFace-shaped vector, a near-twin (cos sim > 0.95), and an orthogonal unrelated vector (cos sim < 0.5); asserts the first two cluster together and the third does not.
  - `test_threshold_is_respected` builds two vectors with cos sim = 0.6 and asserts threshold=0.7 separates them while threshold=0.5 merges them.
- The audit overstated this risk; tests now make regression impossible.

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

### P0-5 `[x]` Object detection writes hardcoded fake confidences — fixed in 3d3ad89
- The heuristic scene classifier (`detect_scenes_heuristic`) now only runs when `allow_heuristic_fallback=true` OR the ONNX classifier already produced real detections (in which case it merely supplements with scene attributes the ImageNet classifier doesn't cover).
- In the production-default config the heuristic confidences (`0.3 * blue_ratio`, etc.) cannot reach the API.
- Covered by the same `tests/test_64_ai_models_required.py` regression as P0-2.

### P0-6 `[x]` Photo subtype detection is brittle (string-search XMP) — fixed in c36c70a
- Motion-photo detection now uses the same tolerant `extract_xmp_str_attr` helper that burst/panorama already used, so single-quote and non-`"1"` values are recognised.
- Removed the duplicate copy-pasted `MotionPhoto="1"` substring checks.
- 11 new Rust unit tests in `photos::metadata::xmp_tests` cover both quote styles, lower-case namespace prefixes, `'0'`/`'false'` rejection, and the legacy MicroVideo schema. All pass.

### P0-7 `[x]` Burst stacking — verified, not actually broken
- **Verified server**: `server/src/photos/handlers.rs` already projects `burst_count` (subquery `COUNT(*) FROM photos WHERE burst_id = ...`) when `collapse_bursts=true`, and includes it as `NULL` for non-burst rows.
- **Verified web**: `web/src/gallery/components/ThumbnailTile.tsx:162` already renders the `{burstCount}` badge when `burstCount > 1`. `Gallery.tsx` consumes it via `_burstCount`.
- **Verified tests**: `tests/test_47_burst.py::TestBurstCollapse` already asserts `burst_count == 3` for a 3-frame burst and `null` for normal photos. All pass.
- The audit overstated this: the user-visible feature works end-to-end. Click-to-expand UX polish is a P2/P3 enhancement, not a P0 bug.

### P0-8 `[x]` Burst grouping has two competing strategies that can disagree — fixed in 607eb01
- Verified the timestamp-based detector already filters `WHERE burst_id IS NULL AND photo_subtype IS NULL` on its candidate `SELECT` and re-checks `burst_id IS NULL` on the `UPDATE`, so XMP-derived burst_ids are never overwritten.
- Added a module-doc block to `burst.rs` documenting this precedence.
- New `tests/test_47_burst.py::TestBurstDetectionPrecedence` (2 tests) uploads XMP-tagged frames, calls `POST /api/photos/detect-bursts`, and asserts the XMP burst_id survives and unrelated photos are not pulled in.

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
