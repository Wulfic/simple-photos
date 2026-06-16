/**
 * Client-side cryptography — AES-256-GCM encrypt/decrypt, Argon2id key
 * derivation, and SHA-256 hashing.
 *
 * Wire format (shared with server `crypto.rs`):
 *   `[12-byte nonce][AES-GCM ciphertext + 16-byte auth tag]`
 *
 * Key derivation: Argon2id(password, salt=username, memory=64 MiB, time=3,
 * parallelism=4) → 32-byte AES key. The derived key is held in-memory
 * (sessionStorage flag tracks its existence across refreshes).
 */
import { argon2id } from "hash-wasm";
import { sha256 } from "@noble/hashes/sha2.js";
import { gcm } from "@noble/ciphers/aes.js";

const ARGON2_MEMORY = 65536;
const ARGON2_TIME = 3;
const ARGON2_PARALLELISM = 4;
const KEY_LENGTH = 32;
const NONCE_LENGTH = 12;

/**
 * Whether the native Web Crypto API is available.
 * It's only present in Secure Contexts (HTTPS / localhost).
 */
const hasSubtle = typeof crypto !== "undefined" && !!crypto.subtle;

/* ------------------------------------------------------------------ */
/*  Internal key cache                                                 */
/* ------------------------------------------------------------------ */

/** Native CryptoKey – used when Web Crypto is available */
let cachedNativeKey: CryptoKey | null = null;
/** Raw 32-byte key – used as fallback when Web Crypto is unavailable */
let cachedRawKey: Uint8Array | null = null;
/**
 * Most recently derived key as hex, held ONLY in memory (never persisted to
 * any web storage). It exists so login/setup can hand the raw key to the
 * server — which legitimately needs it to wrap for autonomous autoscan /
 * migration — without ever writing the raw bytes to sessionStorage. Lost on
 * page refresh and cleared on logout.
 */
let lastDerivedHex: string | null = null;

/** sessionStorage entry. Holds the literal "present" when the real key lives
 *  in IndexedDB (preferred), or raw hex only in no-SubtleCrypto fallback. */
const KEY_FLAG = "sp_key";

export function hasCryptoKey(): boolean {
  return (
    cachedNativeKey !== null ||
    cachedRawKey !== null ||
    sessionStorage.getItem(KEY_FLAG) !== null
  );
}

/* ------------------------------------------------------------------ */
/*  Non-extractable key persistence (IndexedDB)                        */
/* ------------------------------------------------------------------ */
//
// The imported AES key is `extractable: false`. We persist the CryptoKey
// *object* (not its bytes) to IndexedDB so a page refresh can reload it
// without re-running Argon2 — and crucially without ever writing the raw key
// to storage. An XSS attacker can still *use* the key via the handle, but
// cannot exfiltrate the raw material, which the old sessionStorage-hex design
// allowed.

const KEYSTORE_DB = "sp-keystore";
const KEYSTORE_STORE = "keys";
const KEYSTORE_ID = "current";

