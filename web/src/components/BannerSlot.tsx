/**
 * BannerSlot — portals a single global progress banner into the shared,
 * bottom-anchored flex column owned by {@link BannerHost}.
 *
 * Positioning is now pure flex layout: every banner is a full-width flex item in
 * one `flex-col-reverse` container with a real `gap`, so cards stack vertically
 * with consistent spacing and never overlap — regardless of card height or
 * viewport (item #3). Vertical order is set by CSS `order` = the banner's
 * priority; absent banners just don't render, so the column compacts with no
 * gaps.
 *
 * Entrance animation stays pure CSS: the item mounts a hair lower and
 * transparent, then a rAF flips it to its settled state so it slides up and
 * fades in. When siblings come or go, the flex reflow glides the rest.
 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { BannerId, useBannerContainer } from "../store/bannerStack";

export function BannerSlot({
  id: _id,
  priority,
  children,
}: {
  id: BannerId;
  priority: number;
  children: React.ReactNode;
}) {
  const containerEl = useBannerContainer((s) => s.el);
  const [entered, setEntered] = useState(false);

  // Flip to the settled state on the next frame so the browser paints the
  // initial (lower + transparent) state first and the transition runs.
  useEffect(() => {
    const raf = requestAnimationFrame(() => setEntered(true));
    return () => cancelAnimationFrame(raf);
  }, []);

  // Nothing to portal into until BannerHost has mounted.
  if (!containerEl) return null;

  return createPortal(
    <div
      className="w-full flex justify-center"
      style={{
        // CSS order controls vertical position within the flex-col-reverse
        // stack (lower priority → nearer the bottom anchor).
        order: priority,
        opacity: entered ? 1 : 0,
        transform: entered ? "translateY(0)" : "translateY(0.5rem)",
        transition: "transform 300ms ease-out, opacity 300ms ease-out",
      }}
    >
      {children}
    </div>,
    containerEl,
  );
}
