import { useState, useMemo } from "react";
import { useNavigate, Link } from "react-router-dom";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { deriveKey } from "../crypto/crypto";
import ThemeToggle from "../components/ThemeToggle";

/** Password strength rules — mirrors server-side validation. */
function checkPasswordStrength(pw: string) {
  const checks = {
    length: pw.length >= 8,
    uppercase: /[A-Z]/.test(pw),
    lowercase: /[a-z]/.test(pw),
    digit: /\d/.test(pw),
    long: pw.length >= 12,
    special: /[^A-Za-z0-9]/.test(pw),
  };
  const core = checks.length && checks.uppercase && checks.lowercase && checks.digit;
  const score = Object.values(checks).filter(Boolean).length;
  const label =
    score <= 2 ? "Weak" : score <= 3 ? "Fair" : score <= 4 ? "Good" : "Strong";
  const color =
    score <= 2
      ? "bg-red-500"
      : score <= 3
        ? "bg-yellow-500"
        : score <= 4
          ? "bg-blue-500"
          : "bg-green-500";
  return { checks, core, score, label, color, max: 6 };
}

/** Username rules — mirrors server-side validation. */
function checkUsername(name: string) {
  return {
    length: name.length >= 3 && name.length <= 50,
    chars: /^[a-zA-Z0-9_]+$/.test(name),
  };
}

export default function Register() {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const navigate = useNavigate();
  const { setTokens, setUsername: storeSetUsername } = useAuthStore();

  const pw = useMemo(() => checkPasswordStrength(password), [password]);
  const un = useMemo(() => checkUsername(username), [username]);

  async function handleRegister(e: React.FormEvent) {
    e.preventDefault();
    setError("");

    // Client-side validation (mirrors server rules)
    if (!un.length || !un.chars) {
      setError("Username must be 3–50 characters: letters, numbers, underscores only.");
      return;
    }
    if (!pw.core) {
      setError(
        "Password must be at least 8 characters with uppercase, lowercase, and a digit."
      );
      return;
    }
    if (password !== confirmPassword) {
      setError("Passwords do not match.");
      return;
    }

    setLoading(true);
    try {
      await api.auth.register(username, password);
      // Auto-login after registration
      const loginRes = await api.auth.login(username, password);
      if (loginRes.access_token && loginRes.refresh_token) {
        setTokens(loginRes.access_token, loginRes.refresh_token);
        storeSetUsername(username);
        // Derive encryption key from the password
        await deriveKey(password, username);
        navigate("/gallery");
      } else {
        navigate("/login");
      }
    } catch (err: any) {
      setError(err.message || "Registration failed");
    } finally {
      setLoading(false);
    }
  }

  const Checkmark = ({ ok }: { ok: boolean }) => (
    <span className={ok ? "text-green-600 dark:text-green-400" : "text-gray-400"}>
      {ok ? "✓" : "○"}
    </span>
  );

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
      <ThemeToggle />
      <div className="max-w-md w-full bg-white dark:bg-gray-800 rounded-lg shadow p-8">
        <div className="flex flex-col items-center mb-6">
          <img src="/logo.png" alt="Simple Photos" className="w-16 h-16 mb-2" />
          <h1 className="text-2xl font-bold text-center">Create Account</h1>
        </div>
        <form onSubmit={handleRegister} className="space-y-4">
          {/* Username */}
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
              minLength={3}
              maxLength={50}
              autoComplete="username"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              autoFocus
            />
            {username.length > 0 && (
              <ul className="text-xs mt-1 space-y-0.5">
                <li>
                  <Checkmark ok={un.length} /> 3–50 characters
                </li>
                <li>
                  <Checkmark ok={un.chars} /> Letters, numbers, underscores only
                </li>
              </ul>
            )}
          </div>

          {/* Password */}
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
              minLength={8}
              maxLength={128}
              autoComplete="new-password"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
            />

            {/* Strength bar */}
            {password.length > 0 && (
              <div className="mt-2">
                <div className="flex items-center gap-2 mb-1">
                  <div className="flex-1 h-1.5 bg-gray-200 dark:bg-gray-600 rounded-full overflow-hidden">
                    <div
                      className={`h-full rounded-full transition-all duration-300 ${pw.color}`}
                      style={{ width: `${(pw.score / pw.max) * 100}%` }}
                    />
                  </div>
                  <span className="text-xs font-medium text-gray-600 dark:text-gray-400 w-12 text-right">
                    {pw.label}
                  </span>
                </div>
                <ul className="text-xs space-y-0.5">
                  <li>
                    <Checkmark ok={pw.checks.length} /> At least 8 characters
                  </li>
                  <li>
                    <Checkmark ok={pw.checks.uppercase} /> Uppercase letter
                  </li>
                  <li>
                    <Checkmark ok={pw.checks.lowercase} /> Lowercase letter
                  </li>
                  <li>
                    <Checkmark ok={pw.checks.digit} /> Number
                  </li>
                  <li>
                    <Checkmark ok={pw.checks.special} /> Special character
                    <span className="text-gray-400"> (optional)</span>
                  </li>
                  <li>
                    <Checkmark ok={pw.checks.long} /> 12+ characters
                    <span className="text-gray-400"> (recommended)</span>
                  </li>
                </ul>
              </div>
            )}
          </div>

          {/* Confirm password */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Confirm Password
            </label>
            <input
              type="password"
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
              className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
              required
              autoComplete="new-password"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
            />
            {confirmPassword.length > 0 && password !== confirmPassword && (
              <p className="text-xs text-red-500 dark:text-red-400 mt-1">Passwords do not match</p>
            )}
          </div>

          {error && (
            <p className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded">{error}</p>
          )}

          <button
            type="submit"
            disabled={loading || !pw.core}
            className="w-full bg-blue-600 text-white py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 transition-colors"
          >
            {loading ? "Creating account..." : "Register"}
          </button>
        </form>

        <p className="text-center text-sm text-gray-600 dark:text-gray-400 mt-4">
          Already have an account?{" "}
          <Link to="/login" className="text-blue-600 hover:underline">
            Sign In
          </Link>
        </p>
      </div>
    </div>
  );
}
