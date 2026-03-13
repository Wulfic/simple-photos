import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { deriveKey } from "../crypto/crypto";
import { clearAllUserData } from "../db";
import { thumbMemoryCache } from "../utils/gallery";
import ThemeToggle from "../components/ThemeToggle";

export default function Login() {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [totpSession, setTotpSession] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [showPassword, setShowPassword] = useState(false);

  const { setTokens, setUsername: storeSetUsername } = useAuthStore();
  const navigate = useNavigate();

  async function handleLogin(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);

    try {
      if (totpSession) {
        // Detect whether the input is a 6-digit TOTP code or a backup code
        const is6Digit = /^\d{6}$/.test(totpCode.trim());
        const res = await api.auth.loginTotp(
          totpSession,
          is6Digit ? totpCode.trim() : undefined,
          is6Digit ? undefined : totpCode.trim()
        );
        // Clear stale data from a previous user session BEFORE setting
        // tokens — setTokens triggers isAuthenticated=true which causes
        // ProtectedLayout to immediately render Gallery.  If we clear
        // after, there's a race where Gallery reads stale IndexedDB data.
        await clearAllUserData().catch(() => {});
        thumbMemoryCache.clear();
        setTokens(res.access_token, res.refresh_token);
        storeSetUsername(username);
        // Derive encryption key from the login password
        await deriveKey(password, username);
        navigate("/gallery");
      } else {
        const res = await api.auth.login(username, password);
        if (res.requires_totp && res.totp_session_token) {
          setTotpSession(res.totp_session_token);
        } else if (res.access_token && res.refresh_token) {
          // Clear stale data BEFORE setting tokens (see comment above)
          await clearAllUserData().catch(() => {});
          thumbMemoryCache.clear();
          setTokens(res.access_token, res.refresh_token);
          storeSetUsername(username);
          // Derive encryption key from the login password
          await deriveKey(password, username);
          navigate("/gallery");
        }
      }
    } catch (err: any) {
      setError(err.message || "Login failed");
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
      <ThemeToggle />
      <div className="max-w-md w-full bg-white dark:bg-gray-800 rounded-lg shadow p-8">
        <div className="flex flex-col items-center mb-6">
          <img src="/logo.png" alt="Simple Photos" className="w-16 h-16 mb-2" />
          <h1 className="text-2xl font-bold text-center">Simple Photos</h1>
        </div>
        <form onSubmit={handleLogin} className="space-y-4">
          {!totpSession ? (
            <>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Username
                </label>
                <input
                  type="text"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                  required
                  autoFocus
                  autoComplete="username"
                  autoCorrect="off"
                  autoCapitalize="off"
                  spellCheck={false}
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Password
                </label>
                <div className="relative">
                  <input
                    type={showPassword ? "text" : "password"}
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    className="w-full border rounded-md px-3 py-2 pr-10 focus:outline-none focus:ring-2 focus:ring-blue-500"
                    required
                    autoComplete="current-password"
                    autoCorrect="off"
                    autoCapitalize="off"
                    spellCheck={false}
                  />
                  <button
                    type="button"
                    onClick={() => setShowPassword((s) => !s)}
                    className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 focus:outline-none"
                    tabIndex={-1}
                    aria-label={showPassword ? "Hide password" : "Show password"}
                  >
                    <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      {showPassword ? (
                        <>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M3.98 8.223A10.477 10.477 0 001.934 12c1.292 4.338 5.31 7.5 10.066 7.5.993 0 1.953-.138 2.863-.395M6.228 6.228A10.45 10.45 0 0112 4.5c4.756 0 8.773 3.162 10.065 7.498a10.523 10.523 0 01-4.293 5.774M6.228 6.228L3 3m3.228 3.228l3.65 3.65m7.894 7.894L21 21m-3.228-3.228l-3.65-3.65m0 0a3 3 0 10-4.243-4.243m4.242 4.242L9.88 9.88" />
                        </>
                      ) : (
                        <>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M2.036 12.322a1.012 1.012 0 010-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178z" />
                          <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                        </>
                      )}
                    </svg>
                  </button>
                </div>
              </div>
            </>
          ) : (
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Two-Factor Code
              </label>
              <input
                type="text"
                value={totpCode}
                onChange={(e) => setTotpCode(e.target.value)}
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="6-digit code or backup code"
                required
                autoFocus
                autoComplete="one-time-code"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                inputMode="numeric"
              />
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                Enter code from your authenticator app or a backup code
              </p>
            </div>
          )}

          {error && (
            <p className="text-red-600 dark:text-red-400 text-sm">{error}</p>
          )}

          <button
            type="submit"
            disabled={loading}
            className="w-full bg-blue-600 text-white py-2 rounded-md hover:bg-blue-700 disabled:opacity-50"
          >
            {loading ? "Signing in..." : totpSession ? "Verify" : "Sign In"}
          </button>
        </form>


      </div>
    </div>
  );
}
