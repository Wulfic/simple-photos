// Real BroadcastChannels (Node ships them) rather than mocks: what's being
// tested is whether tabs actually hear each other and whether a tab can be
// fooled into hearing itself — a mock would just replay my own assumptions.
import { describe, it, expect, afterEach, vi } from "vitest";
import { peerSessionAlive, startKeySessionResponder, supportsKeySession } from "./keySession";

const teardowns: Array<() => void> = [];

/** Start a responder standing in for another tab (its own tabId). */
function tab(id: string, hasKey: () => boolean) {
  const stop = startKeySessionResponder(hasKey, { tabId: id });
  teardowns.push(stop);
  return stop;
}

afterEach(() => {
  while (teardowns.length) teardowns.pop()!();
  vi.restoreAllMocks();
});

describe("peerSessionAlive", () => {
  it("is true when another tab holds the key", async () => {
    tab("tab-1", () => true);
    await expect(peerSessionAlive({ tabId: "tab-2", timeoutMs: 200 })).resolves.toBe(true);
  });

  it("is false when no other tab is open", async () => {
    await expect(peerSessionAlive({ tabId: "tab-2", timeoutMs: 50 })).resolves.toBe(false);
  });

  it("is false when the other tab is open but has no key", async () => {
    // e.g. a tab sitting on the login screen: it can't vouch for a session
    // that hasn't been unlocked.
    tab("tab-1", () => false);
    await expect(peerSessionAlive({ tabId: "tab-2", timeoutMs: 50 })).resolves.toBe(false);
  });

  it("never lets a tab vouch for itself", async () => {
    // Both channels live in one tab and DO hear each other. Without the tabId
    // guard this tab would answer its own ping and "prove" a peer exists —
    // which is exactly the case (stale flag, empty keystore) where we must
    // fall back to the password instead.
    tab("tab-1", () => true);
    await expect(peerSessionAlive({ tabId: "tab-1", timeoutMs: 50 })).resolves.toBe(false);
  });

  it("is true when any one of several tabs holds the key", async () => {
    tab("tab-1", () => false);
    tab("tab-2", () => true);
    tab("tab-3", () => false);
    await expect(peerSessionAlive({ tabId: "tab-4", timeoutMs: 200 })).resolves.toBe(true);
  });

  it("stops vouching once a tab is torn down", async () => {
    const stop = tab("tab-1", () => true);
    stop();
    await expect(peerSessionAlive({ tabId: "tab-2", timeoutMs: 50 })).resolves.toBe(false);
  });

  it("re-reads hasKey on every ping rather than caching it", async () => {
    // A tab that logs out clears the key; it must stop vouching immediately.
    let unlocked = true;
    tab("tab-1", () => unlocked);
    await expect(peerSessionAlive({ tabId: "tab-2", timeoutMs: 200 })).resolves.toBe(true);

    unlocked = false;
    await expect(peerSessionAlive({ tabId: "tab-2", timeoutMs: 50 })).resolves.toBe(false);
  });
});

describe("failure handling", () => {
  it("resolves false and logs when the channel can't be opened", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const broken = () => {
      throw new Error("BroadcastChannel blocked");
    };

    await expect(peerSessionAlive({ tabId: "tab-2", factory: broken })).resolves.toBe(false);
    expect(logged).toHaveBeenCalled();
  });

  it("returns a safe no-op teardown when a responder can't open its channel", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const broken = () => {
      throw new Error("BroadcastChannel blocked");
    };

    const stop = startKeySessionResponder(() => true, { tabId: "tab-1", factory: broken });
    expect(() => stop()).not.toThrow();
  });

  it("reports support in this environment", () => {
    expect(supportsKeySession()).toBe(true);
  });
});
