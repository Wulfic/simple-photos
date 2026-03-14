/** Zustand store for dark/light theme preference (persists to localStorage). */
import { create } from "zustand";

type Theme = "light" | "dark";

interface ThemeState {
  theme: Theme;
  toggle: () => void;
  setTheme: (t: Theme) => void;
}

/**
 * Zustand store for light/dark mode, persisted to localStorage.
 * Default is dark mode.
 */
export const useThemeStore = create<ThemeState>((set) => ({
  theme: (localStorage.getItem("theme") as Theme) || "dark",
  /** Toggle between light and dark mode, persisting the choice to localStorage. */
  toggle: () =>
    set((s) => {
      const next = s.theme === "light" ? "dark" : "light";
      localStorage.setItem("theme", next);
      return { theme: next };
    }),
  /** Set the theme to a specific value, persisting to localStorage. */
  setTheme: (t: Theme) => {
    localStorage.setItem("theme", t);
    set({ theme: t });
  },
}));
