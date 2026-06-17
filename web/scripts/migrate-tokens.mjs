// One-shot codemod: migrate unambiguous paired Tailwind color classes to the
// new semantic tokens (bg-canvas/surface/fg/edge). Only PAIRED forms (a light
// class immediately followed by its dark: partner) are replaced, because those
// collapse to a single token deterministically. Bare single-mode classes are
// left untouched for manual review. Run from web/: `node scripts/migrate-tokens.mjs`
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

// Ordered longest/most-specific first. Each entry is a literal substring swap.
const REPLACEMENTS = [
  // ── Page / surface backgrounds ──────────────────────────────────────────
  ["bg-gray-100 dark:bg-gray-900", "bg-canvas"],
  ["bg-gray-50 dark:bg-gray-900", "bg-canvas"],
  ["bg-white dark:bg-gray-800", "bg-surface"],
  ["bg-gray-50 dark:bg-gray-800", "bg-surface"],
  ["bg-gray-50 dark:bg-gray-700", "bg-surface-raised"],
  ["bg-gray-100 dark:bg-gray-700", "bg-surface-raised"],
  // ── Text ─────────────────────────────────────────────────────────────────
  ["text-gray-900 dark:text-white", "text-fg"],
  ["text-gray-900 dark:text-gray-100", "text-fg"],
  ["text-gray-900 dark:text-gray-50", "text-fg"],
  ["text-gray-800 dark:text-gray-100", "text-fg"],
  ["text-gray-800 dark:text-gray-200", "text-fg"],
  ["text-gray-700 dark:text-gray-300", "text-fg-muted"],
  ["text-gray-700 dark:text-gray-200", "text-fg-muted"],
  ["text-gray-700 dark:text-gray-400", "text-fg-muted"],
  ["text-gray-700 dark:text-gray-500", "text-fg-muted"],
  ["text-gray-600 dark:text-gray-400", "text-fg-muted"],
  ["text-gray-600 dark:text-gray-300", "text-fg-muted"],
  ["text-gray-600 dark:text-gray-500", "text-fg-muted"],
  ["text-gray-500 dark:text-gray-400", "text-fg-subtle"],
  ["text-gray-500 dark:text-gray-500", "text-fg-subtle"],
  ["text-gray-400 dark:text-gray-500", "text-fg-subtle"],
  ["text-gray-400 dark:text-gray-400", "text-fg-subtle"],
  ["text-gray-400 dark:text-gray-600", "text-fg-subtle"],
  // ── Borders / dividers ─────────────────────────────────────────────────
  ["border-gray-200 dark:border-gray-700", "border-edge"],
  ["border-gray-200 dark:border-white/10", "border-edge"],
  ["border-gray-100 dark:border-gray-700", "border-edge"],
  ["border-gray-100 dark:border-gray-800", "border-edge"],
  ["border-gray-300 dark:border-gray-600", "border-edge-strong"],
  ["border-gray-300 dark:border-gray-700", "border-edge-strong"],
  ["border-gray-300 dark:border-gray-500", "border-edge-strong"],
  ["border-gray-400 dark:border-gray-500", "border-edge-strong"],
  ["border-gray-200 dark:border-gray-600", "border-edge"],
  ["divide-gray-200 dark:divide-gray-700", "divide-edge"],
  ["divide-gray-100 dark:divide-gray-700", "divide-edge"],
  ["bg-gray-200 dark:bg-gray-700", "bg-edge"],
  ["bg-gray-200 dark:bg-gray-600", "bg-edge-strong"],
  ["bg-gray-100 dark:bg-gray-800", "bg-surface-sunken dark:bg-surface"],
  // Exact known badge string (bg+text interleaved) used by diagnostics/SSL tags.
  ["bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300", "bg-edge text-fg-muted"],
  // ── Hover backgrounds (keep dark translucent overlays) ──────────────────
  ["hover:bg-gray-100 dark:hover:bg-gray-700", "hover:bg-surface-sunken dark:hover:bg-white/10"],
  ["hover:bg-gray-50 dark:hover:bg-gray-700", "hover:bg-surface-sunken dark:hover:bg-white/10"],
  ["hover:bg-gray-50 dark:hover:bg-gray-800/50", "hover:bg-surface-sunken dark:hover:bg-white/5"],
  ["hover:bg-gray-100 dark:hover:bg-white/20", "hover:bg-surface-sunken dark:hover:bg-white/20"],
  ["hover:bg-gray-200 dark:hover:bg-gray-600", "hover:bg-edge dark:hover:bg-white/15"],
  ["hover:bg-gray-200 dark:hover:bg-gray-700", "hover:bg-edge dark:hover:bg-white/10"],
  ["hover:bg-gray-300 dark:hover:bg-gray-600", "hover:bg-edge-strong"],
  // ── Hover text pairs ────────────────────────────────────────────────────
  ["hover:text-gray-700 dark:hover:text-gray-300", "hover:text-fg"],
  ["hover:text-gray-700 dark:hover:text-gray-200", "hover:text-fg"],
  ["hover:text-gray-900 dark:hover:text-white", "hover:text-fg"],
  ["hover:text-gray-900 dark:hover:text-gray-100", "hover:text-fg"],
  ["hover:text-gray-600 dark:hover:text-gray-300", "hover:text-fg"],
  ["hover:text-gray-600 dark:hover:text-gray-200", "hover:text-fg"],
  ["hover:text-gray-800 dark:hover:text-gray-200", "hover:text-fg"],
  // ── Hover borders ────────────────────────────────────────────────────────
  ["hover:border-gray-300 dark:hover:border-gray-600", "hover:border-edge-strong"],
  ["hover:border-gray-300 dark:hover:border-gray-500", "hover:border-edge-strong"],
  ["hover:border-gray-400 dark:hover:border-gray-500", "hover:border-edge-strong"],
  // ── Focus ring offsets (the page color behind the ring) ─────────────────
  ["focus-visible:ring-offset-gray-50 dark:focus-visible:ring-offset-gray-900", "focus-visible:ring-offset-canvas"],
  ["focus-visible:ring-offset-white dark:focus-visible:ring-offset-gray-900", "focus-visible:ring-offset-canvas"],
  ["ring-offset-gray-50 dark:ring-offset-gray-900", "ring-offset-canvas"],
  ["ring-offset-white dark:ring-offset-gray-900", "ring-offset-canvas"],
];

const exts = new Set([".tsx", ".ts"]);
function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const s = statSync(p);
    if (s.isDirectory()) walk(p, out);
    else if (exts.has(p.slice(p.lastIndexOf(".")))) out.push(p);
  }
  return out;
}

const root = join(process.cwd(), "src");
let filesChanged = 0;
let totalSwaps = 0;
const perPattern = new Map();

for (const file of walk(root)) {
  let txt = readFileSync(file, "utf8");
  const before = txt;
  for (const [from, to] of REPLACEMENTS) {
    if (txt.includes(from)) {
      const count = txt.split(from).length - 1;
      txt = txt.split(from).join(to);
      totalSwaps += count;
      perPattern.set(from, (perPattern.get(from) || 0) + count);
    }
  }
  if (txt !== before) {
    writeFileSync(file, txt, "utf8");
    filesChanged++;
  }
}

console.log(`Files changed: ${filesChanged}`);
console.log(`Total swaps:   ${totalSwaps}`);
for (const [pat, n] of [...perPattern.entries()].sort((a, b) => b[1] - a[1])) {
  console.log(`  ${String(n).padStart(4)}  ${pat}`);
}
