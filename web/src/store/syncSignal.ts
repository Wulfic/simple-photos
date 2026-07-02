/**
 * Real-time sync signal (item #11).
 *
 * {@link useSyncEvents} subscribes to the server's `/api/sync/events` SSE stream
 * and calls `bump()` whenever the server reports an album/gallery change. Data
 * hooks watch `version` and refetch, so a change made on one device shows up on
 * another within seconds — far faster than the periodic background sync, which
 * remains the offline fallback.
 *
 * Conflict resolution is timestamp-based / last-write-wins and lives on the pull
 * side (the server record's `updated_at`/`taken_at` wins); the signal only tells
 * clients *when* to pull.
 */
import { create } from "zustand";

interface SyncSignalState {
  /** Monotonic counter; bumped on every received change event. */
  version: number;
  /** The last change kind observed ("photo" | "album" | "trash" | "resync"). */
  lastKind: string | null;
  bump: (kind: string) => void;
}

export const useSyncSignal = create<SyncSignalState>((set) => ({
  version: 0,
  lastKind: null,
  bump: (kind) => set((s) => ({ version: s.version + 1, lastKind: kind })),
}));
