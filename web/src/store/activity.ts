import { create } from "zustand";

/**
 * Global activity store — placeholder after conversion removal.
 *
 * Previously tracked conversion progress. Now only exposes the polling
 * start/stop API so existing imports in App.tsx don't break.
 * Can be extended in the future for other background tasks.
 */

interface ActivityState {
  polling: boolean;
  setPolling: (v: boolean) => void;
}

export const useActivityStore = create<ActivityState>((set) => ({
  polling: false,
  setPolling: (v) => set({ polling: v }),
}));

/** Start global polling (no-op — no background tasks to poll). */
export function startActivityPolling() {
  useActivityStore.getState().setPolling(true);
}

/** Stop global polling (no-op). */
export function stopActivityPolling() {
  useActivityStore.getState().setPolling(false);
}
