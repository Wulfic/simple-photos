# TODO 2 — Replace bespoke Compare with true "open the app twice" split-screen (#21 follow-up)

**Goal:** the split-screen feature should let the user literally run the app twice on
their device — two fully independent windows of the SAME app, each with the complete
experience (gallery, viewer, editing, info panel, albums, search — everything). No
bespoke two-pane viewer to maintain; the OS window manager does the splitting, and
nothing changes underneath the hood because each "pane" is just… the app.

Status: **INVESTIGATION DONE — NOT IMPLEMENTED.** Findings below, then the plan.

---

## 1. Findings — what exists today (commit `06a1f2e` "spilitscreeeen view")

The current implementation is a **bespoke two-pane Compare viewer**, not multi-instance:

| Piece | File | Notes |
|---|---|---|
| Web route | `web/src/App.tsx:276` (`/compare`) | State-passed ids, bounces back if reloaded/deep-linked |
| Web page | `web/src/pages/Compare.tsx` (64 ln) | Slim top bar + 2 panes, portrait/landscape flex |
| Web pane | `web/src/components/compare/ComparePane.tsx` (228 ln) | Reuses useViewerMedia, independent zoom/pan |
| Web entry | `web/src/pages/Gallery.tsx:391-395` | Compare button when exactly 2 selected |
| Shared helpers | `web/src/utils/compare.ts` + test | `canCompare` / `compareTargets` — keep these |
| Android route | `Screen.kt` `Screen.Compare` (`compare/{firstId}/{secondId}`) + `NavGraph.kt` wiring |
| Android screen | `ui/screens/viewer/CompareScreen.kt` (239 ln) | 2× PhotoPageContent, per-pane ExoPlayer |
| Android entry | `GalleryScreen.kt:186-196` (`onCompareClick`) | Compare button when exactly 2 selected |
| Android helpers | `ui/components/SelectionState.kt` `canCompare`/`compareTargets` + test — keep these |