function openKeystore(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(KEYSTORE_DB, 1);
    req.onupgradeneeded = () => {
      const idb = req.result;
      if (!idb.objectStoreNames.contains(KEYSTORE_STORE)) {
        idb.createObjectStore(KEYSTORE_STORE);
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function idbPutKey(key: CryptoKey): Promise<void> {
  const idb = await openKeystore();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = idb.transaction(KEYSTORE_STORE, "readwrite");
      tx.objectStore(KEYSTORE_STORE).put(key, KEYSTORE_ID);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  } finally {
    idb.close();
  }
}

async function idbGetKey(): Promise<CryptoKey | null> {
  const idb = await openKeystore();
  try {
    return await new Promise<CryptoKey | null>((resolve, reject) => {
      const tx = idb.transaction(KEYSTORE_STORE, "readonly");
      const req = tx.objectStore(KEYSTORE_STORE).get(KEYSTORE_ID);
      req.onsuccess = () => resolve((req.result as CryptoKey) ?? null);
      req.onerror = () => reject(req.error);
    });
  } finally {
    idb.close();
  }
}

async function idbClearKey(): Promise<void> {
  const idb = await openKeystore();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = idb.transaction(KEYSTORE_STORE, "readwrite");
      tx.objectStore(KEYSTORE_STORE).delete(KEYSTORE_ID);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  } finally {
    idb.close();
  }
}

/* ------------------------------------------------------------------ */
/*  SHA-256 helper (works in any context)                              */
/* ------------------------------------------------------------------ */

async function sha256Digest(data: Uint8Array): Promise<Uint8Array> {
  if (hasSubtle) {
    const buf = await crypto.subtle.digest("SHA-256", data as BufferSource);
    return new Uint8Array(buf);
  }
  // Pure-JS fallback via @noble/hashes
  return sha256(data);
}

/* ------------------------------------------------------------------ */
/*  Key derivation                                                     */
/* ------------------------------------------------------------------ */

/**
 * Derive the AES-256-GCM encryption key from the user's password.
 *
 * The salt is deterministically derived from SHA-256("simple-photos:" + username)
 * so the same username + password always produces the same key — no separate
 * passphrase step needed. The password never leaves the browser in raw form for
 * this purpose; Argon2id makes brute-force infeasible.
 *
 * Returns the derived key as hex. This is the ONLY place the raw key bytes are
 * exposed — callers that must hand the key to the server (so it can wrap it for
 * autonomous autoscan/migration) should use this return value rather than
 * reading it back from storage, which no longer holds the raw key.
 */
export async function deriveKey(password: string, username: string): Promise<string> {
  // Deterministic 16-byte salt = first 16 bytes of SHA-256("simple-photos:" + username)
  const saltInput = new TextEncoder().encode("simple-photos:" + username);
  const saltHash = await sha256Digest(saltInput);
  const salt = saltHash.slice(0, 16);

  const keyBytes = await argon2id({
    password,
    salt,
    parallelism: ARGON2_PARALLELISM,
    iterations: ARGON2_TIME,
    memorySize: ARGON2_MEMORY,
    hashLength: KEY_LENGTH,
    outputType: "binary",
  });

  const rawKey = new Uint8Array((keyBytes as Uint8Array).buffer as ArrayBuffer);

  if (hasSubtle) {
    cachedNativeKey = await crypto.subtle.importKey(
      "raw",
      rawKey.buffer as ArrayBuffer,
      { name: "AES-GCM" },
      false,
      ["encrypt", "decrypt"]
    );
    cachedRawKey = null;
    // Persist the non-extractable CryptoKey to IndexedDB; only a presence flag
    // (never the key bytes) goes to sessionStorage.
    try {
      await idbPutKey(cachedNativeKey);
      sessionStorage.setItem(KEY_FLAG, "present");
    } catch {
      // IndexedDB unavailable (e.g. private browsing) — degrade to the legacy
      // raw-hex sessionStorage path so a refresh still works.
      sessionStorage.setItem(KEY_FLAG, arrayToHex(rawKey));
    }
  } else {
    // No SubtleCrypto (insecure context): the raw key in sessionStorage is the
    // only way to survive a refresh; these contexts can't do better.
    cachedRawKey = rawKey;
    cachedNativeKey = null;
    sessionStorage.setItem(KEY_FLAG, arrayToHex(rawKey));
  }

  const hex = arrayToHex(rawKey);
  lastDerivedHex = hex;
  return hex;
}

/**
 * The most recently derived key as hex, held only in memory (never persisted).
 * Used by login/setup to hand the key to the server for autonomous
 * autoscan/migration. Returns null after logout ([`clearKey`]) or a refresh —
 * callers that have the hex directly (from [`deriveKey`]'s return) should
 * prefer that.
 */
export function getDerivedKeyHex(): string | null {
  return lastDerivedHex;
}

export async function loadKeyFromSession(): Promise<boolean> {
  if (cachedNativeKey || cachedRawKey) return true;

  const stored = sessionStorage.getItem(KEY_FLAG);
  if (!stored) return false;

  if (hasSubtle) {
    // Preferred path: non-extractable CryptoKey from IndexedDB.
    if (stored === "present") {
      try {
        const k = await idbGetKey();
        if (k) {
          cachedNativeKey = k;
          return true;
        }
      } catch {
        /* fall through — treat as no key */
      }
      // Flag claimed a key but IndexedDB has none — force re-auth.
      return false;
    }

    // Legacy / fallback path: `stored` is raw hex (pre-upgrade session or an
    // IndexedDB-write failure). Import it, migrate into IndexedDB, and drop the
    // raw bytes from sessionStorage so they're never persisted again.
    try {
      const keyBytes = hexToArray(stored);
      cachedNativeKey = await crypto.subtle.importKey(
        "raw",
        keyBytes.buffer as ArrayBuffer,
        { name: "AES-GCM" },
        false,
        ["encrypt", "decrypt"]
      );
      try {
        await idbPutKey(cachedNativeKey);
        sessionStorage.setItem(KEY_FLAG, "present");
      } catch {
        /* keep the hex fallback if IndexedDB still won't take it */
      }
      return true;
    } catch {
      return false;
    }
  }

  // No SubtleCrypto: the only persisted form is raw hex.
  if (stored === "present") return false; // can't reconstruct without subtle
  cachedRawKey = hexToArray(stored);
  return true;
}

/* ------------------------------------------------------------------ */
/*  Encrypt / Decrypt                                                  */
/* ------------------------------------------------------------------ */

export async function encrypt(plaintext: Uint8Array): Promise<ArrayBuffer> {
  if (!cachedNativeKey && !cachedRawKey) {
    const loaded = await loadKeyFromSession();
    if (!loaded) throw new Error("No encryption key available");
  }

  const nonce = crypto.getRandomValues(new Uint8Array(NONCE_LENGTH));

  if (hasSubtle && cachedNativeKey) {
    const ciphertext = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce as BufferSource },
      cachedNativeKey,
      plaintext as BufferSource
    );
    // Format: [12-byte nonce][ciphertext + 16-byte auth tag]
    const result = new Uint8Array(NONCE_LENGTH + ciphertext.byteLength);
    result.set(nonce, 0);
    result.set(new Uint8Array(ciphertext), NONCE_LENGTH);
    return result.buffer;
  }

  // Pure-JS fallback via @noble/ciphers
  const aes = gcm(cachedRawKey!, nonce);
  const ciphertext = aes.encrypt(plaintext);
  // ciphertext already includes the 16-byte auth tag
  const result = new Uint8Array(NONCE_LENGTH + ciphertext.length);
  result.set(nonce, 0);
  result.set(ciphertext, NONCE_LENGTH);
  return result.buffer;
}

