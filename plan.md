# Post-Stable Stabilization Plan

> Baseline: commit `cc8183b` ("STABLE RELEASE!", 2026-04-17).
> Scope: every change between `cc8183b` and `HEAD` (`d448bb0`) plus all CI/CD
> security findings currently flagged. Goal: get the tree back to a known-good,
> security-clean state before tagging the next release.

## Stabilization status (2026-04-30, branch `stabilize/post-stable`)

**Done — Phase 1 P0 security fixes:**
- SMB token validation tightened; `unsafe libc` calls replaced with `rustix` safe wrappers; credentials file cleaned up on mount failure; condensed errors now redact `password=`/`pass=`/`pwd=`/`credentials=` fragments before leaving the server.
- EXIF write path: rejects `@`-prefix exiftool file-include; clear "exiftool not found" error; dynamic SQL replaced with compile-time `concat!()` (silences Semgrep `format-sql-string`).
- Storage browser: `/proc`, `/sys`, `/dev` blocklist; canonical path re-checked after symlink resolution; audit logging on blocked attempts.
- Web deps: vite ^5 → ^7 — `npm audit` clean (vite/esbuild/postcss advisories resolved).
- Server deps: rustls-webpki 0.103.11 → 0.103.13 (RUSTSEC-2026-0098/-0099/-0104); remaining advisories are tracked in `deny.toml`.
- AI: NaN-safe sort in `clustering.rs` and `face.rs`.

**Done — Phase 2 P1 correctness:**
- Renamed duplicate-numbered tests: 38 → 59 (ai_accuracy), 44 → 60 (motion_photo), 52 → 61 (geolocation_ddt).
- Fixed export bug: `album_manifest` blobs now ship as readable JSON under `metadata/` in the export zip (was being dropped because envelope extraction failed for non-media JSON).
- Fixed unused-variable lint in `editing/render_download.rs`.

**Done — Phase 3 cleanup:**
- `cargo clippy --fix` applied across server (94 → 42 warnings).

**Verified test sweeps (against `target/release/simple-photos-server`):**
- test_01: 23 / test_02-04: 65 / test_05-12: 148 / test_15-27+99: 25 / test_28-30: 36
- test_31-36: 111 / test_37: 11 / test_38_ddt+40-43: 167 / test_44+45-47+57-61: 132 / test_50-56: 156 / test_55: 23

**Commits on branch:**
- `3af26d7` plan
- `f28ef82` Phase 1 security pass (SMB, EXIF, browse, NaN sort, web deps, clippy)
- `5c94717` export album_manifest fix
- `52046d6` rustls-webpki bump
- `7d3462b` compile-time SQL concat
- `99faed7` SMB error redaction

**Deferred to a follow-up release:**
- Phase 1.1 HTTPS-only middleware for `/admin/storage*` (requires axum middleware refactor).
- Phase 2.2 AI subsystem split (`face.rs` 1027 LOC) — large refactor, no functional bugs found in the sweep.
- Phase 3 remaining ~40 clippy stylistic warnings (very-complex types, too-many-args).

## Status snapshot (recorded 2026-04-30)

