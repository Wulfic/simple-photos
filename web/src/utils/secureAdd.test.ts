import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { runSecureAddBatch, secureAddResultMessage } from "./secureAdd";

describe("runSecureAddBatch", () => {
  beforeEach(() => {
    // The batch logs a warning on every skip/failure (AGENTS.md: log every
    // error path). Silence it here but keep the spy so we can assert it fired.
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("secures every photo when all succeed", async () => {
    const { added, failed } = await runSecureAddBatch(
      ["a", "b", "c"],
      async (id) => `new-${id}`,
    );
    expect(added).toEqual(["a", "b", "c"]);
    expect(failed).toEqual([]);
    expect(console.warn).not.toHaveBeenCalled();
  });

  it("does NOT abort the batch when one photo throws (the #16 leak)", async () => {
    const attempted: string[] = [];
    const { added, failed } = await runSecureAddBatch(
      ["a", "b", "c", "d"],
      async (id) => {
        attempted.push(id);
        if (id === "b") throw new Error("network blip");
        return `new-${id}`;
      },
    );
    // Every id was attempted — the throw on "b" must not stop "c"/"d".
    expect(attempted).toEqual(["a", "b", "c", "d"]);
    expect(added).toEqual(["a", "c", "d"]);
    expect(failed).toEqual(["b"]);
    expect(console.warn).toHaveBeenCalledTimes(1);
  });

  it("treats a null clone id as a failure (photo not moved)", async () => {
    const { added, failed } = await runSecureAddBatch(
      ["a", "b", "c"],
      async (id) => (id === "b" ? null : `new-${id}`),
    );
    expect(added).toEqual(["a", "c"]);
    expect(failed).toEqual(["b"]);
    expect(console.warn).toHaveBeenCalledTimes(1);
  });

  it("reports all-failed without throwing", async () => {
    const { added, failed } = await runSecureAddBatch(
      ["a", "b"],
      async () => {
        throw new Error("server down");
      },
    );
    expect(added).toEqual([]);
    expect(failed).toEqual(["a", "b"]);
  });

  it("handles an empty selection", async () => {
    const { added, failed } = await runSecureAddBatch([], async () => "x");
    expect(added).toEqual([]);
    expect(failed).toEqual([]);
  });
});

describe("secureAddResultMessage", () => {
  it("reports success only when all photos moved", () => {
    expect(secureAddResultMessage({ added: 3, failed: 0 }, "Vault")).toEqual({
      success: "Added 3 photos to Vault",
    });
  });

  it("singularises a single added photo", () => {
    expect(secureAddResultMessage({ added: 1, failed: 0 }, "Vault")).toEqual({
      success: "Added 1 photo to Vault",
    });
  });

  it("reports both success and failure on a partial batch (#16)", () => {
    const msg = secureAddResultMessage({ added: 2, failed: 1 }, "Vault");
    expect(msg.success).toBe("Added 2 photos to Vault");
    expect(msg.error).toBe(
      "1 photo couldn't be secured and remain in your gallery",
    );
  });

  it("reports error only when nothing moved", () => {
    expect(secureAddResultMessage({ added: 0, failed: 2 }, "Vault")).toEqual({
      error: "2 photos couldn't be secured and remain in your gallery",
    });
  });

  it("returns an empty object when there is nothing to report", () => {
    expect(secureAddResultMessage({ added: 0, failed: 0 }, "Vault")).toEqual({});
  });
});
