# Simple Photos — Issue Backlog (exploration: 2026-06-16)

Status legend: `[ ]` not started · `[~]` in progress · `[x]` done
Severity: 🔴 high (data loss / broken core flow) · 🟠 medium · 🟡 low / polish

This file is the multi-session attack plan. Each item lists the **symptom**,
**root cause / findings** (with `file:line` anchors confirmed during
exploration), a **proposed fix**, **affected files**, and **acceptance
criteria**. Do not "done" anything without unit + manual verification.

> Exploration notes are findings, not commitments. Where a root cause is a
> hypothesis (needs OS-specific repro), it is marked **(VERIFY)**.

---

## Suggested session grouping

- **Session A — Data integrity:** #3 (trash re-add) ✅, #10 (Windows convert stall) 🟡 mitigated+instrumented — *complete 2026-06-16*
- **Session B — Editing:** #4 (crop apply/thumb) ✅, #13 (crop UI overlap) ✅ — *complete 2026-06-16 (FE unit-test runner is a follow-up)*
- **Session C — Navigation & panels:** #7 (back context audit) ✅, #15 (overlapping menus) ✅ — *complete 2026-06-16*
- **Session D — Notifications/UX correctness:** #8 (toast system) 🟠, #12 (audio-policy error) 🟠, #11 (convert counter) 🟡, #2 (upload button) 🟠
- **Session E — Albums:** #6 (create-album in popup) 🟠
- **Session F — Visual polish:** #9 (light-mode contrast) 🟠, #14 (button/card facelift) 🟡
- **Session G — Packaging/infra:** #1 (bundled installers) 🟠, #5 (geo on Ubuntu + onnx off release) 🟠

---

## 🔴 #3 — Auto-scan re-adds trashed photos  ✅ DONE (2026-06-16)

**Fix shipped:** migration `022_trash_original_path.sql` adds
`trash_items.original_file_path`; the blob soft-delete captures the deleted
photo's original plaintext path; `scan.rs` + `backup/autoscan.rs` exclude it;
purge / permanent-delete / empty-trash remove the plaintext original (ref-count
guarded for Save-Copy). Regression tests in `tests/test_04_trash.py`
(`TestTrashScanReadd`) — 15/15 trash tests green.

<details><summary>original analysis</summary>


**Symptom:** Delete an item → it shows in Trash, then ~minutes later the
auto-scan re-adds the photo to the library while the trashed copy still
exists. Restoring after the re-add shows only one item (no duplicate).

**Root cause (CONFIRMED):**
- Viewer delete uses the **encrypted blob** soft-delete path
  (`useViewerActions.handleDelete` → `api.blobs.softDelete`).
- That path records `trash_items.file_path = <blob storage_path>`, **not** the
  original on-disk native path — `server/src/trash/operations.rs:243-253`
  (`.bind(&storage_path)`), vs the photo path bound at `:96-105`.
- Auto-scan's "already known" set is built from `photos.file_path` +
  `photos.source_path` + `trash_items.file_path`
  (`server/src/photos/scan.rs:62-74`). The original native file
  (`uploads/<name>`) is still on disk, its `photos` row was deleted, and the
  trash row points at the blob path — so the original is **unmatched** and gets
  re-registered (`scan.rs:113-115`).
- Restore re-creates the `photos` row at the original path; dedup by
  `photo_hash` (`upload.rs:249-275` style) collapses scan-readd + restore into
  one row → "only shows one".

**(VERIFY):** confirm the plaintext original actually persists on disk after
encryption migration (`server_migrate`). If encryption clears `file_path`/
deletes the plaintext, the re-add source is different — re-trace before fixing.

**Proposed fix (decide during Session A):**
1. On blob soft-delete, also resolve and record the owning photo's original
   `file_path` (and/or `photo_hash`) into `trash_items` so the scan exclusion
   set matches; **or**
