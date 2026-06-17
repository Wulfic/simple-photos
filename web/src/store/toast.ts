/**
 * Zustand store for global toast / snackbar notifications.
 *
 * Replaces the per-page "red line under the navbar" error pattern (#8) with a
 * single dismissible popup stack mounted once in `ProtectedLayout`. Any page
 * can surface a user-facing message via the exported `toast` helper without
 * wiring its own inline `<p>` error bar (which caused layout shift and was easy
 * to miss).
 *
 * Usage:
 *   import { toast } from "../store/toast";
 *   toast.error("Cannot add yourself as a member");
 *   toast.success("Album shared");
 */
import { create } from "zustand";

export type ToastKind = "error" | "success" | "info";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
  /** Auto-dismiss delay in ms. 0 disables auto-dismiss (manual close only). */
  duration: number;
}

interface ToastState {
  toasts: Toast[];
  /** Add a toast and return its id (so callers can dismiss it early). */
  push: (kind: ToastKind, message: string, duration?: number) => number;
  /** Remove a toast by id. */
  dismiss: (id: number) => void;
  /** Remove all toasts. */
  clear: () => void;
}

// Monotonic id counter — survives across renders without colliding.
let nextId = 1;

// Default auto-dismiss timings. Errors linger a little longer so they are not
// missed; success/info are transient.
const DEFAULT_DURATION: Record<ToastKind, number> = {
  error: 6000,
  success: 4000,
  info: 4000,
};

export const useToastStore = create<ToastState>((set) => ({
  toasts: [],
  push: (kind, message, duration) => {
    const id = nextId++;
    const d = typeof duration === "number" ? duration : DEFAULT_DURATION[kind];
    set((state) => {
      // De-dupe: if an identical message of the same kind is already showing,
      // don't stack a second copy (e.g. a retry loop hitting the same error).
      if (state.toasts.some((t) => t.kind === kind && t.message === message)) {
        return state;
      }
      return { toasts: [...state.toasts, { id, kind, message, duration: d }] };
    });
    return id;
  },
  dismiss: (id) =>
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
  clear: () => set({ toasts: [] }),
}));

/**
 * Imperative helper for non-component call sites (catch blocks, effects).
 * Trims/ignores empty messages so `toast.error("")` (used to *clear* legacy
 * inline error state) is a no-op rather than an empty popup.
 */
export const toast = {
  error: (message: string, duration?: number) => {
    const m = (message ?? "").trim();
    if (!m) return -1;
    return useToastStore.getState().push("error", m, duration);
  },
  success: (message: string, duration?: number) => {
    const m = (message ?? "").trim();
    if (!m) return -1;
    return useToastStore.getState().push("success", m, duration);
  },
  info: (message: string, duration?: number) => {
    const m = (message ?? "").trim();
    if (!m) return -1;
    return useToastStore.getState().push("info", m, duration);
  },
  dismiss: (id: number) => useToastStore.getState().dismiss(id),
};
