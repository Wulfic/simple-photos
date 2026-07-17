# Viewer & Gallery Polish — Investigation + Fix Plan (todo1)

Status: **IMPLEMENTED on `dev`** (2026-07-16) — one conventional commit per
issue. `compileDebugKotlin` + `testDebugUnitTest` green; web `tsc` + `vite
build` clean. Remaining: device verification on the S21+ harness for #1/#2/#4,
the #2 on-device repro, and optional sticky day headers.

| Issue | Commit | Notes |
|---|---|---|
| #4 New Window everywhere | `10cdb74` | `rememberNewWindowLauncher()`; dropped `onNewWindowClick` knob |
| #1 secure video controls + immersive | `5557433` | TextureView + shared `VideoControlsOverlay`; hide system bars |
| #3 viewer ⋮ overflow menu | `a440667` | web + Android + secure; E2E is API-level, no selector churn |
| #2 grid width + select-day chip | `0ce54a8` | `BoxWithConstraints` width; `shortLabel` "Select MMM d" |

---

## Issue 1 — Secure vs regular video viewers have divergent paths; gear icon unreachable under on-screen nav (Android)

### Audit — confirmed root cause
Two completely separate video playback implementations:

| | Regular gallery viewer | Secure album viewer |
|---|---|---|
| File | [VideoPlayer.kt](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/VideoPlayer.kt) (`VideoPlayerPage`) | [SecurePhotoViewer.kt:401-459](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt#L401-L459) (`SecureVideoPage`) |
| Player | ONE shared `ExoPlayer` owned by `PhotoViewerScreen` | per-page `ExoPlayer.Builder(...).build()` |
| Surface | raw `TextureView` (transform-safe) | media3 `PlayerView` |
| Controls | custom `VideoControlsOverlay` (play/seek/mute — **no gear**) | `useController = true` → **stock media3 controller, incl. the settings-gear button** |
| System bars | Immersive: `PhotoViewerScreen` hides system bars ([PhotoViewerScreen.kt:679-691](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/PhotoViewerScreen.kt#L679-L691)) | Never hidden; `PlayerView` has **no navigationBarsPadding** |

So in the secure viewer the stock controller's bottom row (where the gear lives)
renders underneath the phone's on-screen navigation bar → the gear is visually
covered and effectively unclickable. The regular viewer never shows a gear at
all, and hides the nav bar anyway. Classic copy-path divergence.

Also divergent: secure videos loop via `REPEAT_MODE_ALL` while the regular path
does trim-aware looping (#22) — acceptable (secure clones have no trim
metadata), but the *controls UI* must stop diverging.

### Fix plan
1. **Reuse the custom controls in the secure path.** `VideoControlsOverlay` is
   already `internal` in the same module, and `securegallery` already imports
   viewer symbols (`PanoramaOverlay`, `MAX_PANO_DECODE_PX`). In
   `SecureVideoPage`:
   - Replace `PlayerView(useController = true)` with a `TextureView` +
     `sharedControls` pattern mirroring [VideoPlayer.kt:333-409](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/VideoPlayer.kt#L333-L409):
     tap-catcher toggles `showControls`, 3s auto-hide while playing,
     `VideoControlsOverlay(player, visible, Modifier.align(BottomCenter))`.
   - Keep: per-page player (fine — plays a decrypted temp file), temp-file wipe
     on dispose, `REPEAT_MODE_ALL`.
2. **Immersive parity.** Add the same hide-system-bars `DisposableEffect` from
   `PhotoViewerScreen.kt:679-691` to `SecurePhotoViewer` (the whole viewer, not
   just video pages) so both viewers behave identically. Keep
   `statusBarsPadding()` on the back/action buttons for the transient-bars case,
   and add `navigationBarsPadding()` to the controls overlay so it stays
   reachable when bars are swiped back in.
3. **Guard the motion overlay:** `SecureMotionOverlay` also uses `PlayerView`
   (`useController = false`) — leave it, no controls to cover, but verify the
   LIVE pill (`padding(bottom = 80.dp)`) clears the nav bar once immersive mode
   is in; adjust with `navigationBarsPadding()` if not.
4. **Do NOT attempt** a full merge of SecurePhotoViewer into PhotoViewerScreen in
   this pass (different data models: `SecureGalleryItem` vs `PhotoEntity`,
   decrypt-to-temp vs `spblob://` MediaBlobDataSource). Note it as a follow-up.

### Verification
- Device (S21+ harness, `.device-test\dev.ps1`): open a secure-album video with
  **gesture nav** and with **3-button nav**; all controls tappable; seek works;
  system bars hidden until tapped. Regression: regular gallery video unchanged;
  secure pano/360/motion pages still swipe correctly.
- Confirm no decrypted temp file remains in `cacheDir` after leaving the page.

---

## Issue 2 — Android day selection: chip just says "Select day", date markers missing in gallery

### Audit — NOT yet reproduced statically; the code says the date should render
- `DayHeader` ([GalleryScreen.kt:465-507](android/app/src/main/kotlin/com/simplephotos/ui/screens/gallery/GalleryScreen.kt#L465-L507))
  renders `dateLabel` (`"EEEE, MMMM d, yyyy"`) on the left and the
  "Select day" chip on the right of the SAME row.
- Pipeline is consistent: `groupPhotosByDay`/`buildGridItems`
  ([GalleryViewModel.kt:41-67](android/app/src/main/kotlin/com/simplephotos/ui/screens/gallery/GalleryViewModel.kt#L41-L67))
  → `headerMap`/`dayBreakIndices` ([GalleryScreen.kt:343-379](android/app/src/main/kotlin/com/simplephotos/ui/screens/gallery/GalleryScreen.kt#L343-L379))
  → header emission in [JustifiedGrid.kt:182-196](android/app/src/main/kotlin/com/simplephotos/ui/components/JustifiedGrid.kt#L182-L196).
  Same source list, same stable sort — indices line up.
- Theme contrast is fine (`onSurface` slate-900/gray-100 on slate-50/gray-900).
- `git log` shows no recent regression touching the header path.

### Ranked hypotheses (verify in this order)
1. **Stale APK on the device** — the date-label header + select-day chip landed
   recently (83464b9, Jul 14). Check installed `versionName` vs repo before
   anything else.
2. **Multi-window/split-screen layout bug**: `JustifiedGrid` sizes rows from
   `LocalConfiguration.screenWidthDp` ([JustifiedGrid.kt:159-160](android/app/src/main/kotlin/com/simplephotos/ui/components/JustifiedGrid.kt#L159-L160)),
   NOT the actual container width. In a split window (new #21 flow!) the
   "screen width" is wrong → rows/headers lay out off-window. This is a real
   bug regardless of whether it explains the report.
3. **UX gap, not a render bug**: headers DO render but scroll away, and mid-list
   the chip only says "Select day" — so the user genuinely can't tell which day
   a chip belongs to while scrolling a tall grid.

### Fix plan
1. **Reproduce first**: device screenshots — normal mode, selection mode,
   light/dark, fullscreen AND split-window. If headers are truly absent, add a
   temp log of `dayBreakIndices.size` + emitted header count and bisect.
2. **Fix `JustifiedGrid` container width** (do regardless): wrap the LazyColumn
   in `BoxWithConstraints` and use `constraints.maxWidth` instead of
   `LocalConfiguration.screenWidthDp`. Update
   [JustifiedGridLayoutTest.kt](android/app/src/test/kotlin/com/simplephotos/ui/components/JustifiedGridLayoutTest.kt)
   — `computeRows` itself is pure, so tests stay; add a header-emission test for
   `breakBefore` incl. index 0.
3. **Make the day obvious during selection** (do regardless — this answers the
   actual complaint "can't tell what day you're selecting"):
   - Put the date in the chip: `"Select Jul 14"` / `"Selected"` (short
     `MMM d` format; full date stays in the left label), AND/OR
   - Make day headers **sticky** (`LazyColumn stickyHeader`) so the current
     day marker is always pinned while scrolling, matching web behavior.
4. Re-shoot screenshots; confirm each day group shows its date in both modes.

---

## Issue 3 — Replace viewer top-right trash can with a 3-dot overflow menu (Android + web + secure viewer)

### Audit
Same crowded pattern on all three surfaces, all ending in a bare trash can:
- **Android** [PhotoViewerScreen.kt:1028-1156](android/app/src/main/kotlin/com/simplephotos/ui/screens/viewer/PhotoViewerScreen.kt#L1028-L1156):
  back | favorite ★ | info | tags | Edit | download | **trash** (delete, or
  orange "remove from album" when `albumId != null`).
- **Web** [ViewerTopBar.tsx](web/src/components/viewer/ViewerTopBar.tsx):
  back | info | tags | Edit | favorite | slideshow | download | **trash**
  (red delete or orange remove, gated on `canRemoveFromAlbum` / `isBackupServer`).
- **Android secure** [SecurePhotoViewer.kt:154-168](android/app/src/main/kotlin/com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt#L154-L168):
  back | bare **trash** ("Remove from secure album").

### Proposed grouping (default — flag if you want different)
Visible in the bar: **back | favorite | info | Edit | ⋮**
Overflow menu (top-right `MoreVert`): **Tags · Slideshow (web) · Download ·
Delete / Remove from album** (destructive item last, red/orange, with divider).
Rationale: favorite/info/edit are the high-frequency one-tap actions; tags,
download and destructive ops are occasional and survive an extra tap.

### Fix plan
1. **Android**: new `ViewerOverflowMenu` composable in the viewer package
   (`IconButton(MoreVert)` + `DropdownMenu`). Wire existing handlers:
   `showTagPanel` toggle, download flow (incl. the original-vs-converted
   `showDownloadChoice` dialog), `viewModel.deletePhoto` /
   `viewModel.removeFromAlbum`. Bump `overlayInteraction++` when the menu opens
   so the auto-hide countdown doesn't race it (same trick as favorite, #19).
2. **Web**: add the dropdown to `ViewerTopBar.tsx` (button + absolutely
   positioned menu, close on outside click/Escape — follow the existing
   dropdown pattern in [AppHeader.tsx](web/src/components/AppHeader.tsx)).
   Preserve every current gate: `isBackupServer`, `canRemoveFromAlbum`,
   `isRenderingVideo` spinner state on Download.
3. **Android secure viewer**: replace the bare trash with the same ⋮ menu
   containing "Remove from secure album" (keeps the confirm dialog). This also
   pre-cleans the header for future secure actions (download, etc.).
4. **Parity rule**: identical grouping/order on web and Android — DTO/UX drift
   between the two is a recurring failure mode in this repo.

### Verification
- Web E2E: update the viewer tests in `tests/` that click the delete button
  directly (they will now need to open the overflow first). Run the suite —
  check against the known already-red baseline (test_06/test_20/test_58).
- Device: photo, video, GIF, album-context, secure-context — every action in
  the menu fires; menu closes on selection; auto-hide doesn't eat the open menu.

---

## Issue 4 — "New Window" only appears in the profile menu on the main gallery page (Android)

### Audit — confirmed root cause
- `HeaderNavigation.onNewWindowClick` is nullable and the menu item is skipped
  when null ([AppHeader.kt:67](android/app/src/main/kotlin/com/simplephotos/ui/components/AppHeader.kt#L67),
  [AppHeader.kt:385-400](android/app/src/main/kotlin/com/simplephotos/ui/components/AppHeader.kt#L385-L400)).
- Only `GalleryScreen` wires it ([GalleryScreen.kt:266](android/app/src/main/kotlin/com/simplephotos/ui/screens/gallery/GalleryScreen.kt#L266));
  [AlbumListScreen.kt:94](android/app/src/main/kotlin/com/simplephotos/ui/screens/album/AlbumListScreen.kt#L94),
  [SearchScreen.kt:63](android/app/src/main/kotlin/com/simplephotos/ui/screens/search/SearchScreen.kt#L63),
  [TrashScreen.kt:73](android/app/src/main/kotlin/com/simplephotos/ui/screens/trash/TrashScreen.kt#L73)
  all omit it → item silently missing there.
- The launch-plus-toast UX ("use Recents to arrange split screen") is an inline
  lambda in GalleryScreen ([GalleryScreen.kt:124-135](android/app/src/main/kotlin/com/simplephotos/ui/screens/gallery/GalleryScreen.kt#L124-L135))
  — exactly the divergent-copy setup that caused this.

### Fix plan
1. **Shared helper** in [NewWindow.kt](android/app/src/main/kotlin/com/simplephotos/ui/navigation/NewWindow.kt):
   ```kotlin
   @Composable
   fun rememberNewWindowLauncher(): (String?) -> Unit
   ```
   Moves the GalleryScreen lambda verbatim: `openInNewWindow` + failure toast +
   "arrange from Recents" hint when not already in multi-window mode.
2. **Make the menu item unconditional**: drop `onNewWindowClick` from
   `HeaderNavigation` entirely; `UserMenu` calls `rememberNewWindowLauncher()`
   itself and always shows "New Window". (Alternative: keep the param with a
   non-null default — but there is no screen with AppHeader where the item
   should be hidden, so delete the knob.)
3. Update the 4 call sites; GalleryScreen keeps using the shared helper for the
   Compare flow (`openWindow(Screen.PhotoViewer.createRoute(b))`).
4. Note: a new window opened with `route = null` starts on Gallery — that's the
   intended behavior from any page (route whitelist in `isValidStartRoute`
   stays untouched).

### Verification
- Existing `isValidStartRoute` unit tests must stay green.
- Device: profile menu on Gallery, Albums, Search, Trash → "New Window" present
  everywhere, launches a second window, toast shows when fullscreen.

---

## Execution order
1. **Issue 4** — smallest, pure refactor, deletes duplication.
2. **Issue 1** — secure video controls unification + immersive parity.
3. **Issue 3** — overflow menu, all three surfaces + E2E test updates.
4. **Issue 2** — repro-gated; ship the `JustifiedGrid` width fix + chip/sticky
   header UX regardless of repro outcome.

## Ground rules for the implementation pass
- One conventional commit per issue; never commit with red tests.
- Gradle: `assembleDebug` + unit tests per Android change; web: typecheck +
  build + E2E delta vs the pre-existing-failure baseline
  (test_06 secure 401s, test_20 dates, test_58 harness — see memory).
- Device verification on the S21+ harness is REQUIRED for issues 1, 2, and 4
  (all have symptoms only visible on a real phone with on-screen navigation).
