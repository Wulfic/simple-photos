import { describe, it, expect, vi } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { modalEscapeHandler } from "./Modal";

function key(k: string) {
  return { key: k, stopPropagation: vi.fn() };
}

describe("modalEscapeHandler", () => {
  it("closes on Escape and consumes the key, so the page beneath does not act on it", () => {
    // F4: without stopPropagation the viewer's window-level Escape also fired
    // and left the viewer while closing the dialog.
    const onClose = vi.fn();
    const e = key("Escape");
    modalEscapeHandler(onClose)(e);
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(e.stopPropagation).toHaveBeenCalledTimes(1);
  });

  it("leaves every other key alone — neither closes nor swallows it", () => {
    const onClose = vi.fn();
    for (const k of ["Enter", "ArrowLeft", "ArrowRight", " ", "Tab"]) {
      const e = key(k);
      modalEscapeHandler(onClose)(e);
      expect(e.stopPropagation).not.toHaveBeenCalled();
    }
    expect(onClose).not.toHaveBeenCalled();
  });

  it("is the handler Modal actually registers", () => {
    const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "Modal.tsx"), "utf8");
    expect(src).toContain("export function Modal(");
    expect(src).toContain("const onKey = modalEscapeHandler(onClose);");
    expect(src).toContain('document.addEventListener("keydown", onKey);');
  });
});