Limitations of the bespoke approach (why we're replacing it): each pane is a
stripped viewer — no swiping to other photos, no edit, no info panel, no independent
navigation. Every viewer feature added later needs re-plumbing into ComparePane /
CompareScreen. Multi-instance makes all of that free, forever.

---

## 2. Findings — Android multi-instance feasibility: **GREEN, no under-the-hood changes needed**

Verified in `android/app/src/main/AndroidManifest.xml` and `MainActivity.kt`:

- **`launchMode` is not set → `standard`** ✅ — multiple instances of `MainActivity`
  are allowed. (Any `singleTask`/`singleInstance` would have killed this idea.)
- **`android:resizeableActivity` is not declared → defaults to `true`** for
  targetSdk ≥ 24 (we're targetSdk 34, minSdk 26) ✅. Split-screen already works;
  should be declared explicitly so nobody "cleans it up" later.
- **No `screenOrientation` lock** ✅.
- **`configChanges` already includes `screenSize|smallestScreenSize|screenLayout|orientation`**
  (`AndroidManifest.xml:60`, added for biometric TODO #17) ✅ — multi-window
  resizes will NOT recreate the activity, so no re-lock storms while dragging the divider.
- **Single process, everything app-wide is a Hilt singleton** (Room DB, repositories,
  DataStore, WorkManager, Coil, discovery client). Two activity instances share one
  process by default → same DB handle, same caches, same auth. Literally zero data-layer
  work. ExoPlayer is already per-screen (CompareScreen proved two players coexist).
- **minSdk 26** ✅ — split-screen (API 24) and `FLAG_ACTIVITY_LAUNCH_ADJACENT` (API 24)
  are available on every supported device.

The launch mechanic (the well-trodden "New window" pattern used by Chrome/Samsung Internet):

```kotlin
Intent(context, MainActivity::class.java).apply {
    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK
        or Intent.FLAG_ACTIVITY_MULTIPLE_TASK      // force a second task, don't resume the first
        or Intent.FLAG_ACTIVITY_LAUNCH_ADJACENT)   // fill the other split-screen pane
    putExtra(EXTRA_START_ROUTE, Screen.PhotoViewer.createRoute(photoId)) // optional deep-link
}
```

### Android gotchas found (must be handled in the plan)

1. **`FLAG_ACTIVITY_LAUNCH_ADJACENT` only tiles when already in multi-window mode**
   (stock Android). From fullscreen it just creates a second task in Recents; the user
   then arranges split-screen via Recents. Samsung One UI is more permissive. Mitigation:
   check `activity.isInMultiWindowMode` — if false, show a toast
   ("Opened second window — use Recents to arrange split screen"). Not a blocker.
2. **Biometric gate re-fires per activity instance.** `authenticated` is a per-activity
   `remember` in `MainActivity.kt:115` — window #2 would demand a fingerprint again 2s
   after the user just unlocked window #1. Same process, same session ⇒ hoist the unlock
   flag to a process-scoped holder (`@Singleton UnlockSession` or companion object).
   Process death still cold-starts and re-locks — preserves the documented #17 contract.
3. **Recents dedup/labeling:** with `MULTIPLE_TASK` both tasks show in Recents with the
   same label. Acceptable; optionally add `FLAG_ACTIVITY_NEW_DOCUMENT` if device testing
   shows launchers coalescing the tasks. VERIFY ON DEVICE (S21+ harness `.device-test\dev.ps1`).
4. **Memory:** two galleries = ~2× Coil working set. `largeHeap` already set; Coil cache
   is a shared singleton so thumbs are only decoded once. Watch during device verify.
5. **SSE / sync:** Android has no SSE client yet (deferred, TODO #11 memory) — nothing doubles.

---

## 3. Findings — Web feasibility: **GREEN via `window.open`, BLOCKED via iframes**

- **Auth tokens live in `localStorage`** (`store/auth.ts`) → per-origin, shared by every
  tab/window. A second window is already logged in ✅.
- **E2E master key**: non-extractable `CryptoKey` persisted in **IndexedDB** (per-origin,
  shared ✅) with only a `"present"` flag in `sessionStorage` (`crypto/crypto.ts` KEY_FLAG).
  Critical mechanic: **`window.open()` creates an auxiliary browsing context, which per
  the HTML spec receives a COPY of the opener's `sessionStorage`** → the flag is present
  → `loadKeyFromSession()` loads the key from IndexedDB → second window decrypts photos
  with **zero re-auth**. A manually-opened fresh tab has no flag and would bounce to
  login even though the key sits in IndexedDB — see hardening item P2.3.
  - ⚠️ **Do NOT pass `noopener`** on this window.open: it makes the context
    non-auxiliary and kills the sessionStorage copy. It's our own same-origin app;
    opener access is not a risk here.
- **Secure-album unlock token** is also `sessionStorage` (`utils/galleryToken.ts`,
  deliberately session-scoped) → copied into the opened window the same way. Window #2
  inherits the unlock; independent re-lock per window afterwards. Acceptable & arguably correct.
- **In-app split via two `<iframe>`s is BLOCKED by our own security headers**:
  `server/src/security.rs:55` sends `X-Frame-Options: DENY` and `:87` CSP
  `frame-ancestors 'none'`. Making an in-app dual-iframe shell work would require
  relaxing to `SAMEORIGIN` / `frame-ancestors 'self'`. **Recommendation: do NOT weaken
  clickjacking protection for a layout convenience.** Use real OS windows instead
  (Windows Snap / macOS tiling / browser split-view all handle two app windows fine).
- **SSE** (`useSyncEvents`, `/api/sync/events`): each window opens its own connection.
  Server treats connections independently — fine, just double the (tiny) cost.
- **Desktop auto-tiling bonus:** `window.open` with size/position features
  (`left=0,width=availWidth/2,…`) can place the two windows side-by-side immediately —
  a genuinely better "split screen" than the old fixed 50/50 panes.

---

## 4. The plan

### Phase 0 — decisions (pick before coding)
- [ ] **D1: Delete the bespoke Compare viewer or keep both?**
      Recommendation: DELETE (Compare.tsx, ComparePane.tsx, CompareScreen.kt,
      Screen.Compare/NavGraph wiring, `/compare` route). It's uncommitted-to-main-yet
      dead weight the moment multi-window ships, and it's only on `dev`. Keep the
      `canCompare`/`compareTargets` helpers + tests on both platforms — the gallery
      button still gates on "exactly 2 selected".
- [ ] **D2: Biometric on window #2** — recommendation: suppress via process-scoped
      unlock session (same process = same trust boundary; process death still re-locks).
- [ ] **D3: iframe split on web** — recommendation: NO (keep `frame-ancestors 'none'`).

### Phase 1 — Android multi-instance
- [ ] 1.1 Manifest: declare `android:resizeableActivity="true"` on MainActivity with a
      comment explaining multi-window is a feature, not an accident.
- [ ] 1.2 New `openInNewWindow(context, route: String?)` helper (e.g. in `ui/navigation/`):
      builds the NEW_TASK|MULTIPLE_TASK|LAUNCH_ADJACENT intent, optional
      `EXTRA_START_ROUTE` extra. Log the launch (error-path logging rule applies).
- [ ] 1.3 MainActivity → NavGraph: honor `EXTRA_START_ROUTE` as the start destination
      AFTER the biometric + permission gates. Validate the route string against known
      `Screen` routes — never navigate to an arbitrary extra (exported activity!).
- [ ] 1.4 Process-scoped `UnlockSession` (per D2): first successful biometric unlock
      marks the process unlocked; `MainActivity` consults it before prompting.
      Cleared on process death by construction. Unit-test the holder.
- [ ] 1.5 Rewire gallery Compare button (`GalleryScreen.kt` `onCompareClick`):
      selection of exactly 2 → navigate THIS window to `photo_viewer/{a}` +
      `openInNewWindow(photo_viewer/{b})`. If `!isInMultiWindowMode`, toast the
      Recents hint (see gotcha #1). Also add a general "New window" overflow action
      in the gallery top bar (the feature is useful beyond comparing).
- [ ] 1.6 Delete `CompareScreen.kt`, `Screen.Compare`, NavGraph compare wiring (per D1).
- [ ] 1.7 Unit tests: route-validation for EXTRA_START_ROUTE (reject unknown/garbage),
      UnlockSession semantics, intent-flag builder.

### Phase 2 — Web second window
- [ ] 2.1 New `openInNewWindow(path, tile: "left"|"right"|null)` util (`utils/window.ts`):
      `window.open(origin + path, "_blank", features)` — WITHOUT `noopener` (see §3);
      compute tiling features from `screen.availWidth/Height` when `tile` is set.
      Unit-test the feature-string/URL building.
- [ ] 2.2 Rewire gallery Compare button (`Gallery.tsx:391`): pair → navigate current
      window to `/photo/{a}`, `openInNewWindow("/photo/{b}", "right")` (and optionally
      re-tile self to the left half via `window.resizeTo/moveTo` — best-effort, browsers
      may deny; ignore failures silently but log). Popup blockers: this runs in the
      click handler (user gesture) so it opens; if `window.open` returns null, toast
      the user to allow popups — that's an error path, log it.
- [ ] 2.3 Hardening: `loadKeyFromSession()` — when the sessionStorage flag is absent,
      probe IndexedDB for the key anyway before declaring "no key" (flag is a cache,
      IndexedDB is the truth). Makes manually-opened second tabs work too. Add a test.
- [ ] 2.4 Delete `/compare` route, `Compare.tsx`, `ComparePane.tsx` (per D1); keep
      `utils/compare.ts` + test.
- [ ] 2.5 E2E (playwright): select 2 → Compare → assert second page opens, is
      authenticated without login redirect, and renders the target photo. Assert the
      secure-gallery token copy behavior (unlock in window 1 → open window 2 → still unlocked).

### Phase 3 — verify & close out
- [ ] 3.1 Web: vitest green, `tsc` clean, E2E green.
- [ ] 3.2 Android: unit tests green; **ON-DEVICE VERIFY (S21+)**: split-screen with two
      instances, drag divider (no re-lock, no recreation), two videos playing at once,
      delete-in-one-window reflects in the other (shared Room), Recents shows sane tasks,
      memory stays under control. Batch with the outstanding #17/#20 device verifies.
- [ ] 3.3 Update TODO.md (#21 entry) — bespoke Compare replaced by multi-window; note
      the header decision (D3) explicitly so nobody relaxes frame-ancestors later
      without reading this.
- [ ] 3.4 Conventional commit on `dev`, reference #21.

---

## 5. Risks / open questions

- Stock-Android fullscreen launch won't auto-enter split-screen (gotcha #1) — UX is
  "second window appears, user snaps it". Samsung devices (our test hardware) behave better.
- OEM Recents/launcher variance for same-app multi-task — device verify, fall back to
  `FLAG_ACTIVITY_NEW_DOCUMENT` if tasks coalesce.
- Browser popup-blocker or COOP changes could break window.open inheritance in the
  future — 2.3's IndexedDB fallback is the safety net.
- Both Android windows editing the SAME photo simultaneously: last-write-wins at the
  server, same as web+Android today. Not new, not a blocker.
