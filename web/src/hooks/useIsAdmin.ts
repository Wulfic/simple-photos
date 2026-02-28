import { useAuthStore } from "../store/auth";

/**
 * Check if the current user has the admin role by decoding the JWT payload.
 * Returns false when there is no token or decoding fails.
 */
export function useIsAdmin(): boolean {
  const { accessToken } = useAuthStore();
  if (!accessToken) return false;
  try {
    const payload = JSON.parse(atob(accessToken.split(".")[1]));
    return payload.role === "admin";
  } catch {
    return false;
  }
}
