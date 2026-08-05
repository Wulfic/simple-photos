// Real IndexedDB (in-memory) + real WebCrypto: this covers the key handoff a
// second window depends on (#21), so the CryptoKey has to make an actual
// round-trip through the keystore. A mocked keystore would prove nothing about
// whether the adopted key can still decrypt.
import "fake-indexeddb/auto";
import { describe, it, expect, beforeEach, vi } from "vitest";

/** Minimal sessionStorage — Node has no DOM storage. */
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

/**
 * A fresh import = a fresh tab: new module-level key cache, same origin-scoped
 * IndexedDB and sessionStorage underneath — exactly the split a second window
 * sees.
 */
async function newTab() {
  vi.resetModules();
  return await import("./crypto");
}

/** Open a tab that has derived the key, i.e. an unlocked window. */
async function unlockedTab() {
  const tab = await newTab();
  await tab.deriveKey("correct horse battery staple", "tyler");
  return tab;
}

beforeEach(() => {
  sessionStorage.clear();
});

describe("adoptKeyFromKeystore", () => {
  it("adopts the key an unlocked window left in the keystore", async () => {
    await unlockedTab();

    // Hand-opened tab: IndexedDB still holds the key, but no flag came with it.
    sessionStorage.clear();
    const second = await newTab();
    expect(second.hasCryptoKey()).toBe(false);
    expect(await second.loadKeyFromSession()).toBe(false);

    expect(await second.adoptKeyFromKeystore()).toBe(true);
    expect(second.hasCryptoKey()).toBe(true);
  });

  it("adopts a key that actually decrypts the first window's data", async () => {
    const first = await unlockedTab();
    const secret = new TextEncoder().encode("a photo");
    const ciphertext = await first.encrypt(secret);

    sessionStorage.clear();
    const second = await newTab();
    await second.adoptKeyFromKeystore();

    expect(new Uint8Array(await second.decrypt(ciphertext))).toEqual(secret);
  });

  it("restores the session flag so the rest of the app behaves normally", async () => {
    await unlockedTab();
    sessionStorage.clear();

    const second = await newTab();
    await second.adoptKeyFromKeystore();

    // hasCryptoKey() is read synchronously by the page gates; the adopting tab
    // must look identical to the tab it adopted from.
    expect(sessionStorage.getItem("sp_key")).toBe("present");
  });

  it("fails when the keystore is empty rather than pretending to have a key", async () => {
    const first = await unlockedTab();
    first.clearKey(); // e.g. the peer signed out between the ping and the read
    // clearKey wipes IndexedDB fire-and-forget; let it land.
    await new Promise((resolve) => setTimeout(resolve, 10));

    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const second = await newTab();

    expect(await second.adoptKeyFromKeystore()).toBe(false);
    expect(second.hasCryptoKey()).toBe(false);
    expect(logged).toHaveBeenCalled();
    logged.mockRestore();
  });

  it("keeps the key it already has", async () => {
    const tab = await unlockedTab();
    expect(await tab.adoptKeyFromKeystore()).toBe(true);
    expect(tab.hasCryptoKey()).toBe(true);
  });
});

describe("loadKeyFromSession", () => {
  it("reloads the key after a refresh, which keeps the flag", async () => {
    await unlockedTab();

    // A refresh keeps sessionStorage — no peer handshake needed.
    const refreshed = await newTab();
    expect(await refreshed.loadKeyFromSession()).toBe(true);
  });

  it("refuses to load without the flag, even though the keystore has the key", async () => {
    await unlockedTab();
    sessionStorage.clear();

    // This is the session scoping that adoptKeyFromKeystore deliberately
    // bypasses only on a live peer's say-so: after a browser restart nobody
    // vouches, so the password is required even though the key is right there.
    const second = await newTab();
    expect(await second.loadKeyFromSession()).toBe(false);
    expect(second.hasCryptoKey()).toBe(false);
  });
});
