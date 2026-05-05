/**
 * First-run setup wizard API client.
 *
 * Wraps the small set of endpoints under `/api/setup/*`. These are the only
 * authenticated endpoints reachable while `wizard_completed === false`;
 * every other API call returns 403 with `error_code: "wizard_incomplete"`
 * until {@link setupApi.finalize} succeeds.
 */
import { request } from "./core";

export interface SetupStatusResponse {
  setup_complete: boolean;
  wizard_completed: boolean;
  registration_open: boolean;
  version: string;
  mode: string;
  setup_id?: string;
}

export interface SetupFinalizeResponse {
  wizard_completed: boolean;
  message: string;
}

export const setupApi = {
  status: () => request<SetupStatusResponse>("/setup/status"),

  /**
   * Mark the first-run wizard as fully complete.
   * Called by `CompleteStep` when the admin clicks "Go to Gallery".
   */
  finalize: () =>
    request<SetupFinalizeResponse>("/setup/finalize", { method: "POST" }),
};