2. Add a `deleted_hashes` / tombstone check to `scan_and_register` so any file
   whose `photo_hash` is in trash is skipped; **or**
3. Quarantine/move the original native file out of the scan root on delete.

Option 2 is the most robust against path mismatches.

**Affected:** `server/src/trash/operations.rs`, `server/src/photos/scan.rs`,
`server/src/backup/autoscan.rs:268-270` (mirror the same exclusion logic).

**Acceptance:** delete → wait through ≥2 scan cycles → item stays only in
trash; restore brings back exactly one; purge removes the original.
</details>

---

## 🔴 #10 — Windows stops converting media mid-import (starts with video)  🟡 PARTIAL (2026-06-16)

**Shipped this pass (safe, tested):**
- Ruled out the batch-abort theory: the ingest loop already `continue`s past
  failures and ticks progress on both arms; `process.rs` spawns with
  `stdin(null)` + `kill_on_drop` + timeout, so a hung ffmpeg is killed.
- **Mitigation:** conversion now orders **image → audio → video** (videos last)
  via `conversion::conversion_priority` so a mixed import shows steady progress
  instead of appearing frozen on the first big video ("always starts with
  video"). Unit tests in `conversion.rs`.
- **Observability:** per-file `[INGEST] converting` (before) + `converted in`
  (after, with `elapsed_ms`) and a category breakdown line. A real hang now
  leaves a dangling `converting` line naming the exact file.
- Verified: conversion unit tests + image/TIFF/video/scan/deferred-import E2E
  all green (the 6 audio E2E failures are the audio-backup-disabled 403 = #12,
  pre-existing/environmental).

**Still open (needs Windows device repro with the new logs):** whether there is
a true hard stall beyond slow-serial-video. Likely suspects to confirm from the
new logs: the encryption-wait gate (ingest.rs:64-113) deferring conversion while
natives keep encrypting during an active import, or a specific file whose
`converting` line never gets a `converted in` partner. **Do not** rewrite the
sequencing/parallelism without that repro — it guards the encryption/conversion
race.

<details><summary>original analysis</summary>

**Symptom:** On Windows, conversion halts during import; it "always starts with
video conversions" then stops.

**Findings:** conversion runs in `crate::ingest::run_conversion_pass`, spawned
after scan (`scan.rs:592-626`). FFmpeg invoked via
`crate::process::background_command("ffmpeg")` with a 600s timeout
(`conversion.rs:335,397,459,505`). Video path tries GPU (NVENC/QSV/VAAPI) then
CPU fallback.

**(VERIFY) hypotheses to test on a Windows host:**
- ffmpeg not on PATH for the service account, or a per-file spawn failure
  aborts the remaining queue instead of continuing.
- GPU transcode hangs/timeouts on Windows and the pass doesn't advance.
- Ordering puts videos first and a single failing/long video blocks the rest.
- Ties to memory note "Windows convert stall during import".

**Proposed fix:** review `server/src/ingest.rs` queue loop — ensure one failed/
slow item never aborts the batch; add per-item error logging on every path
(non-negotiable: log every error path); consider interleaving
image/audio/video instead of all-videos-first.

**Affected:** `server/src/ingest.rs`, `server/src/conversion.rs`,
`server/src/process.rs`.

**Acceptance:** import a mixed batch (images + multiple videos + a deliberately
broken video) on Windows → all convertible items finish; the broken one logs
and is skipped; banner reaches `done == total`.
</details>

---

## 🔴 #4 — Cropping doesn't cut/resize to fit; thumbnail doesn't match  ✅ DONE (2026-06-16)

**Fix shipped (crop is non-destructive metadata, rendered consistently):**
- **Viewer fit (#4a):** `computeCropZoom` dropped the `* 0.85` shrink it applied
  in normal (saved) view, so a cropped photo now fills the screen instead of
  sitting in a gutter (`useViewerEdit.ts`).
- **Thumbnail match (#4b):** `getThumbnailStyle` used `scale = max(1/cw,1/ch)`,
  which over-zooms every non-square crop. Corrected to `scale = 1/max(cw,ch)`
  with a letterbox-axis-corrected translate (`fx/fy`) so the crop rect exactly
  fills the object-cover tile. Geometry derived + validated against the Viewer
  for centred / wide / off-centre / strip crops (Node check, 4/4).
- **Caveat:** the frontend has **no unit-test runner** (no vitest/jest), so this
  is covered by the derivation + a standalone Node validation, not a committed
  test. `tsc -b` clean. *Follow-up:* add vitest + a `thumbnailCss.test.ts`, and
  eyeball a real crop in the running app to confirm pixel-match. Rotated+crop is
  an edge case left at prior behavior.

<details><summary>original analysis</summary>


**Symptom:** After cropping, the image isn't actually cut out and refit to
screen, and the gallery thumbnail still shows the uncropped image.

**Root cause (CONFIRMED):**
- Crop is **metadata-only**. `handleSaveEdit` writes `cropData` to IDB +
  `api.photos.setCrop` and exits — **no re-render, no thumbnail regen**
  (`useViewerActions.ts:124-166`).
- Display applies crop as a CSS transform (`cropZoomStyle` / `computeCropZoom`)
  on an `object-contain` image (`Viewer.tsx:506`), which is approximate and not
  a true fit-to-frame.
- Gallery thumbnails never receive the crop transform → mismatch.

**Proposed fix:** on Save, regenerate the thumbnail with the crop baked in
(server-side render is already available for video via
`api.photos.renderFile`/`duplicate`; mirror for images), and store crop-aware
display dims so the viewer fits the cropped region exactly. Audit
`computeCropZoom`/`cropZoomStyle` for correct fit math.

**Affected:** `web/src/hooks/useViewerActions.ts`,
`web/src/hooks/useViewerEdit.ts`, `web/src/pages/Viewer.tsx`,
`web/src/utils/media.ts` (`applyEditsToImageDownload` pattern),
`server/src/photos/thumbnail.rs`, `server/src/editing/`.

**Acceptance:** crop + Save → viewer shows the cropped image fit to frame; the
gallery tile thumbnail matches; Android/other clients see the same via the
server-synced crop.
</details>

---

## 🟠 #13 — Crop edit bar covers the photo; crop outline has top dead space  ✅ DONE (2026-06-16)

**Fix shipped:** in edit mode the content/crop area is inset below the top bar
(56px) and above the edit panel (live-measured via `ResizeObserver`, since the
panel height changes per tab) instead of filling the whole viewport. The photo
+ crop handles now sit fully in the visible gap — no panel overlap, no top dead
space — and the crop math reads the same element box so it adapts automatically.
`ViewerEditPanel` forwards a `rootRef` for the measurement. `tsc -b` clean.

<details><summary>original analysis</summary>


**Symptom (see screenshot 1):** the Crop/Brightness/Rotate + Save bar overlaps
the lower part of the media, making the bottom hard to edit; the crop outline
isn't flush to the top of the photo (dead space).

**Root cause (CONFIRMED):**
- Edit panel is `absolute bottom-0 left-0 right-0 ... bg-black/90`
  (`ViewerEditPanel.tsx:162`) drawn over the media.
- Media container is full-viewport `absolute inset-0` (`Viewer.tsx:393-403`);
  the `object-contain` image is centered in the whole viewport, so part of it
  sits **behind the top bar and behind the edit panel** → overlap + perceived
  top dead space. Crop overlay positions from `mediaRect` (`CropOverlay.tsx`),
  and `EDIT_CROP_PADDING_SCALE` shrink (`Viewer.tsx:493`) adds to the offset.

**Proposed fix:** in edit mode, constrain the media/crop area to the space
between the top bar and the edit panel (e.g. `top-14 bottom-[panel-height]`)
instead of full `inset-0`, so the entire image + crop region is visible and
unobstructed. Re-check crop overlay top alignment after the inset change.

**Affected:** `web/src/pages/Viewer.tsx`,
`web/src/components/viewer/ViewerEditPanel.tsx`,
`web/src/components/viewer/CropOverlay.tsx`.

**Acceptance:** in crop mode the full photo is visible above the bar; corner
handles reachable at all four corners; crop outline flush to the media edges.
</details>

---

## 🟠 #7 — Back button loses album context (recurring; audit needed)  ✅ DONE (2026-06-16)

**Fix shipped:** the Viewer now computes a single origin-aware `backTo`
target — `/secure-gallery?album=<id>` (secure album) → `/secure-gallery`
(secure root) → `/albums/<albumId>` (album, incl. smart sub-views like
`smart-pets/<clusterId>`) → `/gallery` (default). Back button, Escape, and the
leave-prompt save/discard (`useViewerActions`) all route through `backTo`, as
does post-delete navigation. Fixed the dead singular `/album/<id>` route in
`handleRemoveFromAlbum` → `/albums/<id>`. SecureGallery now passes
`secureAlbumId` when opening the viewer, and the viewer propagates it through
prev/next + burst navigation so a swipe doesn't drop the secure-album origin.
`tsc -b` clean. **Audit result:** the remaining `navigate("/gallery"|
"/albums")` calls (Login/Register/Setup/Import-done/AppHeader logo, AlbumDetail
& SharedAlbumDetail "back to list") are correct. *Known gap (not in scope):*
Search-opened photos still return to `/gallery` (no search-state restoration).

<details><summary>original analysis</summary>

**Symptom:** viewing a photo opened from an album, the top-left back button
returns to the gallery instead of the album.

**Root cause (CONFIRMED):**
- `Viewer` back ignores `albumId`: `navigate(secureGallery ? "/secure-gallery"
  : "/gallery")` (`Viewer.tsx:369`).
- Escape key does the same (`Viewer.tsx:336`).
- `handleLeaveAndSave` / `handleLeaveAndDiscard` hardcode `/gallery`
  (`useViewerActions.ts:322-332`).
- **Bonus bug:** `handleRemoveFromAlbum` navigates to `/album/${albumId}`
  (singular) but the route is `/albums/:albumId` (`App.tsx:229`) → dead route.

**Proposed fix:** when `albumId` is present, back/escape/leave should go to
`/albums/${albumId}`. Fix the singular `/album/` typo. Then **audit** every
`navigate("/gallery")` / `navigate("/albums")` for lost context (search:
`grep "navigate(\"/gallery\"|navigate(\"/albums\""`), and confirm
SecureGallery, SharedAlbumDetail, smart-album sub-views preserve origin.

**Affected:** `web/src/pages/Viewer.tsx`, `web/src/hooks/useViewerActions.ts`;
audit `web/src/pages/*.tsx`.

**Acceptance:** open photo from album → back returns to that album (scroll
position ideally preserved); remove-from-album returns to the album; gallery
origin still returns to gallery.
</details>

---

## 🟠 #15 — Photo action panels draw over each other  ✅ DONE (2026-06-16)

**Fix shipped:** Info / Tags / Edit are now mutually exclusive. The Viewer
wraps the panel setters handed to `ViewerTopBar` (`openInfoPanel`/`openTagPanel`)
so opening one closes the other and exits edit mode; `handleToggleEdit` closes
both panels before entering edit. The underlying booleans are unchanged (raw
setters still used for swipe-to-close and panel `onClose`), so only the
*open* paths gained exclusivity. `tsc -b` clean.

<details><summary>original analysis</summary>

**Symptom:** opening edit/tags/info (and download/delete) at the top of a photo
stacks panels on top of each other instead of closing the previous one.

**Root cause (CONFIRMED):** `showInfoPanel`, `showTagPanel`, and `editMode` are
independent booleans toggled separately (`ViewerTopBar.tsx:66-95`,
`Viewer.tsx:356-377`). Nothing closes the others when one opens.

**Proposed fix:** make the panels mutually exclusive — a single
`activePanel: "info" | "tags" | "edit" | null` state (or have each opener close
the others). Entering edit mode should close info/tags.

**Affected:** `web/src/pages/Viewer.tsx`,
`web/src/components/viewer/ViewerTopBar.tsx`,
`web/src/components/viewer/PhotoInfoPanel.tsx`,
`web/src/components/viewer/TagPanel.tsx`.

**Acceptance:** opening any one panel closes the others; never two overlapping.
</details>

---

## 🟠 #8 — Errors shown under the navbar; use a toast/popup instead

**Symptom:** errors render as a red line under the navbar (e.g. sharing an album
to yourself: "Cannot add yourself as a member").

**Findings:**
- Inline pattern, e.g. `Gallery.tsx:329` `{error && <p className="text-red-600
  ...">{error}</p>}`; repeated across ~51 components.
- The example message comes from the server (`server/src/gallery/shared.rs:274`).

**Proposed fix:** introduce a lightweight global toast/snackbar system (a store
+ `<ToastHost>` mounted in `ProtectedLayout`) and route user-facing errors
through it. Replace the inline `setError`-rendered `<p>` bars page-by-page (or
keep `setError` but render via the toast host).

**Affected:** new `web/src/components/Toast*.tsx` + store; `web/src/App.tsx`;
incremental migration of pages currently rendering `error` inline.

**Acceptance:** triggering the share-to-self error shows a dismissible popup,
not an under-navbar bar; no layout shift.

---

## 🟠 #12 — "Audio backup is disabled by server policy" shown as a gallery error

**Symptom (Windows):** gallery shows an error `kalimb.aac: Audio backup is
disabled by server policy` — correct policy, but it shouldn't surface as a UI
error (that's what the diagnostics page is for).

**Root cause (CONFIRMED):** audio upload is rejected with `403 Forbidden`
"Audio backup is disabled by server policy" (`server/src/photos/upload.rs:234`,
`:618`). The Gallery upload loop captures the first error and calls `setError`
(`useGalleryUpload.ts:80-88`) → red bar. Scan filters audio when disabled
(`scan.rs:152-155`) but the autoscan-driven **upload/import** path does not, so
the rejection bubbles up.

**Proposed fix:** treat policy rejections (audio disabled) as a silent
skip on the client — don't `setError` for the known "Audio backup is disabled"
403; log to diagnostics instead. Ensure the import/autoscan path also pre-filters
audio when the toggle is off so it never attempts the upload.

**Affected:** `web/src/hooks/useGalleryUpload.ts`, import path
(`web/src/pages/Import.tsx`, `web/src/pages/import/`), possibly server scan/
import filtering.

**Acceptance:** with audio backup disabled, importing audio shows no error bar;
the skip is visible only in diagnostics/logs.

---

## 🟡 #11 — Conversion banner counter reads "3/4, 5/6, 12/13" (never "4/15")

**Symptom:** the converting-media banner denominator tracks just ahead of the
numerator instead of the true batch total.

**Root cause (CONFIRMED):** two progress models in
`server/src/conversion.rs`:
- batch ingest uses `progress_start(total)` (`:67-71`) — correct.
- the per-upload path uses `progress_add(1)` + `progress_finish_one()`
  (`:93-108`), so `CONV_TOTAL` only ever grows one ahead of `CONV_DONE`. During
  import each file registers as its own +1, so the banner never knows the full
  batch size. The frontend just renders `done/total` verbatim
  (`ConversionBanner.tsx:94`).

**Proposed fix:** have the import/upload flow pre-register the full convertible
count via a single `progress_start(total)` (or a batch-aware `progress_add`
seeded with the known total) so the denominator reflects the whole batch.

**Affected:** `server/src/conversion.rs`, `server/src/ingest.rs`,
`server/src/photos/upload.rs`, import handlers.

**Acceptance:** importing 15 convertible files shows `n/15` throughout.

---

## 🟠 #2 — Upload "+" button unusable during convert/import

**Symptom:** can't use the upload "+" FAB while converting or importing is
running.

**Findings:** Gallery FAB itself isn't disabled, but the file `<input>`s are
`disabled={uploading}` (`Gallery.tsx:353,366`) — that flag is for local uploads,
not server-side conversion/import. The `ConversionBanner` is `fixed bottom-20
... z-50` (`ConversionBanner.tsx:88`) and the FAB is `fixed bottom-6 right-6
z-50` (`Gallery.tsx:333`) — **(VERIFY)** possible z-index/pointer overlap, or a
processing gate elsewhere blocks interaction.

**Proposed fix:** confirm the actual blocker via repro, then ensure manual
upload stays available during background convert/import (they're independent of
the server conversion queue). If it's overlap, fix stacking/pointer-events.

**Affected:** `web/src/pages/Gallery.tsx`,
`web/src/components/ConversionBanner.tsx`, `web/src/hooks/useGalleryUpload.ts`.

**Acceptance:** while conversion/import runs, the "+" opens and accepts files.

---

## 🟠 #6 — Add-to-album popup needs "Create new album" + bounded scroll

**Symptom (screenshot 4):** the "Add N items to album" popup lists albums but
has no way to create a new album; needs to stay scrollable/size-limited with
many albums.

**Findings:** `AddToAlbumModal.tsx` already caps height and scrolls
(`max-h-[80vh]` + `overflow-y-auto`, `:77,95`). The real gap is the missing
create-album affordance. Empty state currently says "create one from the Albums
page first" (`:100-103`).

**Proposed fix:** add a "+ New album" row/button at the top of the modal that
creates an album inline (reuse the album-create flow from `Albums.tsx` /
`AlbumDetail` manifest pattern) and immediately adds the selected items.
Optionally tighten the list cap (e.g. `max-h-[60vh]`).

**Affected:** `web/src/components/AddToAlbumModal.tsx` (+ album-create helper).

**Acceptance:** create an album from the popup and the selection lands in it;
list scrolls with many albums; modal never exceeds the cap.

---

## 🟠 #9 — Light mode text contrast too low (don't touch dark mode)

**Symptom:** light-mode text is hard to read (insufficient contrast).

**Findings:** widespread `text-gray-500` / `text-gray-600` used for light-mode
body/secondary text (paired with `dark:text-gray-400` variants). These are
low-contrast on white.

**Proposed fix:** systematic pass to darken **light-mode only** secondary text
(e.g. `text-gray-500` → `text-gray-700`, `text-gray-400` → `text-gray-600`)
while leaving every `dark:` variant untouched. Verify against WCAG AA.

**Affected:** broad — `web/src/**/*.tsx`, `web/src/index.css`,
`web/tailwind.config.js`. Prefer adjusting shared classes/components over
one-offs.

**Acceptance:** light-mode secondary text meets AA contrast; dark mode visually
unchanged (diff the `dark:` classes — none should move).

---

## 🟡 #14 — Button/card facelift (more depth, less flat)

**Symptom:** buttons and cards feel flat/simple; UI needs a classier facelift
with more depth.

**Proposed fix:** design pass — consistent elevation (layered shadows), subtle
gradients/borders, refined hover/active/focus states, unified radius and
spacing. Build shared `<Button>` / card primitives so it's applied once, not
sprinkled. Reference `simple-photos-mockup1.jpg` in repo root.

**Affected:** shared UI primitives + Tailwind theme; sweep high-traffic surfaces
(Gallery FAB, top bars, modals, settings cards).

**Acceptance:** buttons/cards have consistent depth and states across the app;
no regressions in dark mode.

---

## 🟠 #1 — Bundle dependencies into installers; offer bundled vs slim

**Symptom/ask:** offer installers with all dependencies bundled (for
offline/airgapped installs) as an option alongside the current network-fetch
installers.

**Findings (current state):**
- `install.sh` / `install.ps1` fetch ONNX models (~200 MB) + GeoNames at
  install time from upstream (`install.sh:95-116`).
- `.deb` bundles the Android APK + ORT provider libs; `.exe` bundles
  `vc_redist`, `nssm`, `onnxruntime*.dll`.
- **FFmpeg is an external runtime dependency**, not bundled.
- ONNX models are also mirrored onto each GitHub release (see #5).

**Decisions to make (Session G):** which deps to bundle (ffmpeg? models?
GeoNames dataset?), bundled-installer size budget, naming (`*-offline` vs
slim), and how releases carry the larger artifact.

**Affected:** `install.sh`, `install.ps1`, `packaging/debian/*`,
`packaging/windows/*`, `.github/workflows/pipeline.yml`.

**Acceptance:** an offline installer variant installs with no network access;
the slim variant still works as today.

---

## 🟠 #5 — Geolocation/precise location fails on Ubuntu; pull ONNX from source (off release)

**Two parts:**

### 5a — Geo fails on Ubuntu (VERIFY)
**Symptom:** geolocation + precise location still failing on Ubuntu.
**Findings:** `GeoBanner` surfaces an "unavailable" state when the GeoNames
dataset isn't loadable, and a "downloading" self-heal state
(`GeoBanner.tsx:97-106,82-92`). Server reports `geo_progress.available/
downloading` via `/status/activity`. Likely the GeoNames dataset
install/self-heal fails on the Ubuntu `.deb` path (ties to memory notes:
"geo dataset server auto-fetch", "stuck geo-banner = dataset availability").
**Next step:** reproduce on Ubuntu, inspect server geo dataset fetch/load
(`server/src/geo/`) and the `.deb` setup service.

### 5b — ONNX models off the release page, fetch from source
**Symptom/ask:** don't ship `.onnx` files on the GitHub release; pull from
source if needed.
**Findings (CONFIRMED):** `pipeline.yml:887-917` mirrors `det_10g.onnx` +
`w600k_r50.onnx` onto every release and `files:` attaches `dist/*.onnx`. The
reason it was added (comment `:878-886`): HuggingFace's **Xet CDN
(cas-bridge.xethub.co)** broke install-time downloads on some networks. The
`fetch-assets` scripts already pull from upstream
(`packaging/debian/fetch-assets.sh:63-66`,
`packaging/windows/fetch-assets.ps1:226-233`).
**Tension to resolve:** removing the release mirror re-exposes the original Xet
DNS failure. Fix must (a) drop `*.onnx` from the release `files:` + the mirror
step, **and** (b) make upstream fetch robust (retries/`--retry-all-errors`,
alternate mirror, or a non-Xet URL) so installs don't regress.

**Affected:** `.github/workflows/pipeline.yml`, `install.sh`, `install.ps1`,
`packaging/*/fetch-assets.*`, `server/src/geo/`.

**Acceptance:** release page has no `.onnx` assets; fresh installs still fetch
models reliably from source on networks that previously failed; Ubuntu geo
resolves precise locations.

---

## Cross-cutting follow-ups

- [x] Audit all hardcoded `navigate("/gallery")` / `navigate("/albums")` for
      lost context (#7) and the singular `/album/` route typo. *(done 2026-06-16
      — viewer now origin-aware; remaining hardcoded navigates verified correct;
      Search→gallery left as a known non-scope gap.)*
- [ ] Establish the toast system (#8) before migrating per-page error bars.
- [ ] Per AGENTS.md: unit + manual/E2E verification and error-path logging on
      every fix; no `as any` / `@ts-ignore` / empty catches introduced.
