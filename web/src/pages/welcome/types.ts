export type WizardStep =
  | "loading"
  | "welcome"
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
