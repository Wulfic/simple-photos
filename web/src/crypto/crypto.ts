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

export function hasCryptoKey(): boolean {
  return (
    cachedNativeKey !== null ||
    cachedRawKey !== null ||
    sessionStorage.getItem("sp_key") !== null
  );
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
 */
export async function deriveKey(password: string, username: string): Promise<void> {
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
  } else {
    cachedRawKey = rawKey;
    cachedNativeKey = null;
  }

  // Store raw key in sessionStorage (cleared on tab close)
  sessionStorage.setItem("sp_key", arrayToHex(rawKey));
}

export async function loadKeyFromSession(): Promise<boolean> {
  const hexKey = sessionStorage.getItem("sp_key");
  if (!hexKey) return false;

  const keyBytes = hexToArray(hexKey);

  if (hasSubtle) {
    cachedNativeKey = await crypto.subtle.importKey(
      "raw",
      keyBytes.buffer as ArrayBuffer,
      { name: "AES-GCM" },
      false,
      ["encrypt", "decrypt"]
    );
  } else {
    cachedRawKey = keyBytes;
  }
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
  sessionStorage.removeItem("sp_key");
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
