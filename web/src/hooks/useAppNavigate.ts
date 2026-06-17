/**
 * useAppNavigate — drop-in replacement for react-router's `useNavigate` that
 * opts every route change into the View Transitions API.
 *
 * The returned function has the same call shape as `useNavigate`'s result, so
 * call sites need no changes beyond swapping the hook:
 *   - `navigate("/gallery")`            → crossfades the page (see index.css)
 *   - `navigate("/x", { replace: true })` → options are forwarded, VT still on
 *   - `navigate(-1)` / `navigate(1)`    → delta (back/forward) nav, VT skipped
 *
 * `viewTransition: true` is a no-op in browsers without the View Transitions
 * API — React Router falls back to an instant navigation, and reduced-motion
 * users get instant swaps via the media-query gate in index.css.
 *
 * Use this inside the authenticated app (routes under ProtectedLayout, which
 * provides the `view-transition-name: page` wrapper). Auth-flow pages
 * (login/register/setup/welcome) intentionally keep the plain `useNavigate`.
 */
import { useCallback } from "react";
import { useNavigate, type NavigateOptions, type To } from "react-router-dom";

export function useAppNavigate() {
  const navigate = useNavigate();

  return useCallback(
    (to: To | number, options?: NavigateOptions) => {
      if (typeof to === "number") {
        // Delta navigation (history back/forward) — the destination snapshot
        // isn't ours to crossfade, so just defer to the router.
        navigate(to);
        return;
      }
      navigate(to, { viewTransition: true, ...options });
    },
    [navigate],
  );
}
