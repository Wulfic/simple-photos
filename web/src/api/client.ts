/**
 * Barrel re-export — assembles the public `api` object from domain modules.
 *
 * Each domain module (auth, blobs, photos, etc.) lives in its own file
 * for maintainability. This file re-exports everything under the single
 * `api` namespace that the rest of the app imports.
 */

import { authApi } from "./auth";
import { blobsApi } from "./blobs";
import { adminApi } from "./admin";
import { photosApi } from "./photos";
import { encryptionApi } from "./encryption";
import { secureGalleriesApi } from "./galleries";
import { storageStatsApi, diagnosticsApi } from "./diagnostics";
import { trashApi } from "./trash";
import { backupApi } from "./backup";
import { sharingApi } from "./sharing";
import { tagsApi, searchApi } from "./tags";

// Re-export all types for backward compatibility
export type {
  RegisterResponse,
  LoginResponse,
  TotpSetupResponse,
  ChangePasswordResponse,
  DiagnosticsResponse,
  DiagnosticsResponseUnion,
  DisabledDiagnosticsResponse,
  DiagnosticsConfig,
  UpdateDiagnosticsConfigRequest,
  AuditLogEntry,
  AuditLogListResponse,
  AuditLogParams,
  ClientLogEntry,
  ClientLogListResponse,
  ClientLogParams,
} from "./types";

// ── Public API ───────────────────────────────────────────────────────────────

export const api = {
  auth: authApi,
  blobs: blobsApi,
  admin: adminApi,
  photos: photosApi,
  encryption: encryptionApi,
  secureGalleries: secureGalleriesApi,
  storageStats: storageStatsApi,
  trash: trashApi,
  backup: backupApi,
  sharing: sharingApi,
  tags: tagsApi,
  search: searchApi,
  diagnostics: diagnosticsApi,
};
