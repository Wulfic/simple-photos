import { describe, it, expect, vi, afterEach } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  planSecureRemoval,
  executeSecureRemoval,
  secureRemovalOutcome,
  readSecureFlash,
  type RemovableSecureItem,
} from "./secureRemoval";

const PRIVATE = { id: "g1", name: "Private" };
const WORK = { id: "g2", name: "Work" };

function item(
  id: string,
  over: Partial<RemovableSecureItem> = {},
): RemovableSecureItem {
  return {
    id,
    blob_id: `blob-${id}`,
    gallery_id: "g1",
    burst_id: null,
    galleries: [PRIVATE],
    ...over,
  };
}

// One single, one 3-frame burst, all in "Private" only.
const pool: RemovableSecureItem[] = [
  item("a"),
  item("b1", { burst_id: "burst" }),
  item("b2", { burst_id: "burst" }),
  item("b3", { burst_id: "burst" }),
];

afterEach(() => {
  vi.restoreAllMocks();
});

describe("planSecureRemoval", () => {
  it("returns null for nothing tapped — no prompt for an empty batch", () => {
    expect(planSecureRemoval(pool, [], { id: "g1" })).toBeNull();
  });

  it("expands a burst from a NON-cover frame to the whole stack (F4)", () => {
    // The viewer's burst strip can land on any frame; the removal must still
    // take the stack, or the other frames are stranded (S2) and the prompt
    // counts one photo while the grid tile stood for three.
    const plan = planSecureRemoval(pool, [pool[3]], { id: "g1", name: "Private" })!;
    expect(plan.targets.map((t) => t.item.id).sort()).toEqual(["b1", "b2", "b3"]);
    expect(plan.verdict.title).toContain("3 photos");
  });

  it("routes a real album's items to that album", () => {
    const plan = planSecureRemoval(pool, [pool[0]], PRIVATE)!;
    expect(plan.targets).toEqual([{ item: pool[0], owningGalleryId: "g1" }]);
    expect(plan.otherAlbums).toEqual([0]);
    expect(plan.verdict.kind).toBe("confirm");
  });

  it("routes a smart view's items to their OWNING album, not the synthetic id", () => {
    const inWork = item("w", { gallery_id: "g2", gallery_name: "Work", galleries: [WORK] });
    const plan = planSecureRemoval([inWork], [inWork], { id: "secure-smart-videos", name: "Videos" })!;
    expect(plan.targets[0].owningGalleryId).toBe("g2");
    // Named after the owning album — "Videos" would be the wrong thing to name.
    expect(plan.verdict.title).toContain("Work");
    expect(plan.verdict.title).not.toContain("Videos");
  });

  it("names the album from the item's own memberships when the caller has no name", () => {
    // The viewer only knows the album id; both feeds publish `galleries`.
    const plan = planSecureRemoval(pool, [pool[0]], { id: "g1" })!;
    expect(plan.verdict.title).toContain("Private");
  });

  it("names no album rather than the wrong one for a smart-view batch across albums", () => {
    const p = item("p", { gallery_name: "Private" });
    const w = item("w", { gallery_id: "g2", gallery_name: "Work", galleries: [WORK] });
    const plan = planSecureRemoval([p, w], [p, w], { id: "secure-smart-all" })!;
    expect(plan.verdict.title).toContain("this secure album");
  });

  it("counts other albums, so a photo held elsewhere is said to STAY secured", () => {
    const shared = item("s", { galleries: [PRIVATE, WORK] });
    const plan = planSecureRemoval([shared], [shared], PRIVATE)!;
    expect(plan.otherAlbums).toEqual([1]);
    expect(plan.verdict.body).toMatch(/stay secured/);
  });

  it("refuses (blocked) when membership is unknown — never guesses zero", () => {
    const unknown = item("u", { galleries: [] });
    expect(planSecureRemoval([unknown], [unknown], PRIVATE)!.verdict.kind).toBe("blocked");
  });

  it("refuses when a smart-view item has no owning album id", () => {
    const orphan = item("o", { gallery_id: null });
    const plan = planSecureRemoval([orphan], [orphan], { id: "secure-smart-all" })!;
    expect(plan.targets[0].owningGalleryId).toBeUndefined();
    expect(plan.verdict.kind).toBe("blocked");
  });
});

describe("executeSecureRemoval", () => {
  function deps() {
    return {
      removeItem: vi.fn(async (_g: string, _i: string) => undefined),
      dropCacheRow: vi.fn(async (_b: string) => undefined),
    };
  }

  it("removes exactly the planned (expanded) set from the owning album", async () => {
    const plan = planSecureRemoval(pool, [pool[1]], PRIVATE)!;
    const d = deps();
    const res = await executeSecureRemoval(plan, d);
    expect(d.removeItem.mock.calls.map((c) => c.join("/")).sort()).toEqual([
      "g1/b1",
      "g1/b2",
      "g1/b3",
    ]);
    expect(d.dropCacheRow.mock.calls.map((c) => c[0]).sort()).toEqual([
      "blob-b1",
      "blob-b2",
      "blob-b3",
    ]);
    expect(res).toEqual({ removedCounts: [0, 0, 0], failed: 0 });
  });

  it("isolates a failing item — the rest are still removed", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const plan = planSecureRemoval(pool, [pool[1]], PRIVATE)!;
    const d = deps();
    d.removeItem.mockImplementation(async (_g, i) => {
      if (i === "b2") throw new Error("boom");
      return undefined;
    });
    const res = await executeSecureRemoval(plan, d);
    expect(res).toEqual({ removedCounts: [0, 0], failed: 1 });
    // The failed item's cache row stays: it is still in the album.
    expect(d.dropCacheRow.mock.calls.map((c) => c[0])).not.toContain("blob-b2");
    expect(console.error).toHaveBeenCalled();
  });

  it("does not count a cache-drop failure as a failed removal", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const plan = planSecureRemoval(pool, [pool[0]], PRIVATE)!;
    const d = deps();
    d.dropCacheRow.mockRejectedValue(new Error("idb closed"));
    const res = await executeSecureRemoval(plan, d);
    expect(res).toEqual({ removedCounts: [0], failed: 0 });
    expect(console.warn).toHaveBeenCalled();
  });

  it("never calls the server for a target the plan could not route", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const orphan = item("o", { gallery_id: null });
    const plan = planSecureRemoval([orphan], [orphan], { id: "secure-smart-all" })!;
    const d = deps();
    const res = await executeSecureRemoval(plan, d);
    expect(d.removeItem).not.toHaveBeenCalled();
    expect(res).toEqual({ removedCounts: [], failed: 1 });
  });
});

