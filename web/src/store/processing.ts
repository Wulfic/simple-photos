/**
 * Zustand store for background processing indicator state.
 *
 * Tracks active long-running tasks (import, encryption, upload, recovery)
 * and exposes a processing flag + human-readable label for the nav-bar
 * spinner. Tasks are added/removed by page components during async work.
 */
import { create } from "zustand";

/** Human-readable labels for known task keys */
const TASK_LABELS: Record<string, string> = {
  import: "Importing",
  encryption: "Encrypting",
  conversion: "Converting",
  upload: "Uploading",
  recovery: "Recovering",
  backup: "Backing up",
  download: "Downloading",
  scan: "Scanning",
  saveCopy: "Saving copy",
};

interface ProcessingState {
  /** Set of active processing task keys (e.g. "import", "encryption", "recovery") */
  tasks: Set<string>;
  /** Whether any task is currently processing */
  isProcessing: boolean;
  /** Human-readable label for the current activity (first active task) */
  activeLabel: string | null;
  /** Start a named processing task */
  startTask: (key: string) => void;
  /** End a named processing task */
  endTask: (key: string) => void;
}

function labelFromTasks(tasks: Set<string>): string | null {
  if (tasks.size === 0) return null;
  const first = tasks.values().next().value!;
  return TASK_LABELS[first] ?? first;
}

export const useProcessingStore = create<ProcessingState>((set) => ({
  tasks: new Set(),
  isProcessing: false,
  activeLabel: null,
  startTask: (key) =>
    set((state) => {
      const next = new Set(state.tasks);
      next.add(key);
      return { tasks: next, isProcessing: true, activeLabel: labelFromTasks(next) };
    }),
  endTask: (key) =>
    set((state) => {
      const next = new Set(state.tasks);
      next.delete(key);
      return { tasks: next, isProcessing: next.size > 0, activeLabel: labelFromTasks(next) };
    }),
}));
