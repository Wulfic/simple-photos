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
  toggle: () =>
    set((s) => {
      const next = s.theme === "light" ? "dark" : "light";
      localStorage.setItem("theme", next);
      return { theme: next };
    }),
  setTheme: (t: Theme) => {
    localStorage.setItem("theme", t);
    set({ theme: t });
  },
}));
