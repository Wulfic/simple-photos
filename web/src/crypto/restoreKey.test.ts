// Real keystore + real key material (see crypto.test.ts for why): this is the
// decision every page load makes, and both wrong answers are bad — a password
// prompt with the key sitting in IndexedDB, or a key adopted when no session is
// running.
import "fake-indexeddb/auto";
import { describe, it, expect, beforeEach, vi } from "vitest";

class MemoryStorage {
  private store = new Map<string, string>();
  getItem(key: string) {
    return this.store.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.store.set(key, String(value));
  }
  removeItem(key: string) {
    this.store.delete(key);
  }
  clear() {
    this.store.clear();
  }
}

vi.stubGlobal("sessionStorage", new MemoryStorage());

/** A fresh tab: new module state, same origin-scoped storage. */
async function newTab() {
  vi.resetModules();
  const crypto = await import("./crypto");
  const { restoreKeyOnBoot } = await import("./restoreKey");
  return { crypto, restoreKeyOnBoot };
}

/** A window that has derived the key, i.e. an unlocked session. */
async function unlockedTab() {
  const tab = await newTab();
  await tab.crypto.deriveKey("correct horse battery staple", "tyler");
  return tab;
}

const peerIsAlive = async () => true;
const noPeers = async () => false;

beforeEach(() => {
  sessionStorage.clear();
});

describe("restoreKeyOnBoot", () => {
  it("loads the key directly on a reload, without bothering any peer", async () => {
    await unlockedTab();

    // The flag survives a reload, so this must not depend on another tab
    // existing — a lone tab has to survive F5.
    const reloaded = await newTab();
    const askedPeers = vi.fn(noPeers);

    await expect(
      reloaded.restoreKeyOnBoot({ isAuthenticated: true, peerAlive: askedPeers }),
    ).resolves.toBe("loaded");
    expect(askedPeers).not.toHaveBeenCalled();
    expect(reloaded.crypto.hasCryptoKey()).toBe(true);
  });

  it("adopts the key when a live tab vouches for the session", async () => {
    await unlockedTab();
    sessionStorage.clear(); // hand-opened tab: no flag came with it

    const second = await newTab();
    await expect(
      second.restoreKeyOnBoot({ isAuthenticated: true, peerAlive: peerIsAlive }),
    ).resolves.toBe("adopted");
    expect(second.crypto.hasCryptoKey()).toBe(true);
  });

  it("asks for the password when nobody vouches, key or no key", async () => {
    await unlockedTab();
    sessionStorage.clear();

    // The browser-restart case: the keystore still holds the key, but no tab is
    // alive to confirm the session. This is the assertion that keeps the key
    // session-scoped — if it ever flips to "adopted", the E2E key has quietly
    // become a permanent credential on the device.
    const second = await newTab();
    await expect(
      second.restoreKeyOnBoot({ isAuthenticated: true, peerAlive: noPeers }),
    ).resolves.toBe("absent");
    expect(second.crypto.hasCryptoKey()).toBe(false);
  });

  it("never runs the handshake for a logged-out visitor", async () => {
    await unlockedTab();
    sessionStorage.clear();

    // No token ⇒ nothing to adopt, and the login page shouldn't pay the
    // handshake timeout on every visit.
    const second = await newTab();
    const askedPeers = vi.fn(peerIsAlive);

    await expect(
      second.restoreKeyOnBoot({ isAuthenticated: false, peerAlive: askedPeers }),
    ).resolves.toBe("absent");
    expect(askedPeers).not.toHaveBeenCalled();
    expect(second.crypto.hasCryptoKey()).toBe(false);
  });

  it("asks for the password when a peer vouches but the keystore is empty", async () => {
    const first = await unlockedTab();
    first.crypto.clearKey(); // peer signed out between the ping and the read
    await new Promise((resolve) => setTimeout(resolve, 10)); // clearKey wipes IDB async
    sessionStorage.clear();

    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const second = await newTab();

    await expect(
      second.restoreKeyOnBoot({ isAuthenticated: true, peerAlive: peerIsAlive }),
    ).resolves.toBe("absent");
    expect(logged).toHaveBeenCalled();
    logged.mockRestore();
  });

  it("asks for the password rather than throwing when the handshake blows up", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const second = await newTab();

    await expect(
      second.restoreKeyOnBoot({
        isAuthenticated: true,
        peerAlive: async () => {
          throw new Error("channel exploded");
        },
      }),
    ).resolves.toBe("absent");
    expect(logged).toHaveBeenCalled();
    logged.mockRestore();
  });
});
