/**
 * Encryption key storage API client.
 *
 * The server always operates in encrypted mode (AES-256-GCM).
 * This module handles persisting the client-derived key so the server
 * can process photos autonomously (autoscan).
 *
 * Maps to server route:
 *   `POST /api/admin/encryption/store-key`
 */
import { request } from "./core";

/** Server-authoritative encryption progress — the single source of truth for
 *  the "Encrypting photos…" banner on every client (item #1).
 *  Maps to `GET /api/status/encryption`. */
export interface EncryptionStatus {
  /** `true` while any item (server + client-contributed) is pending. */
  active: boolean;
  /** Batch denominator for the progress bar. */
  total: number;
  /** Items completed in the current batch. */
  done: number;
  /** Total items still pending (server + all client contributions). */
  pending: number;
  /** Server-visible pending count. */
  server_pending: number;
  /** Sum of all client-reported pending counts. */
  client_pending: number;
  /**
   * Photos the server gave up encrypting after three hard failures. Their
   * **originals stay in plaintext at rest** and nothing retries them.
   *
   * Deliberately NOT part of `pending`/`total`/`done`: parked photos are not
   * queued work, and folding them into the progress bar wedges it at a
   * non-zero count forever — which is why they were excluded from every count
   * in the first place, and how 2,500 of them (~17% of the live library) went
   * unnoticed for a month. Render this as a standing warning with its own
   * remedy (`retryParked`), never as remaining progress.
   */
  parked: number;
  /** Estimated seconds remaining, or `null` until throughput is known. */
  eta_seconds: number | null;
  /** Per-source breakdown for debug UIs (`server`, `android-…`, etc.). */
  sources: Record<string, number>;
}

export const encryptionApi = {
  /** Persist the client-derived encryption key so the server can encrypt
   *  photos autonomously (autoscan). Idempotent. */
  storeKey: (keyHex: string) =>
    request<{ ok: boolean }>("/admin/encryption/store-key", {
      method: "POST",
      body: JSON.stringify({ key_hex: keyHex }),
    }),

  /** Fetch the aggregated, server-authoritative encryption progress. */
  status: () => request<EncryptionStatus>("/status/encryption"),

  /** Re-admit every parked photo to the encryption queue, restoring a full
   *  attempt budget, and kick a migration pass. Admin-only, idempotent, and
   *  scoped to the caller's own photos. Returns how many were cleared. */
  retryParked: () =>
    request<{ cleared: number }>("/admin/encryption/retry-parked", {
      method: "POST",
    }),

  /** Report this client's own queued-upload count so the server total
   *  includes work it can't see yet (item #2). `source` is a stable
   *  per-device id; `pending` of 0 clears the contribution. */
  contribute: (source: string, pending: number) =>
    request<{ ok: boolean }>("/status/encryption/contribute", {
      method: "POST",
      body: JSON.stringify({ source, pending }),
    }),
};