export async function decrypt(encrypted: ArrayBuffer): Promise<Uint8Array> {
  if (!cachedNativeKey && !cachedRawKey) {
    const loaded = await loadKeyFromSession();
    if (!loaded) throw new Error("No encryption key available");
  }

  const data = new Uint8Array(encrypted);
  const nonce = data.slice(0, NONCE_LENGTH);
  const ciphertext = data.slice(NONCE_LENGTH);

  if (hasSubtle && cachedNativeKey) {
    const plaintext = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: nonce as BufferSource },
      cachedNativeKey,
      ciphertext as BufferSource
    );
    return new Uint8Array(plaintext);
  }

  // Pure-JS fallback
  const aes = gcm(cachedRawKey!, nonce);
  return aes.decrypt(ciphertext);
}

/* ------------------------------------------------------------------ */
/*  Utilities                                                          */
/* ------------------------------------------------------------------ */

export async function sha256Hex(data: Uint8Array): Promise<string> {
  const hash = await sha256Digest(data);
  return arrayToHex(hash);
}

export function clearKey(): void {
  cachedNativeKey = null;
  cachedRawKey = null;
  lastDerivedHex = null;
  sessionStorage.removeItem(KEY_FLAG);
  // Best-effort wipe of the persisted CryptoKey (fire-and-forget so callers
  // can stay synchronous).
  void idbClearKey().catch(() => {});
}

function arrayToHex(arr: Uint8Array): string {
  return Array.from(arr)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function hexToArray(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    bytes[i / 2] = parseInt(hex.substring(i, i + 2), 16);
  }
  return bytes;
}
