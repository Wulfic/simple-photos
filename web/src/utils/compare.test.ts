import { describe, it, expect } from "vitest";
import { canCompare, compareTargets, COMPARE_COUNT } from "./compare";

describe("compare selection helpers", () => {
  it("only allows compare with exactly two items", () => {
    expect(canCompare(0)).toBe(false);
    expect(canCompare(1)).toBe(false);
    expect(canCompare(COMPARE_COUNT)).toBe(true);
    expect(canCompare(3)).toBe(false);
    expect(canCompare(10)).toBe(false);
  });

  it("resolves an ordered pair from a two-item selection", () => {
    expect(compareTargets(["a", "b"])).toEqual(["a", "b"]);
    // Set preserves insertion order.
    expect(compareTargets(new Set(["x", "y"]))).toEqual(["x", "y"]);
  });

  it("returns null when the selection isn't exactly two", () => {
    expect(compareTargets([])).toBeNull();
    expect(compareTargets(["only"])).toBeNull();
    expect(compareTargets(["a", "b", "c"])).toBeNull();
    expect(compareTargets(new Set(["a", "b", "c"]))).toBeNull();
  });
});
