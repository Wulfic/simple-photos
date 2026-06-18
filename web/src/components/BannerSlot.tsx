/**
 * BannerSlot — fixed, bottom-anchored positioning + entrance animation for a
 * single global progress banner.
 *
 * All slots share the same bottom anchor and stack *upward* via a transformed
 * offset (`slot × STEP`), with a uniform step that leaves the cards slightly
 * overlapping — the look the previous hard-coded `bottom-*` design had, but now
 * gap-free because slots are compacted by {@link useBannerSlot}.
 *
 * Animation is pure CSS (the app uses native View Transitions + CSS, no
 * framer-motion):
 *  - On mount the banner starts a hair higher and transparent, then a rAF flips
 *    it to its settled slot — so it slides DOWN into place and fades in.
 *  - When siblings come or go, the slot index changes and the transformed
 *    offset transitions, so the whole stack glides instead of jumping.
 */
import { useEffect, useState } from "react";
import { BannerId, useBannerSlot } from "../store/bannerStack";

/** Bottom anchor of the lowest slot (matches the old `bottom-6`). */
const BASE_REM = 1.5;
/** Vertical distance between adjacent slots (≈ the old 12-unit overlap step). */
const STEP_REM = 3;
/** Extra lift the banner starts at before sliding down into its slot. */
const ENTER_LIFT_REM = 0.75;

export function BannerSlot({
  id,
  priority,
  children,
}: {
  id: BannerId;
  priority: number;
  children: React.ReactNode;
}) {
  const slot = useBannerSlot(id, priority);
  const [entered, setEntered] = useState(false);

  // Flip to the settled state on the next frame so the browser paints the
  // initial (lifted + transparent) state first and the transition runs.
  useEffect(() => {
    const raf = requestAnimationFrame(() => setEntered(true));
    return () => cancelAnimationFrame(raf);
  }, []);

  const settledOffset = slot * STEP_REM;
  const offsetRem = entered ? settledOffset : settledOffset + ENTER_LIFT_REM;

  return (
    <div
      className="fixed left-4 right-4 pointer-events-none"
      style={{
        bottom: `${BASE_REM}rem`,
        // Negative Y moves the banner up the stack from the bottom anchor.
        transform: `translateY(-${offsetRem}rem)`,
        opacity: entered ? 1 : 0,
        // Keep the whole stack at the banners' historical z-50 ceiling so the
        // FAB (z-60), modals and toasts still sit above it; within the stack the
        // bottom-most slot draws in front, matching the overlap direction.
        zIndex: 50 - slot,
        transition: "transform 300ms ease-out, opacity 300ms ease-out",
      }}
    >
      {children}
    </div>
  );
}
