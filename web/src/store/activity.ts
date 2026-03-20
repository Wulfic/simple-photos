import { create } from "zustand";
import { api } from "../api/client";
import { useProcessingStore } from "./processing";

/**
 * Global activity store — tracks conversion progress across the entire app
 * lifecycle. Polls the server periodically so banners survive page navigation.
 */

interface ActivityState {
  // ── Conversion ────────────────────────────────────────────────────────
  /** Number of photos that still need conversion (web preview / thumbnail). */
  conversionPending: number;
  /** Encrypted items needing conversion but no key is available yet. */
  conversionAwaitingKey: number;
  conversionMissingThumbs: number;
  /** True while the server’s background converter is actively processing */
  conversionActive: boolean;

  // ── Polling control ───────────────────────────────────────────────
  polling: boolean;

  // ── Actions ───────────────────────────────────────────────────────
  setConversion: (pending: number, awaitingKey: number, missingThumbs: number, active: boolean) => void;
  setPolling: (v: boolean) => void;
}

export const useActivityStore = create<ActivityState>((set) => ({
  conversionPending: 0,
  conversionAwaitingKey: 0,
  conversionMissingThumbs: 0,
  conversionActive: false,

  polling: false,

  setConversion: (pending, awaitingKey, missingThumbs, active) =>
    set({ conversionPending: pending, conversionAwaitingKey: awaitingKey, conversionMissingThumbs: missingThumbs, conversionActive: active }),
  setPolling: (v) => set({ polling: v }),
}));

// ── Singleton poller ────────────────────────────────────────────────────────

let pollTimer: ReturnType<typeof setInterval> | null = null;
let refreshCallback: (() => void) | null = null;

/** Register an optional callback to run when conversion finishes (e.g. refresh gallery). */
export function setConversionDoneCallback(cb: (() => void) | null) {
  refreshCallback = cb;
}

async function tick() {
  const store = useActivityStore.getState();
  const processing = useProcessingStore.getState();

  // ── Conversion status ────────────────────────────────────────────────
  try {
    const cs = await api.admin.conversionStatus();
    const wasPending =
      store.conversionPending > 0 || store.conversionMissingThumbs > 0 || store.conversionActive;
    const nowPending = cs.pending_conversions > 0 || cs.missing_thumbnails > 0 || cs.converting;

    console.log(
      `[DIAG:POLL] conversion: pending=${cs.pending_conversions} awaitingKey=${cs.pending_awaiting_key ?? 0} thumbs=${cs.missing_thumbnails} converting=${cs.converting} | wasPending=${wasPending} nowPending=${nowPending}`
    );

    store.setConversion(cs.pending_conversions, cs.pending_awaiting_key ?? 0, cs.missing_thumbnails, cs.converting);

    // Sync processing store for profile icon ring
    if (nowPending && !processing.tasks.has("conversion")) {
      processing.startTask("conversion");
    } else if (!nowPending && processing.tasks.has("conversion")) {
      processing.endTask("conversion");
    }

    // If conversion just finished, fire the callback
    if (wasPending && !nowPending && refreshCallback) {
      refreshCallback();
    }
  } catch {
    // Ignore — endpoint may not be available
  }

}

/** Start global polling. Safe to call multiple times — only one poller runs. */
export function startActivityPolling() {
  if (pollTimer) return;
  useActivityStore.getState().setPolling(true);
  // Immediate first tick
  tick();
  // Then every 3 seconds for responsive feedback
  pollTimer = setInterval(tick, 3_000);
}

/** Stop global polling. */
export function stopActivityPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  useActivityStore.getState().setPolling(false);
}
