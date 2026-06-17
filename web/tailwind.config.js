/** @type {import('tailwindcss').Config} */
import colors from "tailwindcss/colors";

export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // Single source of truth for the primary action color. Re-theme the
        // whole app by pointing this at a different palette (was a raw blue-600
        // sprinkled across ~84 call sites — now `accent-*` everywhere).
        accent: colors.indigo,
      },
      boxShadow: {
        // Refined, layered elevation (ambient + direct light) for the "subtle
        // depth" system. Default Tailwind shadows are a single flat drop; these
        // read as intentionally designed without looking heavy.
        xs: "0 1px 2px 0 rgb(0 0 0 / 0.05)",
        card: "0 1px 2px 0 rgb(0 0 0 / 0.04), 0 1px 3px 0 rgb(0 0 0 / 0.06)",
        "card-hover":
          "0 2px 4px -1px rgb(0 0 0 / 0.06), 0 8px 16px -4px rgb(0 0 0 / 0.10)",
        pop: "0 4px 12px -2px rgb(0 0 0 / 0.10), 0 16px 40px -8px rgb(0 0 0 / 0.20)",
      },
    },
  },
  plugins: [],
};
