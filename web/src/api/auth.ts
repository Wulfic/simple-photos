/**
 * Authentication API client — login, registration, TOTP 2FA, token refresh,
 * and password management.
 *
 * Maps to server routes: `POST /api/auth/*`, `PUT /api/auth/password`,
 * `GET /api/auth/2fa/status`.
 */
import { request } from "./core";
import type {
  RegisterResponse,
  LoginResponse,
  TotpSetupResponse,
  ChangePasswordResponse,
} from "./types";

// ── Authentication API ───────────────────────────────────────────────────────

export const authApi = {
  register: (username: string, password: string) =>
    request<RegisterResponse>("/auth/register", {
      method: "POST",
      body: JSON.stringify({ username, password }),
    }),

  login: (username: string, password: string) =>
    request<LoginResponse>("/auth/login", {
      method: "POST",
      body: JSON.stringify({ username, password }),
    }),

  loginTotp: (
    totp_session_token: string,
    totp_code?: string,
    backup_code?: string
  ) =>
    request<{
      access_token: string;
      refresh_token: string;
      expires_in: number;
    }>("/auth/login/totp", {
      method: "POST",
      body: JSON.stringify({ totp_session_token, totp_code, backup_code }),
    }),

  refresh: (refresh_token: string) =>
    request<{
      access_token: string;
      refresh_token: string;
      expires_in: number;
    }>("/auth/refresh", {
      method: "POST",
      body: JSON.stringify({ refresh_token }),
    }),

  logout: (refresh_token: string) =>
    request<void>("/auth/logout", {
      method: "POST",
      body: JSON.stringify({ refresh_token }),
    }),

  changePassword: (currentPassword: string, newPassword: string) =>
    request<ChangePasswordResponse>("/auth/password", {
      method: "PUT",
      body: JSON.stringify({
        current_password: currentPassword,
        new_password: newPassword,
      }),
    }),

  setup2fa: () =>
    request<TotpSetupResponse>("/auth/2fa/setup", { method: "POST" }),

  confirm2fa: (totp_code: string) =>
    request<void>("/auth/2fa/confirm", {
      method: "POST",
      body: JSON.stringify({ totp_code }),
    }),

  disable2fa: (totp_code: string) =>
    request<void>("/auth/2fa/disable", {
      method: "POST",
      body: JSON.stringify({ totp_code }),
    }),
};