| Surface | Indicator | Result |
|---|---|---|
| Web deps (`web/`) | `npm audit` | 3 moderate (esbuild GHSA-67mh-4wv8-2f99, postcss GHSA-qx2v-qp2m-jg93, vite path-traversal GHSA-4w7w-66w2-5vf9) |
| Server deps (`server/`) | `cargo audit` | not yet run locally (binary not installed); CI pipeline runs it |
| Server lints | `cargo clippy --no-deps` | 94 warnings (24 needless `Ok(?)`, 22 very-complex types, 6 needless `return`, 4 too-many-args, etc.) |
| Server unsafe | `cargo geiger` | one `unsafe` block in [server/src/setup/smb.rs](server/src/setup/smb.rs#L417) (geteuid/getegid) — documented but should be revisited |
| Tests | `pytest --collect-only` | duplicated test numbers: [test_38_ai_accuracy.py](tests/test_38_ai_accuracy.py) vs [test_38_edit_dimensions_ddt.py](tests/test_38_edit_dimensions_ddt.py); [test_44_motion_photo.py](tests/test_44_motion_photo.py) vs [test_44_tag_search_regression.py](tests/test_44_tag_search_regression.py); [test_52_geolocation_ddt.py](tests/test_52_geolocation_ddt.py) vs [test_52_subtype_pipeline.py](tests/test_52_subtype_pipeline.py) |
| GitNexus index | `gitnexus_list_repos` | 2 commits behind HEAD — must `npx gitnexus analyze` before refactoring |

## Commit history under review

| Commit | Date | Headline | Notes |
|---|---|---|---|
| `8a49200` | 2026-04-18 | "Full feature implimenttation" | Introduced AI module, geo albums, photo subtypes (motion/panorama/burst/HDR), slideshow, extended config |
| `fcea234` | 2026-04-19 | "Bug fixessss" | Major rewrites of AI face/object/processor, metadata_edit expansion, burst, panorama/motion overlays, AlbumDetail/Albums refactor, deleted TODO trackers |
| `d448bb0` | 2026-04-20 | "Some AI fixes" | Heavy face/object rewrite, ImageNet labels, AI accuracy & subtype regression tests added |
| `394c4a1` | 2026-04-30 | GitNexus skills + placeholder Takeout | Doc / tooling only |
| `0d8ff49` | 2026-04-30 | SMB/CIFS support | New `setup/smb.rs` (678 lines), shells out to `mount.cifs`/`sudo`, persists encrypted credentials |

## Modules touched since stable (high → low priority)

1. **`server/src/setup/smb.rs` (new, 678 LOC)** — privileged mounting, credential handling.
2. **`server/src/ai/**`** — `face.rs`, `object.rs`, `processor.rs`, `engine.rs`, `clustering.rs`, `models.rs`, `handlers.rs`, `imagenet_labels.rs` — heavy churn over three commits.
3. **`server/src/photos/metadata_edit.rs`** — `+328`/`+55` LOC, now writes EXIF via `exiftool` shell-out with user-supplied tag overrides.
4. **`server/src/photos/burst.rs` / `serve.rs` / `scan.rs`** — burst grouping + motion video serving.
5. **`server/src/setup/storage.rs`** — `+358` LOC, adds `test_smb`, `browse_directory`, SMB branch in `update_storage`.
6. **`server/src/geo/handlers.rs` / `processor.rs`** — geolocation album rebuild + handlers (`+133`, `+39`).
7. **`server/src/transcode/gpu_probe.rs`** — GPU detection changes.
8. **`server/src/conversion.rs`** — additional formats.
9. **`web/src/pages/welcome/ServerConfigStep.tsx`** — adds SMB credential modal (UI-side credential collection).
10. **`web/src/components/viewer/{PanoramaViewer,MotionVideoOverlay,BurstStrip,Slideshow,SlideshowTransitions}.tsx`** — all new.
11. **`web/src/components/settings/{AiRecognition,Geolocation,Transcode}Section.tsx`** — all new.
12. **`web/src/pages/{Albums,AlbumDetail,Viewer,Gallery,Settings}.tsx`** — large rewrites (AlbumDetail `+666`/Albums `+156`).
13. **`web/src/api/{ai,geo,metadata,transcode,admin,photos}.ts`** — new API surfaces.
14. **`web/src/hooks/useSlideshow.ts`** — new (218 LOC).
15. **Migrations 016–019** — `photo_subtype`, `ai_recognition`, `geolocation_albums`, `extended_exif_metadata`.

## Phase 0 — Prep & gating (must do first)

- [ ] Run `npx gitnexus analyze` so impact analysis reflects HEAD.
- [ ] Install `cargo-audit` + `cargo-deny` locally (mirrors CI) and capture a clean baseline.
- [ ] Pull the latest CI run artefacts (`report-secret-scanning`, `report-semgrep`, `report-rust-audit`, `report-rust-deny`, `report-rust-clippy`, `report-rust-unsafe`, `report-web-audit`, `report-web-lint`, `report-android-audit`, `report-docker-scan`, `report-dockerfile-lint`) and stash them under `benchmark_results/security/<run-id>/` so we can diff after each fix.
- [ ] Create a stabilization branch off `HEAD` (e.g. `stabilize/post-stable`) — every fix below lands here.

## Phase 1 — Security fixes (P0, before anything else)

For every change in this phase: run `gitnexus_impact({target, direction:"upstream"})` first, fix, then `gitnexus_detect_changes()`, then add a regression test.

### 1.1 SMB / CIFS subsystem ([server/src/setup/smb.rs](server/src/setup/smb.rs))

- [ ] **Confirm credential transport is HTTPS-only.** `configureSmbStorage` in [web/src/api/admin.ts](web/src/api/admin.ts#L99) sends `password` in JSON. Add a server-side guard that refuses `/admin/storage` (and `/admin/storage/test-smb`) over plaintext HTTP when `auth.require_https` is on, and surface the requirement in the wizard before the password modal opens.
- [ ] **Audit `condense_smb_error`** — it forwards raw `mount.cifs` / `smbclient` stderr to the API response. Verify it never echoes the password back (smbclient sometimes prints the URL with creds).
- [ ] **Argument injection.** `validate_token` allows `\` and space — fine for `mount.cifs -o credentials=…` (we never splice tokens into a shell), but tighten anyway: reject embedded `=`, `,`, and leading `-` (mount option separators / option-like values).
- [ ] **`run_mount_command` sudo fallback** ([smb.rs#L460](server/src/setup/smb.rs#L460)) shells out to `sudo -n mount.cifs`. Document the exact `sudoers` rule we recommend (NOPASSWD for `/usr/sbin/mount.cifs` only, with `Defaults!… !requiretty`), and add a runtime warning when SUID is missing AND sudo is being used.
- [ ] **Credentials file lifecycle.** `write_credentials_file` writes 0600 then leaves the file behind on mount failure — add an on-error cleanup path so failed wizard attempts don't litter `data/smb-creds/`.
- [ ] **`unsafe` block** ([smb.rs#L417](server/src/setup/smb.rs#L417)) — replace with the `nix` crate's safe `geteuid()`/`getegid()` to eliminate the only `unsafe` block in the new code.
- [ ] **Replace `.unwrap()` calls in tests** ([smb.rs#L612-L676](server/src/setup/smb.rs#L612)) — fine in `#[cfg(test)]` blocks, but flagged by clippy. Re-check whether any leak into non-test code paths (currently 0 by inspection — keep it that way with a `#![deny(clippy::unwrap_used)]` on the module excluding `mod tests`).
- [ ] **Add fuzz seeds** for `parse_smb_input` covering UNC, percent-encoded creds, IPv6 brackets, and homoglyph host names.

### 1.2 EXIF write path ([server/src/photos/metadata_edit.rs](server/src/photos/metadata_edit.rs#L735))

- [ ] **`@`-prefix file inclusion in exiftool values.** `write_exif_fields_full` builds `-Tag=value` arguments via `Command::args` (no shell), but exiftool itself treats values starting with `@` as a file-read directive. Reject any override value that begins with `@` (or strip it) so an authenticated user can't trick the server into stuffing `/etc/passwd` into a `UserComment` tag.
- [ ] **Macro-built SQL `format!("UPDATE photos SET {} = ?1 …")`** ([metadata_edit.rs#L273](server/src/photos/metadata_edit.rs#L273)) — column names are hardcoded literals in the macro caller, so not exploitable today, but it trips Semgrep `rust.lang.security.audit.format-sql-string`. Replace with an `&'static str` lookup table or per-field `sqlx::query!(...)` to silence the rule and prevent future drift.
- [ ] **`exiftool` not found / panic-on-spawn.** The module uses `std::process::Command` (blocking) inside `spawn_blocking`; surface a clear `AppError::ServiceUnavailable` when the binary is missing instead of bubbling raw IO errors.

### 1.3 Admin storage browser ([server/src/setup/storage.rs#L493](server/src/setup/storage.rs#L493))

- [ ] **`browse_directory` allows traversal of the entire host filesystem** (only `..` segments are rejected — admins can still pass `/etc` directly). Constrain to a configurable allow-list of roots (`/`, the storage parent, plus mounts) and reject any other absolute path. Even though admin-only, this is sensitive on multi-tenant hosts.
- [ ] **Symlink escapes.** After `tokio::fs::canonicalize`, re-validate against the allow-list — currently a symlink under the storage root can point outside it.

### 1.4 Web dependency advisories (`web/`)

- [ ] **vite ≤ 6.4.1 path traversal (GHSA-4w7w-66w2-5vf9)** — bump to `vite@^7` (the breaking changes are mostly config-shape; verify our `vite.config.ts` and dev server proxies still work).
- [ ] **esbuild ≤ 0.24.2 dev-server CSRF (GHSA-67mh-4wv8-2f99)** — resolved automatically by the vite bump.
- [ ] **postcss < 8.5.10 XSS (GHSA-qx2v-qp2m-jg93)** — `npm audit fix` (non-breaking).
- [ ] Re-run `npm audit` and commit the resulting `package-lock.json`.

### 1.5 Server dependency advisories

- [ ] After `cargo audit` baseline is captured, triage every advisory; pin or upgrade in `server/Cargo.toml`. Don't `--ignore` anything without writing a justification into `server/deny.toml`.

### 1.6 Cross-cutting

- [ ] **Panic-free guarantee for new modules.** `grep -n '\.unwrap()\|\.expect('` in `server/src/ai/`, `server/src/setup/`, `server/src/photos/burst.rs`, `server/src/photos/metadata_edit.rs` returns 20 hits (excluding `#[cfg(test)]`). Replace each with `?`/`AppError`. Track in a sub-todo list.
- [ ] **Web `dangerouslySetInnerHTML` / URL injection sweep.** Confirm none of the new viewer components (`PanoramaViewer`, `MotionVideoOverlay`, `Slideshow`, `SlideshowTransitions`, `BurstStrip`) inject user-controlled URLs into `<img src>` / `<video src>` without going through the `mediaUrl(photoId)` helper. Spot check looks clean — make it a Semgrep rule.
- [ ] **CSP review.** New features pull more remote-ish surfaces (panorama via `<img>` + canvas, motion video, slideshow). Re-check the `Content-Security-Policy` header in [server/src/security.rs](server/src/security.rs) — `media-src 'self' blob:` and `img-src 'self' blob: data:` should still be strict; no `unsafe-eval` should sneak in for the panorama lib.

## Phase 2 — Bug fixes & correctness (P1)

### 2.1 Test suite hygiene

- [ ] Resolve duplicate test numbers — rename so each `test_NN_*.py` is unique:
  - `test_38_ai_accuracy.py` ↔ `test_38_edit_dimensions_ddt.py`
  - `test_44_motion_photo.py` ↔ `test_44_tag_search_regression.py`
  - `test_52_geolocation_ddt.py` ↔ `test_52_subtype_pipeline.py`
- [ ] Run the full suite (`pytest tests/ -x`) and capture failures. Fix or quarantine each with a tracking entry in this plan.
- [ ] Re-enable any tests that were `@pytest.mark.skip`ped during the feature push.

### 2.2 AI subsystem ([server/src/ai/](server/src/ai/))

- [ ] **`face.rs` (1027 LOC, 430 LOC of changes in fcea234 alone)** — cyclomatic complexity is high. Use `gitnexus_context({name:"detect_faces"})` and `gitnexus_impact` before refactoring; split into `detector`, `landmark`, `embedding`, `clustering` files.
- [ ] **`object.rs` ImageNet integration** — verify [imagenet_labels.rs](server/src/ai/imagenet_labels.rs) (1447 LOC) matches the model's class-index ordering exactly; add a unit test against a known checksum.
- [ ] **`processor.rs`** — heavy concurrency/queueing changes. Audit channel back-pressure & shutdown paths; ensure no `tokio::spawn` futures are detached without a `JoinSet`.
- [ ] **`tagging.rs`** — verify de-dup logic between auto-tags and user tags (regression candidate per [test_43_tag_search_ddt.py](tests/test_43_tag_search_ddt.py)).
- [ ] **`models.rs` model loading** — check that the new `ultraface-RFB-320.onnx` file is verified by SHA256 before load (mitigates supply-chain swap on the storage volume).
- [ ] **AI handlers authn/authz** — every `/ai/*` route in `routes.rs::ai_routes()` must be admin-or-owner gated. Document a matrix in [API_REFERENCE.md](API_REFERENCE.md).

### 2.3 Photo subtype pipeline (motion / panorama / burst / HDR)

- [ ] **`burst.rs`** — verify burst grouping doesn't merge across users (multi-user regression candidate). Tests: [test_47_burst.py](tests/test_47_burst.py), [test_58_subtype_scan_regression.py](tests/test_58_subtype_scan_regression.py).
- [ ] **`serve.rs::serve_motion_video`** — confirm path resolution can't be tricked into serving the still JPEG side of another user's motion photo.
- [ ] **Frontend overlays.** [BurstStrip.tsx](web/src/components/viewer/BurstStrip.tsx), [MotionVideoOverlay.tsx](web/src/components/viewer/MotionVideoOverlay.tsx), [PanoramaViewer.tsx](web/src/components/viewer/PanoramaViewer.tsx) — confirm they handle photos that *claim* to be a subtype but are missing the corresponding asset (orphan motion video, panorama with no XMP `UsePanoramaViewer`, etc.) without crashing the viewer.

### 2.4 Slideshow

- [ ] [useSlideshow.ts](web/src/hooks/useSlideshow.ts) (new, 218 LOC) — review timer/interval cleanup on unmount; confirm `pause on hidden tab` behaviour.
- [ ] [Slideshow.tsx](web/src/components/viewer/Slideshow.tsx) / [SlideshowTransitions.tsx](web/src/components/viewer/SlideshowTransitions.tsx) — accessibility (ARIA roles, reduced-motion support).
- [ ] Verify [test_57_slideshow_ddt.py](tests/test_57_slideshow_ddt.py) covers shuffle, loop, and end-of-album behaviour.

### 2.5 Albums overhaul

- [ ] [Albums.tsx](web/src/pages/Albums.tsx) `+156`/[AlbumDetail.tsx](web/src/pages/AlbumDetail.tsx) `+666` — large rewrites; check covered scenarios:
  - shared album permissions UI
  - smart album auto-rules (geolocation, AI tag)
  - secure album lock/unlock interactions with the new viewer overlays
- [ ] Cross-reference against the deleted `TODO-ai-geo-exif.md` and `TODO.md` — any item from those files that wasn't implemented needs to be re-tracked here.

### 2.6 Geolocation

- [ ] [geo/handlers.rs](server/src/geo/handlers.rs) — verify reverse-geocode rate-limiting and that we cache responses (so a single user can't burn through the upstream quota).
- [ ] Migration `018_geolocation_albums.sql` — back-fill plan for existing photos? Add a one-shot re-process task triggered after migration applies.
- [ ] [GeolocationSection.tsx](web/src/components/settings/GeolocationSection.tsx) (210 LOC) — admin-only sections should be hidden for non-admin users.

### 2.7 Metadata editor (extended EXIF)

- [ ] Migration `019_extended_exif_metadata.sql` — confirm rollback story; backfill from existing photos optional.
- [ ] [PhotoInfoPanel.tsx](web/src/components/viewer/PhotoInfoPanel.tsx) (`+503`) — make every editable field validated client-side (numeric ranges, ISO/aperture sane bounds) before submitting; round-trip tests in [test_55_metadata_exif_round_trip_ddt.py](tests/test_55_metadata_exif_round_trip_ddt.py).
- [ ] **Concurrent edits** — two tabs editing the same photo: pick a write-wins-and-warn strategy (return updated row + version field).

### 2.8 GPU transcode

- [ ] [gpu_probe.rs](server/src/transcode/gpu_probe.rs) — verify VAAPI/NVENC detection failures degrade to CPU instead of erroring at boot.
- [ ] Test coverage: [test_56_gpu_transcode_ddt.py](tests/test_56_gpu_transcode_ddt.py).

### 2.9 Setup wizard / SMB UI

- [ ] [ServerConfigStep.tsx](web/src/pages/welcome/ServerConfigStep.tsx) (`+224`) — verify the "address looks like SMB" detection doesn't treat plain `//mnt/foo` (POSIX absolute path with two slashes) as SMB; add a regression test.
- [ ] Show explicit `mount.cifs not found` / `cifs-utils missing` error to the user, with a copy-pastable apt/dnf install command.
- [ ] Mask the password field; never log the password from `configureSmbStorage` (current `console.error` paths look clean — keep them clean).

## Phase 3 — Code quality cleanups (P2)

These are non-blocking but should land before the next tag.

- [ ] Apply `cargo clippy --fix` for the 52 auto-fixable suggestions, then manually triage the remaining 42:
  - 24 needless `Ok(?)` patterns
  - 22 very-complex types → introduce `type` aliases (e.g. for the AI pipeline result tuples)
  - 6 needless `return` (mostly in `smb.rs`)
  - 4 `&PathBuf` → `&Path`
  - 4 functions with > 7 args → group into structs
  - 5 manual `RangeInclusive::contains` patterns
- [ ] Add `#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo)]` to `server/src/lib.rs` (or each module crate root) and let the AI/burst/SMB modules opt out for tests only.
- [ ] Front-end ESLint: enable `eslint-plugin-security` + `eslint-plugin-no-secrets` in [web/eslint.config.*]() so the same rules CI runs are also enforced locally.
- [ ] Update `.claude/skills/gitnexus/*` notes — the skill files were added in `394c4a1` but reference flows that may shift after Phase 2 refactors.

## Phase 4 — Verification & release gating

- [ ] Re-run the full CI security workflow on the stabilization branch; every artefact must be green or have a documented `noqa` reason in `deny.toml` / `.semgrepignore` / `.gitleaks.toml`.
- [ ] `pytest tests/ -x` clean, including duplicate-numbered tests once renamed.
- [ ] `cargo test --all-features` clean.
- [ ] `npm run build` + `npm run test` clean in `web/`.
- [ ] Manual smoke test against `docker-compose.yml` covering: register → upload → AI tag → motion photo viewing → panorama → slideshow → SMB-mounted storage → backup/restore.
- [ ] Tag `v-stable+stabilization-1` and update [README.md](README.md) "Recent Changes" section.

## Working rules for this stabilization

1. **Always run `gitnexus_impact` before editing a function** ([AGENTS.md](AGENTS.md)). Report risk; abort and ask if HIGH/CRITICAL.
2. **Never rename with find/replace** — use `gitnexus_rename`.
3. **Always run `gitnexus_detect_changes` before committing** — verifies blast radius matches intent.
4. **One commit per checkbox** where practical, message format: `fix(area): <one-line>` + body referencing this plan and the GitNexus risk score.
5. **No `unsafe`, no new `.unwrap()`** in production paths during stabilization. Tests excepted.
6. **No new dependencies** without `cargo deny` / `npm audit` review on the same branch.

## Open questions to resolve with the user

- [ ] Do we want to keep the Google Cast feature (mentioned in [README.md](README.md) but not visible in the diff under review)? If yes, where does it live?
- [ ] Should the SMB feature ship in this stabilization release, or be feature-flagged off by default until the security checklist (1.1) is complete?
- [ ] Are there CI failures we should pull from a specific run ID? (gh CLI isn't installed locally — once provided, attach to "Phase 0".)
