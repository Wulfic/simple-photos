/** Shared TypeScript types for the welcome wizard (step names, server roles, install types). */
export type WizardStep =
  | "loading"
  | "welcome"
  | "server-role"
  | "install-type"
  | "restore"
  | "pair"
  | "account"
  | "admin-2fa"
  | "storage"
  | "ssl"
  | "users"
  | "user-2fa"
  | "android"
  | "complete";

export type ServerRole = "primary" | "backup" | null;
export type InstallType = "fresh" | "restore" | null;

/** A server discovered via UDP broadcast on the LAN. */
export interface DiscoveredServer {
  address: string;
  name: string;
  version: string;
}

export interface SetupStatus {
  setup_complete: boolean;
  registration_open: boolean;
  version: string;
}

export interface CreatedUser {
  user_id: string;
  username: string;
  role: string;
}

export interface RestoreSource {
  address: string;
  name: string;
  version: string;
  api_key: string | null;
  photo_count: number;
}
