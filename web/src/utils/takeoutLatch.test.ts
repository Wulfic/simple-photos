import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import {
  clearMaterializedAt,
  getMaterializedAt,
  setMaterializedAt,
  shouldReconstruct,
} from "./takeoutLatch";

// jsdom isn't configured for this suite, so stand up the minimal localStorage
// the latch uses. Keeps the test about the latch's rules, not the environment.
function installStorage(): void {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
  });
}

describe("takeoutLatch", () => {
  beforeEach(installStorage);
  afterEach(() => vi.unstubAllGlobals());

  it("reports -1 until a pass has settled", () => {
    expect(getMaterializedAt("alice")).toBe(-1);
  });

  it("round-trips the mirror size a pass settled at", () => {
    setMaterializedAt("alice", 5200);
    expect(getMaterializedAt("alice")).toBe(5200);
  });

  it("runs reconstruction when it has never settled", () => {
    expect(shouldReconstruct("alice", 5200)).toBe(true);
  });

  it("skips reconstruction once settled at the current mirror size", () => {
    // The steady state this exists for: revisiting the page must not re-run the
    // whole pass. As a ref, this reset on every mount.
    setMaterializedAt("alice", 5200);
    expect(shouldReconstruct("alice", 5200)).toBe(false);
  });

  it("re-opens the moment the mirror grows", () => {
    // Self-healing: new photos synced in may belong to albums we skipped, so a
    // latch that could never re-open would mean permanently incomplete albums.
    setMaterializedAt("alice", 5200);
    expect(shouldReconstruct("alice", 5201)).toBe(true);
  });

  it("re-opens when the mirror shrinks too", () => {
    setMaterializedAt("alice", 5200);
    expect(shouldReconstruct("alice", 4000)).toBe(true);
  });

  it("never runs against an empty mirror", () => {
    // Nothing to match against — a pass would only report everything unmatched.
    expect(shouldReconstruct("alice", 0)).toBe(false);
  });

  it("keeps each user's latch to themselves", () => {
    // A shared browser must never let one account's latch suppress another's
    // reconstruction.
    setMaterializedAt("alice", 5200);
    expect(shouldReconstruct("bob", 5200)).toBe(true);
    expect(getMaterializedAt("bob")).toBe(-1);
  });

  it("forgets a cleared latch", () => {
    setMaterializedAt("alice", 5200);
    clearMaterializedAt("alice");
    expect(shouldReconstruct("alice", 5200)).toBe(true);
  });

  it("treats a corrupted value as never settled", () => {
    localStorage.setItem("sp:takeout-materialized:alice", "not-a-number");
    expect(getMaterializedAt("alice")).toBe(-1);
    expect(shouldReconstruct("alice", 5200)).toBe(true);
  });

  it("survives storage being unavailable", () => {
    // Private mode / quota. Reconstruction is idempotent, so the fallback is
    // simply to run it — never to throw out of a render.
    vi.stubGlobal("localStorage", {
      getItem: () => { throw new Error("denied"); },
      setItem: () => { throw new Error("denied"); },
      removeItem: () => { throw new Error("denied"); },
    });
    expect(() => setMaterializedAt("alice", 10)).not.toThrow();
    expect(getMaterializedAt("alice")).toBe(-1);
    expect(shouldReconstruct("alice", 10)).toBe(true);
  });
});
