# TODO — Open Issue Resolution Plan

Generated from GitHub open issues (Wulfic/simple-photos) on 2026-07-13.
18 open issues. Ordered by **priority**, then grouped by **subsystem** so shared
root causes get fixed once, not three times.

Legend: 🔴 High · 🟡 Medium · 🟢 Low · ✨ Feature

---

## EPIC A — Album Path Unification (the root cause of ~6 issues)

> **Fix this first.** Issues #12, #20, #25, #26, #27 are symptoms of three
> divergent album implementations (smart / secure / regular). #16 leaks across
> the same seam. Stop patching each surface — build the shared layer, then the
> symptom tickets become trivial re-wires.

### A0 — #28 🔴 Unify smart / secure / regular album functions — ⏳ WEB DONE
> **PREMISE CORRECTION (2026-07-13):** A0 as generated assumed a server-side
> `album_members(album_id)` resolver could unify all three. That is **infeasible**
> — regular albums are E2E-encrypted manifests (`album_manifest` blobs); the
> server only ever sees ciphertext and cannot read membership. Confirmed against
> both clients + mem0 ("Albums stay real E2E, NOT virtual"). The unification is
> therefore **client-side**, not server-side. Reframed accordingly below.
- [x] **code-explore**: mapped all three album paths across server/web/Android.
      Regular = E2E manifest resolved client-side (web: `db.albums.photoBlobIds`
      filtered against `db.photos`; android: Room `PhotoAlbumXRef`). Smart = live
      client filter (+ server clusters for people/pets/trips/memories). Secure =
      `secure_gallery_items` server table, token-gated. Shared = `shared_album_photos`.
- [x] Divergences found: (1) regular-album **count source** diverged from the
      rendered grid — header showed raw `photoBlobIds.length` (secure-inclusive,
      stale-inclusive) while the grid showed a secure-filtered list → #12/#20;
      (2) burst-collapse re-implemented inline in 3 web surfaces; (3) Android
      regular albums not secure-excluded in `getAlbumPhotos` (→ folded into #16).
- [x] Shared contract (client-side):
  - [x] Web: one `useAlbumPhotos(albumId)` hook (`hooks/useAlbumPhotos.ts`) with a
        pure `resolveAlbumPhotos` core — single source for `photos` **and**
        `count` (always `photos.length`), secure-excluded. Shared
        `utils/burstCollapse.ts` + `gallery/smartAlbums.ts`. SmartAlbumView +
        RegularAlbumView rewired onto it; header count bug fixed.
  - [x] Android: `getAlbumPhotos` already the shared smart+regular resolver
        (grid + viewer). Verified. Remaining divergence = regular-album
        secure-exclusion → handled in **#16** (secure filtering lives at the
        Android VM layer, added there with its exclusion test).
  - [ ] `AlbumGrid` component consolidation deferred — SelectablePhotoGrid +
        JustifiedGrid already shared by smart/people/pets/trips; RegularAlbumView
        keeps its own grid for the manifest-CRUD affordances. Low value to merge.
- [x] Unit tests: `useAlbumPhotos.test.ts` (10) + `burstCollapse.test.ts` (5) —
      assert membership, secure-exclusion, ordering, limit, and the
      count===length invariant. Added vitest to web (`npm run test`). Build green.
- [x] **Dependency for A1–A5 satisfied for web.** (Gallery.tsx/Search.tsx still
      have their own inline burst-collapse — follow-up dedup, not blocking.)

### A1 — #12 🟢 Album photo counts missing (regular albums) — ✅ DONE (5637fd0)
- [x] Root cause: regular albums didn't populate `count` like smart albums.
      Wired to the shared count source from A0.
- [x] Web (`countRegularAlbum`) + Android (`AlbumCard` "N items") render the
      secure-excluded count on regular album tiles.
- [x] Tests: `useAlbumPhotos.test.ts` count===length + `SecureExclusionTest`.

### A2 — #27 🔴 Album "add photos" has no photo selector (regular) — ✅ DONE (e3f7407)
- [x] Root cause was NOT "no picker wired" — the picker fed the whole ~7000
      photo library into `AddPhotosPanel` whose `ThumbnailImg` eagerly built an
      object URL per tile at mount, freezing the tab so the picker never painted.
- [x] Fix: `ThumbnailImg` now lazy-loads via IntersectionObserver (parity with
      `ThumbnailTile`); `AddPhotosPanel` gained a filename search + select-all +
      60vh area so the large list is navigable.
- [x] Unit test: pure `filterPickerPhotos` (`pickerFilter.test.ts`). Browser E2E
      pending (no web DOM harness).

### A3 — #20 🔴 Albums section glitch / count flicker (Android)
- [ ] Symptom: album list "constantly refreshing", tiles reorder, count flashes
      ~7000 then settles to 5645. Likely: (a) list re-fetch loop with unstable
      keys, and (b) count shows total-library before album-scoped count resolves.
- [ ] Fix unstable list keys / re-render loop (check `LaunchedEffect` deps and
      album list state source).
- [ ] Show album-scoped count from the start (A0 resolver), never the global
      library total as a placeholder.
- [ ] Verify no infinite refresh via network log on the albums screen.

### A4 — #25 🟢 Secure albums missing "Select all" button — ✅ DONE (pending commit)
- [x] Added Select-all/Deselect-all to BOTH Android secure-album surfaces that
      lacked it: the item-selection top bar (manage/remove) and the add-photos
      picker header (secure a whole source in one tap). Web secure gallery has no
      multi-select mode, so nothing to add there.
- [x] Selection top bar toggles all `displayItems`; picker toggles the current
      source's `availablePhotos` into the running selection.

