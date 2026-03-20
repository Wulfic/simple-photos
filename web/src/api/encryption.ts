/**
 * Encryption key storage API client.
 *
 * The server always operates in encrypted mode (AES-256-GCM).
 * This module handles persisting the client-derived key so the server
 * can process photos autonomously (autoscan, conversion).
 *
 * Maps to server route:
 *   `POST /api/admin/encryption/store-key`
 */
import { request } from "./core";

export const encryptionApi = {
  /** Persist the client-derived encryption key so the server can encrypt
   *  photos autonomously (autoscan, conversion). Idempotent. */
  storeKey: (keyHex: string) =>
    request<{ ok: boolean }>("/admin/encryption/store-key", {
      method: "POST",
      body: JSON.stringify({ key_hex: keyHex }),
    }),
};
