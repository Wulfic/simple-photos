# TODO 2 — Replace bespoke Compare with true "open the app twice" split-screen (#21 follow-up)

> **STATUS (Jul 16): CODE COMPLETE on `dev`, uncommitted.** Phase 1 (Android) built in
> full. Phase 2 (web) built to the *scoped* extent — see the scope note below. Only the
> on-device verify (3.2) is outstanding; it needs the S21+ and is batched with #17/#20.
>
> **SCOPE CALL (user, Jul 16): split-screen is a PHONE feature only.** On a PC you just
> open the browser page twice, so the web side ships no split-screen UI at all: no
> `window.open` shell, no tiling, and the bespoke Compare page is simply deleted (2.1/2.2
> dropped, 2.5 moot — there is no web split UI to E2E, and no browser harness exists;
> `tests/` is API-level Python, not Playwright). What the web DID need was 2.3: a
> hand-opened second tab used to bounce to the password screen. That now works.

**Goal:** the split-screen feature should let the user literally run the app twice on
their device — two fully independent windows of the SAME app, each with the complete
experience (gallery, viewer, editing, info panel, albums, search — everything). No
bespoke two-pane viewer to maintain; the OS window manager does the splitting, and
nothing changes underneath the hood because each "pane" is just… the app.

Status: **INVESTIGATION DONE — NOT IMPLEMENTED.** Findings below, then the plan.

---

## 1. Findings — what USED to exist (commit `06a1f2e` "spilitscreeeen view")

⚠️ **HISTORICAL — every file in this table is now deleted** (D1). Kept as the record of
what was replaced. The implementation was a **bespoke two-pane Compare viewer**:

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

### Phase 0 — decisions (DECIDED)
- [x] **D1: DELETE the bespoke Compare viewer.** Done on both platforms.
      Android keeps `canCompare`/`compareTargets` + tests — the gallery button still
      gates on "exactly 2 selected", it just opens two windows now. Web deleted
      `utils/compare.ts` + its test TOO (deviation from the original note): with no web
      split UI, the helpers had zero callers. Dead code, not a "shared helper".
- [x] **D2: Biometric on window #2 — suppressed** via a process-scoped
      `@Singleton UnlockSession` (`security/UnlockSession.kt`). Same process = same trust
      boundary; process death still re-locks, preserving the #17 contract.
- [x] **D3: iframe split on web — NO.** `frame-ancestors 'none'` / `X-Frame-Options: DENY`
      stay exactly as they are (`server/src/security.rs`). Nothing in this work touched
      them, and nothing should: OS windows do the splitting.
- [x] **D4 (new, user-decided Jul 16): how a fresh web tab gets the E2E key.**
      Item 2.3 as originally written ("the flag is a cache, IndexedDB is the truth")
      would have silently destroyed the key's session scoping — the keystore also
      survives a browser restart, so a blind probe means the app decrypts everything
      days later with no password. CHOSEN: adopt the key **only while another tab is
      alive and unlocked**, confirmed over a BroadcastChannel handshake. Restart still
      requires the password. See `web/src/crypto/keySession.ts`.

### Phase 1 — Android multi-instance — **DONE (code)**
- [x] 1.1 Manifest: `android:resizeableActivity="true"` on MainActivity + a comment
      saying resizeable/`standard` launchMode are load-bearing, not leftovers.
- [x] 1.2 `ui/navigation/NewWindow.kt`: `openInNewWindow(context, route?)` builds the
      NEW_TASK|MULTIPLE_TASK|LAUNCH_ADJACENT intent + optional `EXTRA_START_ROUTE`;
      returns success and logs every failure path. Also `findActivity()` (for
      `isInMultiWindowMode`) and `startRouteFromIntent()`.
- [x] 1.3 MainActivity reads+validates the extra, passes it to `NavGraph(startRoute=)`,
      which pushes it on top of the gallery ONLY once the start destination resolves to
      the gallery — so it can't jump the login gate. Back still returns to the gallery.
- [x] 1.4 `security/UnlockSession.kt` — `@Singleton`, `isUnlocked`/`markUnlocked()`.
      Deliberately has no reset: the singleton dies with the process, which IS the
      intended lifetime.
- [x] 1.5 Gallery Compare button → `openInNewWindow(photo_viewer/{b})` + this window
      navigates to `photo_viewer/{a}`; toasts the Recents hint when
      `!isInMultiWindowMode`, and toasts on launch failure. "New Window" added to the
      header overflow menu (`HeaderNavigation.onNewWindowClick`, null ⇒ item omitted,
      so only the gallery shows it).
- [x] 1.6 Deleted `CompareScreen.kt`, `Screen.Compare`, the NavGraph wiring, and
      `PhotoViewerViewModel.loadPhotoByLocalId` (orphaned with it).
- [x] 1.7 Unit tests green: `StartRouteValidationTest` (whitelist; rejects
      `secure_gallery`/`settings`/traversal/extra-query smuggling/over-long ids),
      `NewWindowFlagsTest` (each flag present; CLEAR_TOP/SINGLE_TOP absent),
      `UnlockSessionTest`.

