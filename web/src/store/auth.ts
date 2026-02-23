import { create } from "zustand";

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
    localStorage.removeItem("sp_access_token");
    localStorage.removeItem("sp_refresh_token");
    localStorage.removeItem("sp_username");
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
