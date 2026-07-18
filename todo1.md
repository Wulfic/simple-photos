# todo1 — Open GitHub Issues Investigation & Resolution Plan

Pulled from `Wulfic/simple-photos` on 2026-07-17. 9 open issues: #29–#37.
Ordered by priority (High → Medium → Low). Each item: root cause / current
state (verified in code), plan, and test expectations.

Recommended execution order: **#32 → #37 → #31 → #29 → #35 → #30 → #34 → #33 → #36**
(priority first, then cheap wins). One conventional commit per issue on `dev`.

---

## HIGH

### #32 — Face recognition: rename person on phone + zoomed face previews

**Issue:** (1) On Android there is no way to rename a person in the People
section. (2) Face previews should be zoomed/cropped to the detected face,
not the whole photo — matters most for group shots.

**Current state (verified):**
- Server already has everything needed:
  - `PUT /api/ai/faces/:cluster_id/name` — [handlers.rs:323](server/src/ai/handlers.rs#L323)
  - Per-detection bboxes: `GET /api/ai/faces/:id/photos` returns
    `bbox_x/y/w/h` — [handlers.rs:307](server/src/ai/handlers.rs#L307);
    `GET /api/ai/photos/:photo_id/faces` too ([handlers.rs:519](server/src/ai/handlers.rs#L519)).
  - **Gap:** `GET /api/ai/faces` (cluster list, [handlers.rs:272](server/src/ai/handlers.rs#L272))
    returns only `representative` photo id — **no bbox** for the representative
    face, so clients can't crop the People-grid tile.
- Android already has the API plumbed: `renameFaceCluster` in
  [ApiService.kt:444](android/app/src/main/kotlin/com/simplephotos/data/remote/ApiService.kt#L444)
  and [AiRepository.kt:38](android/app/src/main/kotlin/com/simplephotos/data/repository/AiRepository.kt#L38)
  — it is simply **not wired to any UI** in `PeopleScreen` / `PersonDetailScreen`
  ([LibraryFeatureScreens.kt](android/app/src/main/kotlin/com/simplephotos/ui/screens/library/LibraryFeatureScreens.kt)).
- Web `PersonDetailView` already renames ([PeopleView.tsx:94](web/src/pages/albumDetail/PeopleView.tsx#L94)) — parity target.
- Tiles on both platforms render the whole photo thumb (`ContentScale.Crop` /
  `object-fit: cover`), no face-aware crop.

**Plan:**
1. **Server:** extend the `GET /api/ai/faces` query to also return the
   representative detection's `bbox_x/y/w/h` (join `face_detections` on the
   representative photo, highest confidence). Add fields to
   `FaceClusterSummary` (additive — no client breakage). Same for pets if
   trivial, else skip.
2. **Android rename UI:** in `PersonDetailScreen` add an edit/rename affordance
   (pencil icon next to the title, or long-press on a person card in
   `PeopleScreen`) → `AlertDialog` with text field → `AiRepository.renameFaceCluster`
   → refresh list. Log failures; toast on error.
3. **Android face-zoom tiles:** given bbox (normalized coords — confirm units
   in `face/mod.rs`), render the thumbnail cropped around the face: expand the
   bbox ~2× (clamped to image bounds) and apply a `graphicsLayer`
   scale+translate inside a clipped square tile — same technique as the crop
   display fix in the viewer (see memory `android-crop-display-bug`). Applies
   to `PeopleScreen` grid and the face chips in the viewer tag panel.
4. **Web parity:** same zoomed crop for `PeopleView` cards using the new bbox
   fields (CSS `object-position`/transform math in the tile).
5. **Tests:** server unit test for the new summary fields; Android build +
   device sanity; web vitest for the crop-math helper (pure function —
   `computeFaceCropTransform(bbox, tileSize)` shared, exported, tested).

---

## MEDIUM

### #37 — Phone back button should honor screen history (max 5, then Gallery)

**Issue:** Back always returns to "the last regular screen". Wanted: back
walks real history up to 5 screens; deeper than that → jump to Gallery.

**Current state (verified):** [NavGraph.kt](android/app/src/main/kotlin/com/simplephotos/ui/navigation/NavGraph.kt)
— bottom-nav Gallery clicks do `navigate(Gallery) { popUpTo(0) }` (wipes
stack), while every other top-level navigation is a plain `navigate()` with no
`launchSingleTop`/dedup, so the stack grows unboundedly with duplicates
(Albums→Search→Albums→Search…). Back then replays every hop.

**Plan:**
1. Add `launchSingleTop = true` to top-level (bottom-nav) destinations so
   A↔B ping-pong doesn't stack duplicates.
2. Enforce the 5-screen cap with a `navController.addOnDestinationChangedListener`
   in `NavGraph`: track our own route-history list in `NavViewModel` (don't
   touch the restricted `backQueue` API). Add a `BackHandler` at the NavHost
   level (enabled when depth-above-Gallery > 5) that does
   `navigate(Gallery) { popUpTo(0); launchSingleTop = true }`; otherwise let
   the system pop normally.
3. Careful not to intercept back inside PhotoViewer edit mode / secure gallery
   password gate — those screens have their own BackHandlers; ours must be
   outermost (registered first) so theirs win.
4. **Tests:** compose navigation unit test: push 7 screens → press back → land
   on Gallery; push 3 → back walks each. Device-verify on S21+ (predictive
   back too).

### #31 — Secure albums: align features with regular albums + cross-secure-album picker

**Issue:** (1) Secure album photos should have the same features (edit, info,
etc.) as regular albums, via the standard shared helpers. (2) Add an option —
secure albums only — to add media *from other secure albums*.

**Current state (verified):**
- Memory `todo1-viewer-gallery-polish-implemented` says secure viewer got
  ⋮-menu parity, but edit/info in the secure viewer still diverge from the
  shared regular-album path (web [SecureGallery.tsx](web/src/pages/SecureGallery.tsx),
  Android [GalleryDetailView.kt](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/GalleryDetailView.kt),
  [SecurePhotoViewer.kt](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt)).
- Server already has the aggregate `GET /galleries/secure/items` endpoint
  returning `gallery_id` per item (built for smart albums — memory
  `secure-smart-albums-implemented`). That is exactly the data source a
  cross-secure-album picker needs.
- Secure-add pickers currently source from the main gallery only (memory
  `secure-add-from-gallery`).

**Plan (audit first — this is the fuzziest issue):**
1. **Audit pass:** enumerate viewer features available in regular albums
   (edit panel, info panel, favorite, download, cast, tags, slideshow) vs the
   secure viewer, web + Android. Produce the concrete gap list before coding;
   confirm scope with the issue if a feature is intentionally absent
   (e.g. cast of decrypted secure media may be a deliberate no).
2. **Alignment:** route secure viewer through the shared helpers
   (`useViewerActions`, shared info panel / edit components) with a
   `secure` source flag, instead of parallel implementations. Server-side:
   metadata-edit endpoints may need secure-item equivalents — check before
   promising in-place edit; if secure blobs are immutable, "edit" = re-encrypt
   new blob + replace item (same pattern as regular E2E edit save).
3. **Cross-secure-album picker:** in the secure album "add items" flow, add a
   source chip "Other secure albums" (web `AddPhotosPanel` variant +
   Android picker source chips — the chip pattern already exists from the
   gallery-source work). Data: aggregate secure items endpoint filtered to
   `gallery_id != current`, with the 409 dup-guard already on the server.
   Adding copies membership; decide copy-vs-move (default: copy, matching
   regular albums).
4. **Tests:** server tests for any new/changed endpoints; web vitest for the
   picker filtering; E2E secure flow (note pre-existing test_06 401 failures —
   memory `e2e-preexisting-failures` — don't chase those).

---

## LOW

### #29 — Portrait photos falsely flagged as panoramic

**Issue:** Some portrait photos get the pano treatment. "Panoramic photos are
wide, not tall."

**Root cause (verified):** the XMP-less aspect fallback in
[subtype.rs:224-277](server/src/photos/metadata/subtype.rs#L224-L277)
**deliberately tags tall images as vertical panoramas**
(`h/w ≥ 3.0` Strict, `≥ 2.5` Loose — line 256-262). EXIF orientation is
already handled (dims swapped, one-time repair `orientation_dim_fix_v2` in
[media.rs:539](server/src/photos/metadata/media.rs#L539)), so the vertical-pano
rule itself is the offender — tall screenshots/scrolls/collages hit it.
Also: rows tagged before the Strict threshold change are never re-evaluated
(backfill only fills NULLs, never un-tags).

**Plan:**
1. Remove the vertical-pano branch from `apply_aspect_subtype_fallback_with`
   (keep horizontal + equirectangular). Photos with genuine `GPano` XMP are
   unaffected (handled upstream).
2. Add a one-time repair pass (same `server_settings` latch pattern as
   `orientation_dim_fix_v2`): `UPDATE photos SET photo_subtype = NULL` where
   `photo_subtype = 'panorama' AND height > width`, then let the normal
   backfill re-evaluate (they'll stay untagged). Only touch heuristic tags —
   photos whose XMP genuinely says pano get re-tagged by backfill anyway since
   re-scan reads XMP first.
3. Update/remove the vertical-pano unit tests
   ([subtype.rs:843](server/src/photos/metadata/subtype.rs#L843),
   `strict_keeps_extreme_vertical_pano`, etc.); add a regression test: tall
   portrait (1080×4000) stays `None` under both sensitivities.
4. `cargo test --bin` (crate is a binary — memory `todo-item16`).

### #35 — Regular albums UI changes (album detail header + persistent navbar)

**Issue:** In album detail: replace top-right trashcan with a ⋮ menu
(Delete, Rename, Cast), add a `+` button next to it for adding items, drop the
photo-count text, drop the separate Add-Photos header row. Keep the main
navbar visible everywhere in the albums section except the photo viewer.

**Current state (verified):** both platforms have the described layout:
- Web [RegularAlbumView.tsx](web/src/pages/albumDetail/RegularAlbumView.tsx):
  `DetailHeader` with `count="N items"` (line 281), Cast + Add Photos + Delete
  as separate header buttons (lines 284–306). Web keeps `AppHeader` (line 262)
  — navbar persistence is satisfied on web; verify People/Pets/Memories/Trips
  views also render `AppHeader` (they use the same DetailHeader pattern —
  audit each).
- Android [AlbumDetailScreen.kt](android/app/src/main/kotlin/com/simplephotos/ui/screens/album/AlbumDetailScreen.kt):
  trashcan `IconButton` in TopAppBar (line 123), separate "Add Photos" row
  (lines 248–265). No bottom navbar on detail screens — `AlbumDetailScreen`
  (and People/Pets/etc. detail screens) are pushed routes without the nav bar,
  so "navbar always present in albums section" is an **Android change**:
  give AlbumDetail (and the library detail screens) the same bottom-nav
  scaffold used by `AlbumListScreen`, hidden only in `PhotoViewerScreen`.

**Plan:**
1. **Web:** collapse header actions into ⋮ menu (Delete, Rename — new,
   name-edit modal writing via `saveAlbumManifest` — and Cast) + `+` icon
   button toggling the AddPhotosPanel. Remove `count` prop usage. Rename needs
   a small modal; album rename already exists on the album list — reuse that
   logic.
2. **Android:** TopAppBar actions → ⋮ `DropdownMenu` (Delete, Rename, Cast) +
   `+` action; remove the standalone Add-Photos row; remove photo-count
   subtitle if shown.
3. **Android navbar:** add the bottom nav to album/library detail screens
   (component already shared by the list screens — check `AlbumListScreen`'s
   scaffold for the reusable piece).
4. **Tests:** web vitest snapshot/interaction for the new menu; Android build;
   manual device verify.

### #30 — Photo viewer: put Info in the ⋮ menu as well

**Current state (verified):** both platforms have a standalone Info button
plus an overflow menu that lacks Info:
- Web [ViewerTopBar.tsx:113](web/src/components/viewer/ViewerTopBar.tsx#L113)
  (Info toggle) and the ⋮ menu at line 133.
- Android [PhotoViewerScreen.kt:1071](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/PhotoViewerScreen.kt#L1071)
  (Info) and `DropdownMenu` at line 1121.

**Plan:** add an "Info" item at the top of the ⋮ menu on web, Android, and the
secure viewer's menu (`SecurePhotoViewer.kt` — it got menu parity in the
todo1 polish pass); keep the standalone button (issue says "as well").
Trivial; test = build + snapshot.

### #34 — Android settings: Manage Users card should open the web client

**Current state (verified):** [SettingsScreen.kt:458-473](android/app/src/main/kotlin/com/simplephotos/ui/screens/settings/SettingsScreen.kt#L458-L473)
— admin-only card renders static text "Open {serverUrl} in a browser…" with no
click handler.

**Plan:** make the card (or a button in it) clickable →
`LocalUriHandler.openUri("${serverUrl}/settings")` (verify the web settings
route — user management lives in web Settings → Users,
[UserManagement.tsx](web/src/components/settings/UserManagement.tsx)). Also
make the "Active Server" URL row (line 453) open `serverUrl`. Guard: catch
`ActivityNotFoundException`/handler failure with a toast + log. HTTPS/HTTP
scheme comes straight from `serverUrl` so no scheme guessing.

### #33 — Android thumbnail-size labels: "Large" wraps vertically

**Current state (verified):** [SettingsScreen.kt:290-322](android/app/src/main/kotlin/com/simplephotos/ui/screens/settings/SettingsScreen.kt#L290-L322)
— outer `Row(SpaceBetween)` with an unconstrained left `Column` (long
description text) squeezes the right `Row("Normal" · Switch · "Large")`; the
trailing "Large" text gets ~0 width and wraps character-per-line.

**Plan:** give the left `Column` `Modifier.weight(1f, fill = false)` (or
`weight(1f)` + `padding(end)`) and mark both labels `softWrap = false` /
`maxLines = 1` so the toggle group keeps its intrinsic width. One-file fix;
verify on device at normal + large font scale.

### #36 — Add-to-album with large selection breaks button alignment

**Current state (best candidates, verify visually first):**
- [RegularAlbumView.tsx:326-361](web/src/pages/albumDetail/RegularAlbumView.tsx#L326-L361)
  — selection bar (`flex items-center justify-between`) with count text +
  "Add Selected"/Cancel; long counts push/wrap the buttons.
- [AddPhotosPanel.tsx:47-64](web/src/components/AddPhotosPanel.tsx#L47-L64)
  — same pattern ("Select photos to add (N selected)" + buttons).
- [AddToAlbumModal.tsx:103](web/src/components/AddToAlbumModal.tsx#L103) —
  title `Add N items to album` can wrap the modal header.

**Plan:** reproduce with a 4-digit selection, then fix the offending bar(s):
`min-w-0 truncate`/`tabular-nums` on the count text, `shrink-0 whitespace-nowrap`
on the buttons, `flex-wrap` fallback for narrow viewports. Check the Android
equivalent selection bar for the same overflow while there. Test: vitest DOM
assertion that buttons stay on one row with a 10,000-item label (or visual
check + screenshot).

---

## Cross-cutting notes

- **Branching/commits:** work on `dev`, one `fix:`/`feat:` conventional commit
  per issue referencing `(#NN)`; never commit red (E2E pre-existing reds in
  memory `e2e-preexisting-failures` are the known exceptions).
- **Server DTO changes** (#32 cluster bbox, any #31 endpoint work): server is
  authoritative for DTOs — update web + Android DTOs in the same commit
  (memory `android-web-alignment-state`).
- **Builds:** cargo only in PowerShell (OpenSSL perl); AI crate tests via
  `cargo test --bin`.
- **Device verification** queue after implementation: #32, #37, #35, #34, #33
  on the S21+ harness (`.device-test\dev.ps1`).