### A5 — #26 🟢 Regular album trash-can icon too small — ✅ DONE (5637fd0)
- [x] Bumped trash + back icons 12dp→24dp on AlbumDetailScreen.

---

## EPIC B — Google Takeout Import Correctness

> Recurring pain (see mem0: register-path-blind, dedup-drop, dates). These are
> **High** and user-facing on every fresh import.

### B1 — #11 🔴 Google Photos imports still not recreating albums faithfully
- [ ] Reproduce with a known Takeout sample; diff reconstructed albums vs
      Takeout `metadata.json` album membership.
- [ ] Verify the register-path (`register_native_file`) resolves sidecars +
      album membership (mem0 says this was fixed — confirm it holds for ALL
      entry paths, not just one).
- [ ] Confirm dedup no longer drops album membership for duplicated copies
      (mem0: dedup-drop root cause of empty albums).
- [ ] Add integration test: import fixture Takeout → assert album count + per
      album membership match manifest.

### B2 — #13 🔴 Date/time import errors (wrong photo ordering)
- [ ] Taken-at timestamps wrong → gallery order wrong.
- [ ] Verify EXIF `DateTimeOriginal` + `OffsetTimeOriginal` parsing and the
      `taken_at` / `taken_at_offset` write (migr 026).
- [ ] Verify Takeout `photoTakenTime` JSON sidecar is preferred/merged correctly
      when EXIF is missing.
- [ ] Check timezone handling — offset-aware `taken_at` (mem0 note).
- [ ] Test: fixtures with (a) EXIF only, (b) sidecar only, (c) both conflicting,
      (d) neither → assert deterministic, correct ordering.

---

## EPIC C — GIF Handling

### C1 — #14 🟢 GIFs not detected → missed by smart album
- [ ] Audit GIF detection (magic bytes / mime, not just extension).
- [ ] Ensure detected GIFs get the subtype that the GIF smart album queries on.
- [ ] Backfill: re-tag existing undetected GIFs.
- [ ] Test: fixture set of GIFs (various sources) all land in the smart album.

### C2 — #18 🟡 GIF crop-save doesn't regenerate thumbnail
- [ ] Crop-save edits the full GIF correctly but thumbnail keeps old frame.
- [ ] Trigger thumbnail regeneration on GIF edit-save (same path as photo crop
      thumb regen — check `crop no-thumb-regen` mem0 note for the pattern).
- [ ] Test: crop a GIF → assert stored thumbnail bytes changed.

---

## EPIC D — Video Playback

### D1 — #17 🔴 Large videos fail to play / small videos buffer slowly (Android)
- [ ] Reproduce large-file failure + small-file slow-buffer on Android.
- [ ] Check server range-response handling (206/416 — `http_utils.rs`) under
      large files; confirm chunk sizes + content-range correctness.
- [ ] Check Android player buffering config + whether decrypt/stream is
      blocking (blob_stream path).
- [ ] Consider transcode/faststart (moov atom at front) for large source files.
- [ ] Test: play a large (>1GB) and small (<20MB) video end-to-end on device.

### D2 — #22 🟢 Videos should loop instead of stopping after one play
- [ ] Enable looping in the video player (web + Android). Small change; do both
      platforms for parity.

---

## EPIC E — Selection & UI Polish

### E1 — #24 🟢 Multi-select can't select a whole day at once
- [ ] Add day-header "select all in day" action to gallery multi-select.
- [ ] Reuse unified SelectionState; share the SelectAll action with #25.

### E2 — #23 🟢 Delete button text wraps to 2 lines → use trash icon
- [ ] Replace "Delete" text with trash-can icon (parity with #26 sizing).

### E3 — #19 🟢 Rapid favoriting hides top bar; slow to reappear
- [ ] Investigate top-bar auto-hide triggering on fast successive taps
      (immersive/gesture sensitivity). Debounce or exempt favorite taps from the
      auto-hide gesture detector.
- [ ] Verify tap-to-reveal responds immediately after rapid favoriting.

### E4 — #15 🟢 Search shows tags on media (shouldn't)
- [ ] Search results render media WITH tag overlays; suppress tag overlay in
      search result tiles.

### E5 — #16 🔴 Secure albums don't fully remove media from regular Gallery — ✅ DONE
- [x] Read-side exclusion: Android album-detail/add-picker secure-excluded via
      shared `excludeSecure` (5637fd0); web already filtered `secureBlobIds`.
- [x] Root cause of "most but not all": the secure-ADD **batch aborted on the
      first failure**, leaving every later photo un-secured yet still shown. Web
      looped with no per-item catch; Android `async{}.awaitAll()` cancelled all
      siblings on one throw. Both now secure each photo independently, log every
      failure, clean up the succeeded set, and report partials (359fd8d).
- [x] Unit tests: `runSecureAddBatch` resilience + `secureAddResultMessage`
      (+10). Bulk-add device E2E pending (no automated device harness here).

---

## EPIC F — Feature Requests

### F1 — #21 ✨ Split-screen: view 2 photos simultaneously
- [ ] Design first (think-plan). Web + Android. Lower priority than all bug
      work above — schedule after Epics A–E.

---

## Suggested execution order
1. **A0 (#28)** — unblocks A1–A5 and de-risks #16.
2. **B1, B2 (#11, #13)** — High, hits every import.
3. **D1 (#17)** — High, core playback broken.
4. **A1–A5, #16** — clear the album symptom tickets on the new shared layer.
5. **C1, C2, D2, E1–E4** — Low/Medium polish, batch them.
6. **F1 (#21)** — feature, last.

## Definition of done (per AGENTS.md — non-negotiable)
- Unit tests AND E2E/device tests green before "done".
- Logging on every new error path.
- No `@ts-ignore` / `as any` / empty catch.
- Conventional commit + updated mem0 memory per session.
