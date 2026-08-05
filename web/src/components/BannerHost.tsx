/**
 * BannerHost — the single fixed, bottom-anchored container that every global
 * progress banner ({@link BannerSlot}) portals into.
 *
 * `flex-col-reverse` runs the main axis bottom→top, so:
 *   - the first-ordered item (encryption, `order: 0`) sits at the very bottom
 *     and stays pinned there;
 *   - new banners appear above existing ones;
 *   - the default scroll position of a `column-reverse` scroll container is the
 *     bottom, so when more banners are active than fit, the most important
 *     (encryption) stays visible and the overflow scrolls off the top (item #3:
 *     "encryption banner remains visible and readable", plus max-height + scroll).
 *
 * `gap` provides consistent spacing with no overlap; `pointer-events-none` lets
 * clicks pass through the empty container, while each banner card re-enables
 * pointer events on itself.
 */
import { useBannerContainer } from "../store/bannerStack";

export default function BannerHost() {
  const setEl = useBannerContainer((s) => s.setEl);
  return (
    <div
      ref={setEl}
      aria-live="polite"
      className="fixed inset-x-0 safe-bottom-4 sm:safe-bottom-6 z-50 flex flex-col-reverse items-center gap-2 px-4 max-h-[calc(100dvh-5rem)] overflow-y-auto overflow-x-hidden pointer-events-none"
    />
  );
}
