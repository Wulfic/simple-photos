import { create } from "zustand";
import { api } from "../api/client";
import { useProcessingStore } from "./processing";

/**
 * Global activity store — tracks conversion and migration progress across
 * the entire app lifecycle. Polls the server periodically so banners survive
 * page navigation.
 */

interface ActivityState {
  // ── Conversion ────────────────────────────────────────────────────────
  conversionPending: number;
  conversionMissingThumbs: number;
  /** True while the server’s background converter is actively processing */
  conversionActive: boolean;

  // ── Encryption migration ──────────────────────────────────────────
  migrationStatus: string; // "idle" | "encrypting" | "decrypting"
  migrationTotal: number;
  migrationCompleted: number;
  migrationError: string | null;

  // ── Polling control ───────────────────────────────────────────────
  polling: boolean;

  // ── Actions ───────────────────────────────────────────────────────
  setConversion: (pending: number, missingThumbs: number, active: boolean) => void;
  setMigration: (status: string, total: number, completed: number, error: string | null) => void;
  setPolling: (v: boolean) => void;
}

export const useActivityStore = create<ActivityState>((set) => ({
  conversionPending: 0,
  conversionMissingThumbs: 0,
  conversionActive: false,

  migrationStatus: "idle",
  migrationTotal: 0,
  migrationCompleted: 0,
  migrationError: null,

  polling: false,

  setConversion: (pending, missingThumbs, active) =>
    set({ conversionPending: pending, conversionMissingThumbs: missingThumbs, conversionActive: active }),
  setMigration: (status, total, completed, error) =>
    set({
      migrationStatus: status,
      migrationTotal: total,
      migrationCompleted: completed,
      migrationError: error,
    }),
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
    store.setConversion(cs.pending_conversions, cs.missing_thumbnails, cs.converting);

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

  // ── Encryption migration status ──────────────────────────────────────
  try {
    const es = await api.encryption.getSettings();
    store.setMigration(
      es.migration_status,
      es.migration_total,
      es.migration_completed,
      es.migration_error ?? null,
    );

    // Sync processing store for profile icon ring
    const migActive = es.migration_status === "encrypting" || es.migration_status === "decrypting";
    if (migActive && !processing.tasks.has("encryption")) {
      processing.startTask("encryption");
    } else if (!migActive && processing.tasks.has("encryption")) {
      processing.endTask("encryption");
    }
  } catch {
    // Ignore
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
