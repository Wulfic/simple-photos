import { create } from "zustand";

interface ProcessingState {
  /** Set of active processing task keys (e.g. "import", "encryption", "recovery") */
  tasks: Set<string>;
  /** Whether any task is currently processing */
  isProcessing: boolean;
  /** Start a named processing task */
  startTask: (key: string) => void;
  /** End a named processing task */
  endTask: (key: string) => void;
}

export const useProcessingStore = create<ProcessingState>((set) => ({
  tasks: new Set(),
  isProcessing: false,
  startTask: (key) =>
    set((state) => {
      const next = new Set(state.tasks);
      next.add(key);
      return { tasks: next, isProcessing: true };
    }),
  endTask: (key) =>
    set((state) => {
      const next = new Set(state.tasks);
      next.delete(key);
      return { tasks: next, isProcessing: next.size > 0 };
    }),
}));
