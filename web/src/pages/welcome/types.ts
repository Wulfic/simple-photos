export type WizardStep =
  | "loading"
  | "welcome"
  | "server-role"
  | "pair"
  | "account"
  | "admin-2fa"
  | "storage"
  | "backup"
  | "ssl"
  | "encryption"
  | "users"
  | "user-2fa"
  | "android"
  | "complete";

export type ServerRole = "primary" | "backup" | null;

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
