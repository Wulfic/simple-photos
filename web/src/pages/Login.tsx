import { useState } from "react";
import { useNavigate, Link } from "react-router-dom";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { deriveKey } from "../crypto/crypto";
import ThemeToggle from "../components/ThemeToggle";

export default function Login() {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [totpSession, setTotpSession] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

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
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Password
                </label>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                  required
                />
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

        <p className="text-center text-sm text-gray-600 dark:text-gray-400 mt-4">
          Don't have an account?{" "}
          <Link to="/register" className="text-blue-600 hover:underline">
            Register
          </Link>
        </p>
      </div>
    </div>
  );
}
