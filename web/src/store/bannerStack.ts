/**
 * Banner-stack shared state.
 *
 * Global progress banners (encryption, conversion, AI, geo, …) all render into a
 * single fixed, bottom-anchored flex column owned by {@link BannerHost}. Because
 * they live in one flex container with a real gap, they stack vertically with
 * consistent spacing and **never overlap** regardless of each card's height or
 * the viewport size (the old design offset each `fixed` card by a hard-coded
 * step that was smaller than the card, so tall cards overlapped — item #3).
 *
 * Ordering within the stack is driven purely by CSS `order` = the priority
 * below; missing banners simply don't render, so the column compacts with no
 * gaps and no registry bookkeeping.
 */
import { create } from "zustand";

/**
 * Stable bottom→top ordering of the global banners, expressed as priorities and
 * applied via CSS `order` inside a `flex-col-reverse` container: lower priority
 * sits lower in the stack (closer to the bottom anchor). Keeping the
 * highest-signal banner (encryption) at priority 0 pins it to the bottom, where
 * it stays visible even when the stack scrolls.
 */
export const BANNERS = {
  encryption: 0,
  conversion: 10,
  saveCopy: 20,
  ai: 30,
  geo: 40,
  geoPrecise: 50,
} as const;

export type BannerId = keyof typeof BANNERS;

interface BannerContainerState {
  /** The live DOM node banners portal into, or null before {@link BannerHost} mounts. */
  el: HTMLElement | null;
  setEl: (el: HTMLElement | null) => void;
}

/**
 * Holds the shared container node. {@link BannerHost} publishes its `<div>` here
 * on mount; each {@link BannerSlot} subscribes and portals into it once present.
 */
export const useBannerContainer = create<BannerContainerState>((set) => ({
  el: null,
  setEl: (el) => set({ el }),
}));
