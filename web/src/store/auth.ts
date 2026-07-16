/**
 * Zustand store for authentication state.
 *
 * Persists access/refresh tokens and username to localStorage so sessions
 * survive page reloads. The access token is sent as a Bearer header on
 * every API request (see api/core.ts).
 */
import { create } from "zustand";
import { clearGalleryToken } from "../utils/galleryToken";
import { clearMaterializedAt } from "../utils/takeoutLatch";
import { clearKey } from "../crypto/crypto";

interface AuthState {
  accessToken: string | null;
  refreshToken: string | null;
  username: string | null;
  isAuthenticated: boolean;
  setTokens: (access: string, refresh: string) => void;
  setUsername: (username: string) => void;
  logout: () => void;
  loadFromStorage: () => void;
}

export const useAuthStore = create<AuthState>((set) => ({
  accessToken: null,
  refreshToken: null,
  username: null,
  isAuthenticated: false,

  setTokens: (access, refresh) => {
    localStorage.setItem("sp_access_token", access);
    localStorage.setItem("sp_refresh_token", refresh);
    set({ accessToken: access, refreshToken: refresh, isAuthenticated: true });
  },

  setUsername: (username) => {
    localStorage.setItem("sp_username", username);
    set({ username });
  },

  logout: () => {
    // Read before the username is removed — the latch is keyed by it.
    const user = localStorage.getItem("sp_username");
    localStorage.removeItem("sp_access_token");
    localStorage.removeItem("sp_refresh_token");
    localStorage.removeItem("sp_username");
    // Logout wipes the local photo mirror, so the Takeout reconstruction latch
    // ("already materialized at N photos") describes a library that no longer
    // exists. Left in place, the rebuilt mirror could coincidentally reach N
    // again and the pass would skip itself forever.
    if (user) clearMaterializedAt(user);
    // Drop the persisted smart-album count summary (see usePhotoSummary) so the
    // next account on this browser never flashes the previous user's counts.
    localStorage.removeItem("sp_photo_summary_v1");
    // Drop the secure-album unlock token so a re-login must re-unlock.
    clearGalleryToken();
    // Always wipe the in-memory + sessionStorage AES key on logout. Doing it
    // here (rather than relying on each caller) guarantees no logout path —
    // e.g. an admin-triggered forced logout — leaves the key recoverable.
    clearKey();
    set({
      accessToken: null,
      refreshToken: null,
      username: null,
      isAuthenticated: false,
    });
  },

  loadFromStorage: () => {
    const accessToken = localStorage.getItem("sp_access_token");
    const refreshToken = localStorage.getItem("sp_refresh_token");
    const username = localStorage.getItem("sp_username");
    if (accessToken && refreshToken) {
      set({ accessToken, refreshToken, username, isAuthenticated: true });
    }
  },
}));
