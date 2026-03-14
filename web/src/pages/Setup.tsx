/**
 * Encryption key unlock / re-derivation page.
 *
 * Shown when the user is authenticated but the in-memory AES encryption key
 * is missing (e.g. after a page refresh in encrypted mode). Prompts for the
 * password and re-derives the key via Argon2id.
 */
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { deriveKey } from "../crypto/crypto";
import { useAuthStore } from "../store/auth";
import ThemeToggle from "../components/ThemeToggle";
import { getErrorMessage } from "../utils/formatters";

/**
 * Encryption unlock page.
 *
 * When the user's session has expired (tab closed) but they're still
 * authenticated, they need to re-enter their password to re-derive the
 * encryption key. The key is deterministically derived from their
 * username + password via Argon2id.
 */
export default function Setup() {
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const navigate = useNavigate();
  const { username } = useAuthStore();

  async function handleUnlock(e: React.FormEvent) {
    e.preventDefault();
    setError("");

    if (!username) {
      setError("No username found. Please log in again.");
      return;
    }

    setLoading(true);
    try {
      await deriveKey(password, username);
      navigate("/gallery");
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Key derivation failed"));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
      <ThemeToggle />
      <div className="max-w-md w-full bg-white dark:bg-gray-800 rounded-lg shadow p-8">
        <img src="/logo.png" alt="Simple Photos" className="w-14 h-14 mx-auto mb-3" />
        <h1 className="text-2xl font-bold text-center mb-2">Unlock Photos</h1>
        <p className="text-gray-600 dark:text-gray-400 text-sm text-center mb-6">
          Enter your password to unlock your encrypted photos. Your encryption
          key is derived from your password and never stored permanently.
        </p>
        <form onSubmit={handleUnlock} className="space-y-4">
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
              autoFocus
              autoComplete="current-password"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
            />
          </div>

          {error && <p className="text-red-600 dark:text-red-400 text-sm">{error}</p>}

          <button
            type="submit"
            disabled={loading}
            className="w-full bg-blue-600 text-white py-2 rounded-md hover:bg-blue-700 disabled:opacity-50"
          >
            {loading ? "Deriving key..." : "Unlock"}
          </button>
        </form>
      </div>
    </div>
  );
}
