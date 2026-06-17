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
        // Layered elevation (ambient + direct light) for the depth system.
        // Default Tailwind shadows are a single flat drop; these pair a soft
        // grounding shadow with a lit top edge so surfaces read as raised.
        // Kept neutral/black-based on purpose so the swappable `accent` color
        // never bleeds into the elevation system.
        xs: "0 1px 2px 0 rgb(0 0 0 / 0.05)",
        card: "0 1px 2px 0 rgb(0 0 0 / 0.05), 0 4px 12px -3px rgb(0 0 0 / 0.10)",
        "card-hover":
          "0 4px 8px -2px rgb(0 0 0 / 0.08), 0 14px 32px -8px rgb(0 0 0 / 0.18)",
        pop: "0 4px 12px -2px rgb(0 0 0 / 0.10), 0 16px 40px -8px rgb(0 0 0 / 0.20)",
        // Raised button: lit top edge (inset highlight) + two grounding drops.
        btn: "inset 0 1px 0 0 rgb(255 255 255 / 0.15), 0 1px 2px 0 rgb(0 0 0 / 0.18), 0 4px 10px -2px rgb(0 0 0 / 0.22)",
        "btn-hover":
          "inset 0 1px 0 0 rgb(255 255 255 / 0.22), 0 2px 4px 0 rgb(0 0 0 / 0.18), 0 8px 20px -4px rgb(0 0 0 / 0.30)",
        // Pressed: highlight gone, shadow pulls inward (recessed).
        "btn-inset": "inset 0 2px 4px 0 rgb(0 0 0 / 0.22)",
        // Carved-in form field.
        input: "inset 0 1px 2px 0 rgb(0 0 0 / 0.06)",
      },
    },
  },
  plugins: [],
};
