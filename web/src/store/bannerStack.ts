/**
 * Banner-stack registry — assigns each visible global progress banner a
 * *compacted* slot in a bottom-anchored stack.
 *
 * Previously every banner hard-coded a Tailwind `bottom-*` slot tied to its
 * TYPE, so an inactive middle banner (e.g. geo) left a visible hole between the
 * banners above and below it. This registry instead hands out slot indices
 * based on which banners are *actually rendering right now*, so the stack always
 * compacts to the bottom with no gaps and grows upward as more appear.
 *
 * Each banner registers itself (via {@link useBannerSlot}) only while it is
 * mounted/visible. We deliberately do NOT derive slots from `useProcessingStore`
 * because a dismissed banner keeps its task active there (polling continues to
 * drive the nav-bar spinner) even though it is no longer drawn — that would
 * create phantom slots.
 */
import { useEffect } from "react";
import { create } from "zustand";

/**
 * Stable bottom→top ordering of the global banners, expressed as priorities.
 * Lower priority sits lower in the stack (closer to the bottom anchor). Slot
 * indices are derived from these, so banners never reshuffle relative to each
 * other — they only compact toward the bottom as siblings come and go.
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

interface BannerStackState {
  /** id → priority for every banner currently rendering. */
  active: Record<string, number>;
  register: (id: string, priority: number) => void;
  unregister: (id: string) => void;
}

export const useBannerStackStore = create<BannerStackState>((set) => ({
  active: {},
  register: (id, priority) =>
    set((s) =>
      s.active[id] === priority
        ? s
        : { active: { ...s.active, [id]: priority } },
    ),
  unregister: (id) =>
    set((s) => {
      if (!(id in s.active)) return s;
      const next = { ...s.active };
      delete next[id];
      return { active: next };
    }),
}));

/**
 * Register a banner while it is visible and return its compacted slot index
 * (0 = bottom-most). The slot is the number of *other* active banners that sit
 * below it (strictly lower priority), so the stack stays gap-free.
 *
 * Mount this hook only from a component that renders solely when the banner is
 * visible — its mount/unmount lifecycle is what drives registration.
 */
export function useBannerSlot(id: BannerId, priority: number): number {
  const register = useBannerStackStore((s) => s.register);
  const unregister = useBannerStackStore((s) => s.unregister);

  useEffect(() => {
    register(id, priority);
    return () => unregister(id);
  }, [id, priority, register, unregister]);

  // Recomputes on any registry change; returns a primitive so consumers only
  // re-render when their slot actually moves.
  return useBannerStackStore((s) => {
    let slot = 0;
    for (const [otherId, otherPriority] of Object.entries(s.active)) {
      if (otherId !== id && otherPriority < priority) slot++;
    }
    return slot;
  });
}
