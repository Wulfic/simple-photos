/**
 * Encryption settings API client — the server always operates in encrypted mode.
 *
 * Maps to server routes:
 *   `GET  /api/settings/encryption`
 *   `POST /api/admin/encryption/store-key`
 */
import { request } from "./core";

// ── Encryption Settings API ──────────────────────────────────────────────────

export const encryptionApi = {
  /** Returns the encryption settings (always "encrypted" mode). */
  getSettings: () =>
    request<{
      encryption_mode: string;
    }>("/settings/encryption"),

  /** Persist the client-derived encryption key so the server can encrypt
   *  photos autonomously (autoscan, auto-migration). Idempotent. */
  storeKey: (keyHex: string) =>
    request<{ ok: boolean }>("/admin/encryption/store-key", {
      method: "POST",
      body: JSON.stringify({ key_hex: keyHex }),
    }),
};