describe("secureRemovalOutcome", () => {
  it("reports from the plan's counts", () => {
    expect(secureRemovalOutcome({ removedCounts: [0], failed: 0 })).toEqual({
      success: "Photo returned to your gallery.",
      error: "",
    });
    expect(secureRemovalOutcome({ removedCounts: [1], failed: 0 }).success).toMatch(
      /stays secured/,
    );
  });

  it("claims no success when nothing was removed, and counts failures", () => {
    expect(secureRemovalOutcome({ removedCounts: [], failed: 2 })).toEqual({
      success: "",
      error: "2 items couldn't be removed.",
    });
    expect(secureRemovalOutcome({ removedCounts: [0], failed: 1 }).error).toBe(
      "1 item couldn't be removed.",
    );
  });
});

describe("readSecureFlash", () => {
  it("reads the flash the viewer sends", () => {
    expect(readSecureFlash({ secureFlash: { success: "ok", error: "" } })).toEqual({
      success: "ok",
      error: "",
    });
  });

  it("reads nothing from absent, empty or malformed route state", () => {
    expect(readSecureFlash(null)).toBeNull();
    expect(readSecureFlash(undefined)).toBeNull();
    expect(readSecureFlash({})).toBeNull();
    expect(readSecureFlash({ photoIds: ["a"] })).toBeNull();
    expect(readSecureFlash({ secureFlash: "ok" })).toBeNull();
    expect(readSecureFlash({ secureFlash: { success: 1, error: "" } })).toBeNull();
    // Nothing to say is not a flash — no empty toast.
    expect(readSecureFlash({ secureFlash: { success: "", error: "" } })).toBeNull();
  });
});

/**
 * Wiring guards for the viewer's removal (F4). No jsdom in this repo, so — as in
 * `albumRemoval.test.ts` — the call sites are read from source. A pipeline with
 * no viewer call site is exactly how Z1 shipped half-wired.
 */
describe("the secure viewer offers removal (F4)", () => {
  const SRC = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const viewer = readFileSync(join(SRC, "pages", "Viewer.tsx"), "utf8");
  const topBar = readFileSync(join(SRC, "components", "viewer", "ViewerTopBar.tsx"), "utf8");
  const grid = readFileSync(join(SRC, "pages", "SecureGallery.tsx"), "utf8");

  it("reads non-empty source files", () => {
    expect(viewer).toContain("export default function Viewer");
    expect(topBar).toContain("export default function ViewerTopBar");
    expect(grid).toContain("export default function SecureGallery");
  });

  it("the top bar has the menu item, gated only on its handler", () => {
    expect(topBar).toContain("Remove from secure album");
    expect(topBar).toContain("{onRemoveFromSecureAlbum && (");
  });

  it("the viewer plans, executes and reports through the shared pipeline", () => {
    expect(viewer).toContain("planSecureRemoval(");
    expect(viewer).toContain("executeSecureRemoval(");
    expect(viewer).toContain("secureFlash: secureRemovalOutcome(");
    expect(viewer).toContain("onRemoveFromSecureAlbum={canRemoveFromSecureAlbum ? requestSecureRemoval");
  });

  it("asks with the three-way dialog, not window.confirm", () => {
    expect(viewer).toContain("pendingSecureRemoval.verdict.kind === \"confirm\"");
    expect(viewer).toContain("pendingSecureRemoval.verdict.kind === \"blocked\"");
    expect(viewer).not.toMatch(/(?<![.\w])confirm\(["'`]/);
  });

  it("the keyboard cannot move the photo under an open removal dialog", () => {
    const start = viewer.indexOf("function handleKey(e: KeyboardEvent)");
    const handler = viewer.slice(start, viewer.indexOf("window.addEventListener(\"keydown\"", start));
    expect(start).toBeGreaterThan(0);
    // The guard must come before the arrow keys are read.
    const guard = handler.indexOf("if (pendingSecureRemoval) return;");
    expect(guard).toBeGreaterThan(0);
    expect(guard).toBeLessThan(handler.indexOf("ArrowLeft"));
  });

  it("a swipe on the removal dialog cannot reach the viewer's swipe handlers", () => {
    // Modal renders inside the viewer root, whose onTouch* handlers navigate.
    const start = viewer.indexOf("{pendingSecureRemoval && (");
    const wrapper = viewer.slice(start, viewer.indexOf("<ConfirmDialog", start));
    expect(start).toBeGreaterThan(0);
    for (const ev of ["onTouchStart", "onTouchMove", "onTouchEnd"]) {
      expect(wrapper).toContain(`${ev}={stopTouch}`);
    }
    expect(viewer).toContain("e.stopPropagation()");
  });

  it("the grid shows the toast the viewer sends back", () => {
    expect(grid).toContain("readSecureFlash(location.state)");
  });
});