### Phase 2 — Web second window — **DONE, as scoped (see scope call at top)**
- [x] ~~2.1 `openInNewWindow(path, tile)` util~~ **DROPPED.** No web split UI: the user
      opens the browser page twice themselves, and the OS/browser tiles it.
- [x] ~~2.2 Rewire the web gallery Compare button~~ **DROPPED** — button deleted (2.4).
- [x] 2.3 **Done, but NOT as originally written** (see D4). `loadKeyFromSession()` is
      unchanged: no flag still means no key, which is what keeps the key scoped to a
      browser session. Instead:
      - `crypto/keySession.ts` — BroadcastChannel ping/pong. A tab holding the key
        answers "yes, the session is live". No key material ever crosses the channel;
        the asking tab reads its own IndexedDB. A tab can't vouch for itself (both
        channel objects in one tab DO hear each other — the tabId guard is load-bearing
        and tested).
      - `crypto/crypto.ts` — new `adoptKeyFromKeystore()`: reads the CryptoKey and
        restores the `sp_key` flag. Documented as safe ONLY behind a peer's say-so.
      - `crypto/restoreKey.ts` — `restoreKeyOnBoot()`, the whole boot decision tree,
        extracted from App.tsx so it's testable without a DOM (this repo has no
        jsdom/testing-library, and adding one for a 5-line component wasn't worth it).
        Skips the handshake entirely when logged out so the login page pays nothing.
      - `App.tsx` — starts the responder, gates first render on the restore, and
        **bounds it at 3s**: the restore now reads IndexedDB before rendering, so a
        hung keystore must degrade to today's behavior, not a permanent spinner.
      - Tests: `keySession.test.ts` (10, real BroadcastChannels), `crypto.test.ts` (7,
        real WebCrypto + fake-indexeddb, incl. tab-1-encrypts → tab-2-decrypts),
        `restoreKey.test.ts` (6, incl. "restart still asks for the password").
- [x] 2.4 Deleted `/compare` route, `Compare.tsx`, `ComparePane.tsx` — **and**
      `utils/compare.ts` + test (zero callers left on web; see D1).
- [x] ~~2.5 E2E (playwright)~~ **MOOT.** There is no web split UI to drive, and no
      browser harness exists (`tests/` is API-level Python; no Playwright anywhere in
      the repo). The behavior that mattered — tab 2 authenticates and decrypts without
      a login bounce — is covered by the unit tests above against real IndexedDB, real
      WebCrypto and real BroadcastChannels.

### Phase 3 — verify & close out
- [x] 3.1 Web: vitest green (126/126, 12 files), `tsc -b` clean, `vite build` clean.
      E2E: n/a, see 2.5.
- [ ] 3.2 Android: unit tests green (`gradlew testDebugUnitTest` BUILD SUCCESSFUL, main
      sources compile). **ON-DEVICE VERIFY (S21+) STILL OUTSTANDING** — the only thing
      left in this file. Batch with the #17/#20 device verifies:
      - two instances side by side; drag the divider → no re-lock, no recreation
      - biometric prompts ONCE across both windows (UnlockSession), but a cold start
        after swiping both away still prompts
      - Compare on 2 selected → this window shows A, second window shows B
      - "New Window" from the overflow menu; toast appears when launching from
        fullscreen (stock behavior — LAUNCH_ADJACENT only tiles from multi-window)
      - two videos playing at once; delete in one window reflects in the other (shared
        Room); Recents shows two sane tasks (if they coalesce → try
        `FLAG_ACTIVITY_NEW_DOCUMENT`, gotcha #3); memory sane (~2× Coil working set)
- [x] 3.3 Docs updated: this file. **D3 stands — do NOT relax `frame-ancestors 'none'` /
      `X-Frame-Options: DENY` in `server/src/security.rs` for a layout convenience.**
      (There is no repo-root TODO.md tracking #21; `todo1.md`/this file are the live
      lists, and the #21 history is in mem0.)
- [ ] 3.4 Conventional commit on `dev`, reference #21. **NOT DONE — deliberately.** The
      branch already carries a large pile of unrelated uncommitted work (album counts,
      Takeout albums); committing here would sweep it in. Stage this feature's files
      explicitly when committing.

---

## 5. Risks / open questions

- Stock-Android fullscreen launch won't auto-enter split-screen (gotcha #1) — UX is
  "second window appears, user snaps it". Samsung devices (our test hardware) behave
  better. Handled: toast tells the user where the window went.
- OEM Recents/launcher variance for same-app multi-task — device verify, fall back to
  `FLAG_ACTIVITY_NEW_DOCUMENT` if tasks coalesce.
- ~~Browser popup-blocker / COOP breaking window.open inheritance~~ — no longer a risk:
  the web ships no `window.open`, and the BroadcastChannel handshake doesn't care how
  the second tab was opened (2.3 / D4).
- Both Android windows editing the SAME photo simultaneously: last-write-wins at the
  server, same as web+Android today. Not new, not a blocker.
- Two Android windows = two of anything that was implicitly "once per activity". Audited:
  the data layer is all Hilt singletons and ExoPlayer was already per-screen. Android has
  no SSE client (#11 deferred), so nothing doubles there either.
