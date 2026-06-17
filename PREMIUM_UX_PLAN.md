# Premium UX Plan — Transitions, Loading States & Design Consistency

**Created:** 2026-06-17 · **Branch target:** `dev` · **Status:** planned (not started)

Goal: kill the "2000s hard-swap" feel. Make navigation feel integrated (smooth
transitions), make loading feel intentional (skeletons, not blank spinners), and
finish migrating the remaining UI primitives to the `#14` facelift design system
so banners / toggles / selects / inputs all match the current look and feel.

## Decisions (locked)

- **Transition style:** subtle crossfade (~200ms), header stays fixed. No slide,
  no shared-element morphs (can revisit later).
- **Approach:** native **View Transitions API** via React Router 6.30's
  `viewTransition` option. **No new runtime dependencies** (no framer-motion).
- **Top progress bar:** DEFERRED. Needs `useNavigation` (data router) or a
  store-driven hack — out of scope for now.
- **Skeletons over spinners** for content fetches; keep spinners ONLY for
  blocking actions (login/register/setup submit, save buttons).

## Key technical gotcha

The default view transition snapshots the whole document (`::view-transition*(root)`),
including the `fixed` AppHeader (web/src/components/AppHeader.tsx:135). Without
isolating it, the header crossfades/flickers on every nav. Fix: give the header
its own `view-transition-name` so it's excluded, and let only the page-content
wrapper animate.

---

## Phase 1 — Foundation (primitives + plumbing)

- [ ] 1.1 Add base view-transition CSS to web/src/index.css:
      `::view-transition-old(page)` / `::view-transition-new(page)` crossfade
      keyframes (~200ms ease). Wrap ALL of it in
      `@media (prefers-reduced-motion: no-preference)` so reduced-motion users
      get instant swaps.
- [ ] 1.2 Assign `view-transition-name: app-header` to the fixed header and
      `view-transition-name: page` to the page-content wrapper (a shared layout
      wrapper rendered inside ProtectedLayout's `<Outlet/>` area, or a small
      `<PageContainer>` each page opts into).
- [ ] 1.3 Build web/src/components/ui/Skeleton.tsx — shimmer primitive:
      dark-mode aware, reduced-motion aware (static gray when reduced), uses
      existing tokens (rounded, `shadow-card`/`bg-gray-*`). Variants: `text`,
      `rect`, `circle`, configurable w/h. Export from ui/index.ts.

## Phase 2 — Page transitions

- [ ] 2.1 Centralize navigation: add `useAppNavigate()` wrapper (or pass
      `{ viewTransition: true }`) so every `navigate()` call opts in
      consistently. Primary call sites: AppHeader.tsx:166 (nav items) + logo +
      dropdown items; per-page `navigate()` calls.
- [ ] 2.2 Confirm graceful no-op where View Transitions unsupported (older
      Safari/Firefox) — React Router falls back to instant nav, verify no errors.
- [ ] 2.3 Unify the existing Viewer `slide-in-left/right` animations
      (index.css:45-74) with the VT system OR leave them scoped to the viewer
      (they're intra-page, not route transitions — likely leave as-is, note it).

## Phase 3 — Skeleton loading states (content fetches)

- [ ] 3.1 Compose page skeletons matching REAL layout (no generic boxes):
      - `GallerySkeleton` — grid mirroring JustifiedGrid aspect ratios
        (web/src/components/gallery/JustifiedGrid.tsx)
      - `AlbumGridSkeleton` — album card grid
      - `SettingsSkeleton` — section cards
      - `ListSkeleton` — Trash / Search results
- [ ] 3.2 Replace full-page `animate-spin` ring spinners with the matching
      skeleton in: Gallery, Albums, AlbumDetail, Search, Trash, Settings,
      SecureGallery. (Loading flags already exist via useGalleryData etc.)
- [ ] 3.3 KEEP spinners for blocking actions: Login, Register, Setup submit;
      save/confirm buttons. Do NOT skeleton these.

## Phase 4 — Perceived-perf polish

- [ ] 4.1 Code-split routes in web/src/App.tsx with `React.lazy` + `Suspense`;
      use the matching skeleton as each route's Suspense fallback. Faster first
      paint + intentional loading. Keep auth/guard logic intact.
- [ ] 4.2 Fade-in thumbnails on decode (blur-up) in
      web/src/gallery/components/ThumbnailTile.tsx instead of pop-in.

## Phase 5 — Design-system consistency (banners, toggles, selects, inputs)

Finish the `#14` facelift migration so everything matches the current look/feel.

- [ ] 5.1 **Banners** — audit all 6 and unify on the `card` + accent design
      language (EncryptionBanner already uses `card shadow-card-hover`; bring the
      rest in line): ConversionBanner, SavingBanner, AiBanner, GeoBanner,
      ServerOfflineBanner. Consistent position, padding, icon, progress bar,
      dismiss button, dark-mode treatment. Consider a shared `<ProgressBanner>`
      primitive to dedupe.
- [ ] 5.2 **Toggle switches** — migrate remaining hand-rolled
      `<button role="switch">` blocks and raw checkboxes to the `<Toggle>`
      primitive (web/src/components/ui/Toggle.tsx). Audit the ~32 files flagged
      with raw switch/input usage; prioritize settings/* + welcome/* steps.
- [ ] 5.3 **Select / dropdown boxes** — create a `<Select>` primitive in
      web/src/components/ui/ with the recessed look (matching `.input`: inset
      shadow, accent focus ring) + custom chevron + dark-mode. Replace raw
      `<select>` elements across settings/*, diagnostics/*, welcome/*, pages/*.
- [ ] 5.4 **Input / textarea boxes** — ensure every `<input>`/`<textarea>` uses
      the `.input` recipe (or a new `<Input>`/`<Textarea>` primitive wrapping it)
      so all fields get the carved-in look + accent focus. Sweep the ~32 files
      with raw `<input>` usage.

## Phase 6 — Verify

- [ ] 6.1 Manual run (Chromium): no header flicker, no layout shift on nav,
      skeletons visually match the real layout that replaces them.
- [ ] 6.2 Run with a reduced-motion profile: transitions become instant,
      shimmer becomes static.
- [ ] 6.3 `npm run build` (tsc -b + vite build) clean; no `as any`/`@ts-ignore`.
- [ ] 6.4 Spot-check dark mode on every changed surface (banners, selects,
      inputs, toggles, skeletons).

---

## Reference

- React Router view transitions: https://reactrouter.com/how-to/view-transitions
- Skeleton vs spinner UX: https://www.onething.design/post/skeleton-screens-vs-loading-spinners
- Design tokens live in web/tailwind.config.js (shadows) + web/src/index.css
  (`@layer components`: btn/card/input/toggle/stat-tile recipes).
