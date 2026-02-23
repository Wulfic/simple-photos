import { argon2id } from "hash-wasm";

const ARGON2_MEMORY = 65536;
const ARGON2_TIME = 3;
const ARGON2_PARALLELISM = 4;
const KEY_LENGTH = 32;
const NONCE_LENGTH = 12;

let cachedKey: CryptoKey | null = null;

export function hasCryptoKey(): boolean {
  return cachedKey !== null || sessionStorage.getItem("sp_key") !== null;
}

/**
 * Derive the AES-256-GCM encryption key from the user's password.
 *
 * The salt is deterministically derived from SHA-256(username) so the same
 * username + password always produces the same key — no separate passphrase
 * step needed. The password never leaves the browser in raw form for this
 * purpose; Argon2id makes brute-force infeasible.
 */
export async function deriveKey(password: string, username: string): Promise<void> {
  // Deterministic 16-byte salt = first 16 bytes of SHA-256("simple-photos:" + username)
  const saltInput = new TextEncoder().encode("simple-photos:" + username);
  const saltHash = await crypto.subtle.digest("SHA-256", saltInput);
  const salt = new Uint8Array(saltHash).slice(0, 16);

  const keyBytes = await argon2id({
    password,
    salt,
    parallelism: ARGON2_PARALLELISM,
    iterations: ARGON2_TIME,
    memorySize: ARGON2_MEMORY,
    hashLength: KEY_LENGTH,
    outputType: "binary",
  });

  const keyBuffer = (keyBytes as Uint8Array).buffer as ArrayBuffer;
  cachedKey = await crypto.subtle.importKey(
    "raw",
    keyBuffer,
    { name: "AES-GCM" },
    false,
    ["encrypt", "decrypt"]
  );

  // Store raw key in sessionStorage (cleared on tab close)
  sessionStorage.setItem("sp_key", arrayToHex(new Uint8Array(keyBuffer)));
}

export async function loadKeyFromSession(): Promise<boolean> {
  const hexKey = sessionStorage.getItem("sp_key");
  if (!hexKey) return false;

  const keyBytes = hexToArray(hexKey);
  cachedKey = await crypto.subtle.importKey(
    "raw",
    keyBytes.buffer as ArrayBuffer,
    { name: "AES-GCM" },
    false,
    ["encrypt", "decrypt"]
  );
  return true;
}

/** @deprecated Salt is now derived deterministically from the username. */
export function getSalt(): Uint8Array | null {
  const hex = sessionStorage.getItem("sp_salt");
  return hex ? hexToArray(hex) : null;
}

export async function encrypt(plaintext: Uint8Array): Promise<ArrayBuffer> {
  if (!cachedKey) {
    const loaded = await loadKeyFromSession();
    if (!loaded) throw new Error("No encryption key available");
  }

  const nonce = crypto.getRandomValues(new Uint8Array(NONCE_LENGTH));
  const ciphertext = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: nonce as BufferSource },
    cachedKey!,
    plaintext as BufferSource
  );

  // Format: [12-byte nonce][ciphertext + 16-byte auth tag]
  const result = new Uint8Array(NONCE_LENGTH + ciphertext.byteLength);
  result.set(nonce, 0);
  result.set(new Uint8Array(ciphertext), NONCE_LENGTH);
  return result.buffer;
}

export async function decrypt(encrypted: ArrayBuffer): Promise<Uint8Array> {
  if (!cachedKey) {
    const loaded = await loadKeyFromSession();
    if (!loaded) throw new Error("No encryption key available");
  }

  const data = new Uint8Array(encrypted);
  const nonce = data.slice(0, NONCE_LENGTH);
  const ciphertext = data.slice(NONCE_LENGTH);

  const plaintext = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: nonce as BufferSource },
    cachedKey!,
    ciphertext as BufferSource
  );

  return new Uint8Array(plaintext);
}

export async function sha256Hex(data: Uint8Array): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", data as BufferSource);
  return arrayToHex(new Uint8Array(hash));
}

export function clearKey(): void {
  cachedKey = null;
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
