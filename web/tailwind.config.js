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
        // Violet reads warmer/friendlier than indigo in light mode while still
        // pairing with the cool-neutral surface ramp below.
        accent: colors.violet,

        // ── Semantic surface / text / border tokens ───────────────────────────
        // Backed by CSS custom properties (see index.css @layer base) that swap
        // between the light and dark ramps on `.dark`. Using these means a call
        // site writes `bg-surface text-fg` ONCE instead of
        // `bg-white text-gray-900 dark:bg-gray-800 dark:text-gray-100` — light
        // and dark are now tuned from one place. The `<alpha-value>` shim keeps
        // opacity utilities (`bg-surface/80`, `border-edge/50`) working.
        canvas: "rgb(var(--canvas) / <alpha-value>)",
        surface: {
          DEFAULT: "rgb(var(--surface) / <alpha-value>)",
          raised: "rgb(var(--surface-raised) / <alpha-value>)",
          sunken: "rgb(var(--surface-sunken) / <alpha-value>)",
        },
        edge: {
          DEFAULT: "rgb(var(--edge) / <alpha-value>)",
          strong: "rgb(var(--edge-strong) / <alpha-value>)",
        },
        fg: {
          DEFAULT: "rgb(var(--fg) / <alpha-value>)",
          muted: "rgb(var(--fg-muted) / <alpha-value>)",
          subtle: "rgb(var(--fg-subtle) / <alpha-value>)",
        },
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
