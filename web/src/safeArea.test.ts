/**
 * Safe-area inset guards (#50).
 *
 * There is no unit test for "the controls clear the navigation bar" — that is
 * CSS, this repo has no jsdom, and `env(safe-area-inset-*)` is not resolvable
 * outside a real device anyway. So these tests pin the three things that would
 * silently turn the whole fix into a no-op, each of which IS checkable here:
 *
 *  1. `viewport-fit=cover` is present. Without it `viewport-fit` defaults to
 *     `auto`, the viewport is constrained to the safe area, and every `env()`
 *     resolves to 0 — the padding computes to exactly the value it replaced
 *     and nothing moves. The fix would look shipped and do nothing.
 *  2. Every `safe-*` class used in the tree is actually defined. These are
 *     plain CSS classes, so a typo is not a type error, not a build error, and
 *     not a runtime error — it is a class that quietly matches nothing.
 *  3. Every `env()` carries a fallback. A browser that does not know the
 *     keyword invalidates the whole declaration, dropping the padding
 *     entirely — worse than the overlap it was added to fix.
 *
 * Read via `node:fs`, NOT `import.meta.glob(..., "?raw")`: Vite's CSS plugin
 * takes precedence over `?raw` for `.css`, so the glob returns an EMPTY STRING
 * and every assertion below passes vacuously. That failure mode is the exact
 * one this file exists to catch, so it is worth the `@types/node` devDependency.
 */
import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname, resolve, relative } from "node:path";
import { fileURLToPath } from "node:url";

const SRC = dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = resolve(SRC, "..");

const indexHtml = readFileSync(join(WEB_ROOT, "index.html"), "utf8");
const indexCss = readFileSync(join(SRC, "index.css"), "utf8");

const stripCssComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");

/** The contents of every `className="…"` / `className={`…`}` in a source file. */
function classNames(source: string): string[] {
  return [...source.matchAll(/className=(?:"([^"]*)"|\{`([^`]*)`\})/g)].map(
    (m) => m[1] ?? m[2],
  );
}

/** Every source file under src/ that could carry a className. */
function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) return sourceFiles(full);
    return /\.tsx?$/.test(entry) ? [full] : [];
  });
}

describe("safe-area insets (#50)", () => {
  it("index.html opts into edge-to-edge with viewport-fit=cover", () => {
    const meta = indexHtml.match(/<meta\s+name="viewport"\s+content="([^"]+)"/);
    expect(meta, "no viewport meta tag at all").not.toBeNull();
    expect(meta![1]).toContain("viewport-fit=cover");
  });

  it("defines every safe-* class the tree references", () => {
    const defined = new Set(
      [...indexCss.matchAll(/^\s*\.(safe-[A-Za-z0-9_-]+)/gm)].map((m) => m[1]),
    );
    expect(defined.size, "no safe-* utilities defined in index.css").toBeGreaterThan(0);

    const used = new Map<string, string[]>();
    for (const file of sourceFiles(SRC)) {
      if (file.endsWith("safeArea.test.ts")) continue;
      for (const cls of classNames(readFileSync(file, "utf8"))) {
        // The `sm:`/`md:` prefix is deliberately not captured — Tailwind
        // generates those variants from the base utility, so `sm:safe-bottom-6`
        // needs `.safe-bottom-6` to exist, not `.sm\:safe-bottom-6`.
        for (const m of cls.matchAll(/(?:^|[\s:])(safe-[A-Za-z0-9_-]+)/g)) {
          const at = relative(WEB_ROOT, file);
          used.set(m[1], [...(used.get(m[1]) ?? []), at]);
        }
      }
    }
    expect(used.size, "no safe-* classes applied anywhere").toBeGreaterThan(0);

    const undefinedUses = [...used.entries()]
      .filter(([cls]) => !defined.has(cls))
      .map(([cls, files]) => `${cls} (used in ${[...new Set(files)].join(", ")})`);
    expect(undefinedUses, "safe-* classes that match no CSS rule").toEqual([]);
  });

  it("gives every env() a fallback so the calc() cannot be invalidated", () => {
    // Comments are stripped first — the prose above these rules talks ABOUT
    // env(), and scanning it finds argument-less matches that are not code.
    const envs = [...stripCssComments(indexCss).matchAll(/env\(([^)]*)\)/g)].map(
      (m) => m[1],
    );
    expect(envs.length, "no env() usages found").toBeGreaterThan(0);
    const withoutFallback = envs.filter((args) => !args.includes(","));
    expect(withoutFallback, "env() without a fallback value").toEqual([]);
  });

  it("keeps the reported surface — the video control bar — inset at the bottom", () => {
    // Only the class lists are inspected, not the whole file: the comment
    // explaining the change necessarily names the class it replaced.
    const classes = classNames(
      readFileSync(join(SRC, "components", "viewer", "VideoControls.tsx"), "utf8"),
    );
    expect(classes.some((c) => c.includes("safe-pb-3"))).toBe(true);
    // The bare Tailwind class is what #50 reported as broken. If it survived
    // alongside the fix, cascade order — not intent — would decide the padding.
    const stillBare = classes.filter((c) => /(?:^|\s)pb-3(?:\s|$)/.test(c));
    expect(stillBare, "bare pb-3 left on a control surface").toEqual([]);
  });
});
