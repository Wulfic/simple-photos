import { useState, useEffect, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { api, TotpSetupResponse } from "../api/client";
import { useAuthStore } from "../store/auth";
import { deriveKey } from "../crypto/crypto";
import ThemeToggle from "../components/ThemeToggle";

// ── Types ─────────────────────────────────────────────────────────────────────

interface SetupStatus {
  setup_complete: boolean;
  registration_open: boolean;
  version: string;
}

type WizardStep =
  | "loading"
  | "welcome"
  | "account"
  | "admin-2fa"
  | "storage"
  | "users"
  | "user-2fa"
  | "android"
  | "complete";

interface CreatedUser {
  user_id: string;
  username: string;
  role: string;
}

// ── Validation helpers ────────────────────────────────────────────────────────

function checkPasswordStrength(pw: string) {
  const checks = {
    length: pw.length >= 8,
    uppercase: /[A-Z]/.test(pw),
    lowercase: /[a-z]/.test(pw),
    digit: /\d/.test(pw),
    long: pw.length >= 12,
    special: /[^A-Za-z0-9]/.test(pw),
  };
  const core =
    checks.length && checks.uppercase && checks.lowercase && checks.digit;
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

function checkUsername(name: string) {
  return {
    length: name.length >= 3 && name.length <= 50,
    chars: /^[a-zA-Z0-9_]+$/.test(name),
  };
}

// ── Shared sub-components (defined outside Welcome to avoid re-mount on render) ──

const Checkmark = ({ ok }: { ok: boolean }) => (
  <span className={ok ? "text-green-600 dark:text-green-400" : "text-gray-400"}>
    {ok ? "\u2713" : "\u25CB"}
  </span>
);

/** Eye icon SVG for show/hide password toggle */
const EyeIcon = ({ open }: { open: boolean }) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    className="w-5 h-5 text-gray-400 hover:text-gray-600 dark:hover:text-gray-400 dark:text-gray-400 transition-colors"
    fill="none"
    viewBox="0 0 24 24"
    stroke="currentColor"
    strokeWidth={1.5}
  >
    {open ? (
      <>
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M2.036 12.322a1.012 1.012 0 010-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178a1.01 1.01 0 010 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178z"
        />
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
        />
      </>
    ) : (
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M3.98 8.223A10.477 10.477 0 001.934 12c1.292 4.338 5.31 7.5 10.066 7.5.993 0 1.953-.138 2.863-.395M6.228 6.228A10.45 10.45 0 0112 4.5c4.756 0 8.773 3.162 10.065 7.498a10.523 10.523 0 01-4.293 5.774M6.228 6.228L3 3m3.228 3.228l3.65 3.65m7.894 7.894L21 21m-3.228-3.228l-3.65-3.65m0 0a3 3 0 10-4.243-4.243m4.242 4.242L9.88 9.88"
      />
    )}
  </svg>
);

function PasswordField({
  value,
  onChange,
  pwData,
  label = "Password",
}: {
  value: string;
  onChange: (v: string) => void;
  pwData: ReturnType<typeof checkPasswordStrength>;
  label?: string;
}) {
  const [show, setShow] = useState(false);

  return (
    <div>
      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
        {label}
      </label>
      <div className="relative">
        <input
          type={show ? "text" : "password"}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 pr-11 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
          required
          minLength={8}
          maxLength={128}
          autoComplete="new-password"
        />
        <button
          type="button"
          onClick={() => setShow((s) => !s)}
          className="absolute right-3 top-1/2 -translate-y-1/2 focus:outline-none"
          tabIndex={-1}
          aria-label={show ? "Hide password" : "Show password"}
        >
          <EyeIcon open={show} />
        </button>
      </div>
      {value.length > 0 && (
        <div className="mt-2">
          <div className="flex items-center gap-2 mb-1">
            <div className="flex-1 h-1.5 bg-gray-200 dark:bg-gray-600 rounded-full overflow-hidden">
              <div
                className={`h-full rounded-full transition-all duration-300 ${pwData.color}`}
                style={{ width: `${(pwData.score / pwData.max) * 100}%` }}
              />
            </div>
            <span className="text-xs font-medium text-gray-600 dark:text-gray-400 w-12 text-right">
              {pwData.label}
            </span>
          </div>
          <ul className="text-xs space-y-0.5">
            <li>
              <Checkmark ok={pwData.checks.length} /> At least 8 characters
            </li>
            <li>
              <Checkmark ok={pwData.checks.uppercase} /> Uppercase letter
            </li>
            <li>
              <Checkmark ok={pwData.checks.lowercase} /> Lowercase letter
            </li>
            <li>
              <Checkmark ok={pwData.checks.digit} /> Number
            </li>
            <li>
              <Checkmark ok={pwData.checks.special} /> Special character
              <span className="text-gray-400"> (optional)</span>
            </li>
          </ul>
        </div>
      )}
    </div>
  );
}

/** Confirm password field with eye toggle */
function ConfirmPasswordField({
  value,
  onChange,
  password,
}: {
  value: string;
  onChange: (v: string) => void;
  password: string;
}) {
  const [show, setShow] = useState(false);

  return (
    <div>
      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
        Confirm Password
      </label>
      <div className="relative">
        <input
          type={show ? "text" : "password"}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 pr-11 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
          required
          autoComplete="new-password"
        />
        <button
          type="button"
          onClick={() => setShow((s) => !s)}
          className="absolute right-3 top-1/2 -translate-y-1/2 focus:outline-none"
          tabIndex={-1}
          aria-label={show ? "Hide password" : "Show password"}
        >
          <EyeIcon open={show} />
        </button>
      </div>
      {value.length > 0 && password !== value && (
        <p className="text-xs text-red-500 dark:text-red-400 mt-1">Passwords do not match</p>
      )}
    </div>
  );
}

// ── Component ─────────────────────────────────────────────────────────────────

export default function Welcome() {
  const [step, setStep] = useState<WizardStep>("loading");
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const navigate = useNavigate();

  // ── Admin account form ──────────────────────────────────────────────────
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");

  // ── 2FA state (shared between admin-2fa and user-2fa steps) ─────────────
  const [totpData, setTotpData] = useState<TotpSetupResponse | null>(null);
  const [totpCode, setTotpCode] = useState("");
  const [backupCodes, setBackupCodes] = useState<string[]>([]);
  const [totpConfirmed, setTotpConfirmed] = useState(false);

  // ── Storage ─────────────────────────────────────────────────────────────
  const [storagePath, setStoragePath] = useState("");
  const [browsePath, setBrowsePath] = useState("");
  const [browseParent, setBrowseParent] = useState<string | null>(null);
  const [browseDirs, setBrowseDirs] = useState<
    Array<{ name: string; path: string }>
  >([]);
  const [browseWritable, setBrowseWritable] = useState(false);
  const [browseLoading, setBrowseLoading] = useState(false);
  const [manualPathInput, setManualPathInput] = useState("");
  const [showManualInput, setShowManualInput] = useState(false);
  const [storageConfirmed, setStorageConfirmed] = useState(false);

  // ── Additional users ────────────────────────────────────────────────────
  const [createdUsers, setCreatedUsers] = useState<CreatedUser[]>([]);
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [newConfirmPassword, setNewConfirmPassword] = useState("");
  const [newRole, setNewRole] = useState<"user" | "admin">("user");
  const [showUserForm, setShowUserForm] = useState(false);
  // Track which newly-created user is being offered 2FA
  const [pendingTotpUser, setPendingTotpUser] = useState<CreatedUser | null>(
    null
  );

  const { setTokens, setUsername: storeSetUsername } = useAuthStore();

  const pw = useMemo(() => checkPasswordStrength(password), [password]);
  const un = useMemo(() => checkUsername(username), [username]);
  const newPw = useMemo(() => checkPasswordStrength(newPassword), [newPassword]);
  const newUn = useMemo(() => checkUsername(newUsername), [newUsername]);

  // ── Check setup status on mount ──────────────────────────────────────────

  useEffect(() => {
    checkSetupStatus();
  }, []);

  async function checkSetupStatus() {
    try {
      const res = await fetch("/api/setup/status");
      const data: SetupStatus = await res.json();
      setStatus(data);

      if (data.setup_complete) {
        navigate("/login", { replace: true });
      } else {
        setStep("welcome");
      }
    } catch {
      setError(
        "Cannot connect to the server. Make sure the Simple Photos server is running."
      );
      setStep("welcome");
    }
  }

  // ── Step handlers ─────────────────────────────────────────────────────────

  async function handleCreateAccount(e: React.FormEvent) {
    e.preventDefault();
    setError("");

    if (!un.length || !un.chars) {
      setError(
        "Username must be 3-50 characters: letters, numbers, underscores only."
      );
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
      await fetch("/api/setup/init", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username, password }),
      }).then(async (res) => {
        if (!res.ok) {
          const err = await res.json().catch(() => ({ error: "Setup failed" }));
          throw new Error(err.error || `HTTP ${res.status}`);
        }
      });

      const loginRes = await api.auth.login(username, password);
      if (loginRes.access_token && loginRes.refresh_token) {
        setTokens(loginRes.access_token, loginRes.refresh_token);
        storeSetUsername(username);
        await deriveKey(password, username);
        setStep("admin-2fa");
      } else {
        throw new Error("Unexpected login response");
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Setup failed");
    } finally {
      setLoading(false);
    }
  }

  // ── 2FA setup (for admin or informational for newly created user) ───────

  async function startTotpSetup() {
    setError("");
    setLoading(true);
    try {
      const data = await api.auth.setup2fa();
      setTotpData(data);
      setTotpCode("");
      setTotpConfirmed(false);
      setBackupCodes([]);
    } catch (err: unknown) {
      setError(
        err instanceof Error ? err.message : "Failed to start 2FA setup"
      );
    } finally {
      setLoading(false);
    }
  }

  async function confirmTotp(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      await api.auth.confirm2fa(totpCode);
      setTotpConfirmed(true);
      setBackupCodes(totpData?.backup_codes ?? []);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Invalid TOTP code");
    } finally {
      setLoading(false);
    }
  }

  function finishTotpStep() {
    setTotpData(null);
    setTotpCode("");
    setTotpConfirmed(false);
    setBackupCodes([]);

    if (step === "admin-2fa") {
      loadStoragePath();
    } else if (step === "user-2fa") {
      setPendingTotpUser(null);
      setStep("users");
    }
  }

  function skipTotpStep() {
    setTotpData(null);
    if (step === "admin-2fa") {
      loadStoragePath();
    } else if (step === "user-2fa") {
      setPendingTotpUser(null);
      setStep("users");
    }
  }

  // ── Storage ─────────────────────────────────────────────────────────────

  async function loadStoragePath() {
    setLoading(true);
    setStorageConfirmed(false);
    try {
      const data = await api.admin.getStorage();
      setStoragePath(data.storage_path);
      // Also browse the current storage path to populate the file browser
      await browseDirectory(undefined); // will default to current storage root
      setStep("storage");
    } catch (err: unknown) {
      setError(
        err instanceof Error ? err.message : "Failed to load storage info"
      );
      setStep("storage");
    } finally {
      setLoading(false);
    }
  }

  async function browseDirectory(path?: string) {
    setBrowseLoading(true);
    try {
      const data = await api.admin.browseDirectory(path);
      setBrowsePath(data.current_path);
      setBrowseParent(data.parent_path);
      setBrowseDirs(data.directories);
      setBrowseWritable(data.writable);
      setManualPathInput(data.current_path);
    } catch (err: unknown) {
      setError(
        err instanceof Error ? err.message : "Failed to browse directory"
      );
    } finally {
      setBrowseLoading(false);
    }
  }

  async function handleSelectStoragePath() {
    setError("");
    setLoading(true);
    try {
      const res = await api.admin.updateStorage(browsePath);
      setStoragePath(res.storage_path);
      setStorageConfirmed(true);
    } catch (err: unknown) {
      setError(
        err instanceof Error ? err.message : "Failed to update storage path"
      );
    } finally {
      setLoading(false);
    }
  }

  async function handleManualPathGo() {
    if (!manualPathInput.trim()) return;
    setError("");
    await browseDirectory(manualPathInput.trim());
  }

  // ── Additional user creation ────────────────────────────────────────────

  async function handleCreateUser(e: React.FormEvent) {
    e.preventDefault();
    setError("");

    if (!newUn.length || !newUn.chars) {
      setError(
        "Username must be 3-50 characters: letters, numbers, underscores only."
      );
      return;
    }
    if (!newPw.core) {
      setError(
        "Password must be at least 8 characters with uppercase, lowercase, and a digit."
      );
      return;
    }
    if (newPassword !== newConfirmPassword) {
      setError("Passwords do not match.");
      return;
    }

    setLoading(true);
    try {
      const res = await api.admin.createUser(newUsername, newPassword, newRole);
      const newUser: CreatedUser = {
        user_id: res.user_id,
        username: res.username,
        role: res.role,
      };
      setCreatedUsers((prev) => [...prev, newUser]);
      setNewUsername("");
      setNewPassword("");
      setNewConfirmPassword("");
      setNewRole("user");
      setShowUserForm(false);

      // Offer 2FA info for the newly created user
      setPendingTotpUser(newUser);
      setStep("user-2fa");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to create user");
    } finally {
      setLoading(false);
    }
  }

  // ── Render helpers ──────────────────────────────────────────────────────

  const StepIndicator = () => {
    const steps = [
      { id: "welcome", label: "Welcome" },
      { id: "account", label: "Account" },
      { id: "admin-2fa", label: "2FA" },
      { id: "storage", label: "Storage" },
      { id: "users", label: "Users" },
      { id: "android", label: "Android" },
      { id: "complete", label: "Done" },
    ];
    // Map user-2fa to users for indicator purposes
    const displayStep = step === "user-2fa" ? "users" : step;
    const currentIdx = steps.findIndex((s) => s.id === displayStep);

    return (
      <div className="flex items-center justify-center gap-1 mb-8 flex-wrap">
        {steps.map((s, i) => (
          <div key={s.id} className="flex items-center gap-1">
            <div
              className={`w-7 h-7 rounded-full flex items-center justify-center text-xs font-medium transition-colors ${
                i < currentIdx
                  ? "bg-green-500 text-white"
                  : i === currentIdx
                    ? "bg-blue-600 text-white"
                    : "bg-gray-200 dark:bg-gray-600 text-gray-500 dark:text-gray-400"
              }`}
            >
              {i < currentIdx ? "\u2713" : i + 1}
            </div>
            <span
              className={`text-xs hidden sm:inline ${
                i === currentIdx
                  ? "text-blue-600 font-medium"
                  : "text-gray-400"
              }`}
            >
              {s.label}
            </span>
            {i < steps.length - 1 && (
              <div
                className={`w-4 h-0.5 ${
                  i < currentIdx ? "bg-green-500" : "bg-gray-200 dark:bg-gray-600"
                }`}
              />
            )}
          </div>
        ))}
      </div>
    );
  };

  // ── Render ────────────────────────────────────────────────────────────────

  if (step === "loading") {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gradient-to-br from-blue-50 dark:from-gray-900 to-indigo-100 dark:to-gray-800">
        <ThemeToggle />
        <div className="text-center">
          <div className="w-12 h-12 border-4 border-blue-600 border-t-transparent rounded-full animate-spin mx-auto mb-4" />
          <p className="text-gray-600 dark:text-gray-400">Connecting to server\u2026</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gradient-to-br from-blue-50 dark:from-gray-900 to-indigo-100 dark:to-gray-800 p-4">
      <ThemeToggle />
      <div className="max-w-lg w-full">
        <StepIndicator />

        <div className="bg-white dark:bg-gray-800 rounded-xl shadow-lg p-8">
          {/* ═══════════════ WELCOME ═══════════════════════════════════════ */}
          {step === "welcome" && (
            <div className="text-center">
              <img src="/logo.png" alt="Simple Photos" className="w-24 h-24 mx-auto mb-4" />
              <h1 className="text-3xl font-bold text-gray-900 dark:text-gray-100 mb-2">
                Welcome to Simple Photos
              </h1>
              <p className="text-gray-600 dark:text-gray-400 mb-2">
                Your self-hosted, end-to-end encrypted photo & video library.
              </p>
              <p className="text-gray-500 dark:text-gray-400 text-sm mb-8">
                Let's get you set up. This will only take a minute.
              </p>

              {status && (
                <div className="text-left bg-blue-50 dark:bg-blue-900/30 rounded-lg p-4 mb-6 text-sm">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="w-2 h-2 rounded-full bg-green-500" />
                    <span className="text-gray-700 dark:text-gray-300">
                      Server connected — v{status.version}
                    </span>
                  </div>
                  <p className="text-gray-600 dark:text-gray-400">
                    No users exist yet. You'll create the admin account next.
                  </p>
                </div>
              )}

              {error && (
                <div className="bg-red-50 dark:bg-red-900/30 rounded-lg p-4 mb-6 text-sm text-red-700 dark:text-red-400">
                  {error}
                </div>
              )}

              <button
                onClick={() => setStep("account")}
                disabled={!!error && !status}
                className="w-full bg-blue-600 text-white py-3 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-lg font-medium transition-colors"
              >
                Get Started →
              </button>
            </div>
          )}

          {/* ═══════════════ ADMIN ACCOUNT ═════════════════════════════════ */}
          {step === "account" && (
            <div>
              <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
                Create Admin Account
              </h2>
              <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
                This will be the first account with full admin privileges.
              </p>

              <form onSubmit={handleCreateAccount} className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                    Username
                  </label>
                  <input
                    type="text"
                    value={username}
                    onChange={(e) => setUsername(e.target.value)}
                    className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                    required
                    minLength={3}
                    maxLength={50}
                    autoComplete="username"
                    autoFocus
                    placeholder="e.g. admin"
                  />
                  {username.length > 0 && (
                    <ul className="text-xs mt-1.5 space-y-0.5">
                      <li>
                        <Checkmark ok={un.length} /> 3–50 characters
                      </li>
                      <li>
                        <Checkmark ok={un.chars} /> Letters, numbers, underscores
                        only
                      </li>
                    </ul>
                  )}
                </div>

                <PasswordField
                  value={password}
                  onChange={setPassword}
                  pwData={pw}
                />

                <ConfirmPasswordField
                  value={confirmPassword}
                  onChange={setConfirmPassword}
                  password={password}
                />

                {error && (
                  <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg">
                    {error}
                  </div>
                )}

                <div className="flex gap-3 pt-2">
                  <button
                    type="button"
                    onClick={() => {
                      setStep("welcome");
                      setError("");
                    }}
                    className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors"
                  >
                    ← Back
                  </button>
                  <button
                    type="submit"
                    disabled={loading || !pw.core || !un.length || !un.chars}
                    className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
                  >
                    {loading ? "Creating account\u2026" : "Create Account →"}
                  </button>
                </div>
              </form>
            </div>
          )}

          {/* ═══════════════ 2FA SETUP (admin or user) ════════════════════ */}
          {(step === "admin-2fa" || step === "user-2fa") && (
            <div>
              <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
                Two-Factor Authentication
              </h2>
              <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
                {step === "admin-2fa"
                  ? "Secure your admin account with 2FA. Highly recommended."
                  : `Set up 2FA for ${pendingTotpUser?.username ?? "the new user"}. Each user can also do this later in Settings.`}
              </p>

              {!totpData && !totpConfirmed && (
                <div className="text-center space-y-4">
                  <div className="bg-blue-50 dark:bg-blue-900/30 rounded-lg p-4 text-sm text-blue-800 dark:text-blue-300">
                    <p>
                      Two-factor authentication adds an extra layer of security.
                      You'll need an authenticator app like Google Authenticator,
                      Authy, or 1Password.
                    </p>
                  </div>
                  {step === "user-2fa" && (
                    <div className="bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-800 dark:text-amber-300">
                      <strong>Note:</strong> 2FA can only be set up while logged
                      in as that user. The user can enable it themselves in Settings
                      after their first login.
                    </div>
                  )}
                  <div className="flex gap-3">
                    <button
                      onClick={skipTotpStep}
                      className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors"
                    >
                      Skip for now →
                    </button>
                    {step === "admin-2fa" && (
                      <button
                        onClick={startTotpSetup}
                        disabled={loading}
                        className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
                      >
                        {loading ? "Setting up\u2026" : "Enable 2FA →"}
                      </button>
                    )}
                  </div>
                </div>
              )}

              {totpData && !totpConfirmed && (
                <div className="space-y-4">
                  <div className="flex justify-center">
                    <div className="bg-white dark:bg-gray-800 p-4 rounded-lg border-2 border-gray-200 dark:border-gray-700">
                      <img
                        src={`https://api.qrserver.com/v1/create-qr-code/?size=200x200&data=${encodeURIComponent(totpData.otpauth_uri)}`}
                        alt="TOTP QR Code"
                        width={200}
                        height={200}
                        className="rounded"
                      />
                    </div>
                  </div>
                  <details className="text-xs text-gray-500 dark:text-gray-400">
                    <summary className="cursor-pointer hover:text-gray-700 dark:hover:text-gray-300 dark:text-gray-300">
                      Can't scan? Enter manually
                    </summary>
                    <code className="block mt-2 bg-gray-50 dark:bg-gray-900 p-2 rounded break-all font-mono">
                      {(() => {
                        try {
                          const url = new URL(totpData.otpauth_uri);
                          return url.searchParams.get("secret") ?? totpData.otpauth_uri;
                        } catch {
                          return totpData.otpauth_uri;
                        }
                      })()}
                    </code>
                  </details>
                  <p className="text-center text-sm text-gray-600 dark:text-gray-400">
                    Scan this QR code with your authenticator app, then enter the
                    6-digit code below.
                  </p>
                  <form onSubmit={confirmTotp} className="space-y-3">
                    <input
                      type="text"
                      value={totpCode}
                      onChange={(e) => setTotpCode(e.target.value)}
                      className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 text-center text-lg tracking-widest focus:outline-none focus:ring-2 focus:ring-blue-500"
                      placeholder="000000"
                      maxLength={6}
                      pattern="\d{6}"
                      autoFocus
                      required
                    />
                    {error && (
                      <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg">
                        {error}
                      </div>
                    )}
                    <button
                      type="submit"
                      disabled={loading || totpCode.length !== 6}
                      className="w-full bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium"
                    >
                      {loading ? "Verifying\u2026" : "Verify Code"}
                    </button>
                  </form>
                </div>
              )}

              {totpConfirmed && (
                <div className="space-y-4">
                  <div className="bg-green-50 dark:bg-green-900/30 rounded-lg p-4 text-center">
                    <span className="text-green-600 dark:text-green-400 text-2xl">{"\u2713"}</span>
                    <p className="text-green-800 dark:text-green-300 font-medium mt-1">
                      2FA Enabled!
                    </p>
                  </div>

                  {backupCodes.length > 0 && (
                    <div>
                      <p className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                        Save these backup codes somewhere safe. Each can be used
                        once if you lose your authenticator:
                      </p>
                      <div className="bg-gray-50 dark:bg-gray-900 rounded-lg p-4 font-mono text-sm grid grid-cols-2 gap-1">
                        {backupCodes.map((code, i) => (
                          <div key={i} className="text-gray-700 dark:text-gray-300">
                            {code}
                          </div>
                        ))}
                      </div>
                      <button
                        onClick={() => {
                          navigator.clipboard.writeText(
                            backupCodes.join("\n")
                          );
                        }}
                        className="mt-2 text-blue-600 text-sm hover:underline"
                      >
                        Copy all codes
                      </button>
                    </div>
                  )}

                  <button
                    onClick={finishTotpStep}
                    className="w-full bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 text-sm font-medium transition-colors"
                  >
                    Continue →
                  </button>
                </div>
              )}
            </div>
          )}

          {/* ═══════════════ STORAGE LOCATION ══════════════════════════════ */}
          {step === "storage" && (
            <div>
              <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
                Photo Storage Location
              </h2>
              <p className="text-gray-500 dark:text-gray-400 text-sm mb-4">
                Choose where your encrypted photos and videos will be stored.
                You can use a local folder, mounted network share, or external
                drive.
              </p>

              {/* Current / selected path indicator */}
              <div className="bg-gray-50 dark:bg-gray-900 rounded-lg p-3 mb-4">
                <div className="flex items-center justify-between">
                  <div>
                    <span className="text-xs font-medium text-gray-500 dark:text-gray-400 block mb-0.5">
                      {storageConfirmed ? "Selected path" : "Current path"}
                    </span>
                    <span className="font-mono text-sm text-gray-800 dark:text-gray-200 break-all">
                      {storagePath || browsePath || "Loading\u2026"}
                    </span>
                  </div>
                  {storageConfirmed && (
                    <span className="text-green-600 dark:text-green-400 text-sm font-medium flex items-center gap-1">
                      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                      </svg>
                      Saved
                    </span>
                  )}
                </div>
              </div>

              {/* Directory browser */}
              <div className="border border-gray-200 dark:border-gray-700 rounded-lg mb-4 overflow-hidden">
                {/* Breadcrumb / current browse path */}
                <div className="bg-gray-100 dark:bg-gray-700 border-b border-gray-200 dark:border-gray-700 px-3 py-2 flex items-center gap-2">
                  <svg className="w-4 h-4 text-gray-500 dark:text-gray-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z" />
                  </svg>
                  <span className="font-mono text-xs text-gray-700 dark:text-gray-300 truncate flex-1">
                    {browsePath}
                  </span>
                  {browseLoading && (
                    <div className="w-4 h-4 border-2 border-blue-500 dark:border-blue-400 border-t-transparent rounded-full animate-spin" />
                  )}
                </div>

                {/* Directory list */}
                <div className="max-h-60 overflow-y-auto">
                  {/* Up / parent directory */}
                  {browseParent && (
                    <button
                      type="button"
                      onClick={() => browseDirectory(browseParent)}
                      disabled={browseLoading}
                      className="w-full text-left px-3 py-2 hover:bg-blue-50 dark:hover:bg-blue-900/30 dark:bg-blue-900/30 flex items-center gap-2 text-sm border-b border-gray-100 dark:border-gray-700 transition-colors disabled:opacity-50"
                    >
                      <svg className="w-4 h-4 text-blue-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M9 15L3 9m0 0l6-6M3 9h12a6 6 0 010 12h-3" />
                      </svg>
                      <span className="text-blue-600 font-medium">..</span>
                      <span className="text-gray-400 text-xs ml-auto">
                        Parent folder
                      </span>
                    </button>
                  )}

                  {/* Subdirectories */}
                  {browseDirs.length === 0 && !browseLoading && (
                    <div className="px-3 py-6 text-center text-gray-400 text-sm">
                      No subdirectories
                    </div>
                  )}
                  {browseDirs.map((dir) => (
                    <button
                      key={dir.path}
                      type="button"
                      onClick={() => browseDirectory(dir.path)}
                      disabled={browseLoading}
                      className="w-full text-left px-3 py-2 hover:bg-blue-50 dark:hover:bg-blue-900/30 dark:bg-blue-900/30 flex items-center gap-2 text-sm border-b border-gray-100 dark:border-gray-700 last:border-b-0 transition-colors disabled:opacity-50"
                    >
                      <svg className="w-4 h-4 text-yellow-500 shrink-0" fill="currentColor" viewBox="0 0 20 20">
                        <path d="M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
                      </svg>
                      <span className="text-gray-800 dark:text-gray-200 truncate">{dir.name}</span>
                    </button>
                  ))}
                </div>
              </div>

              {/* Writable indicator */}
              <div className="flex items-center gap-2 mb-3 text-xs">
                <span className={`w-2 h-2 rounded-full ${browseWritable ? "bg-green-500" : "bg-red-500"}`} />
                <span className={browseWritable ? "text-green-700 dark:text-green-400" : "text-red-700 dark:text-red-400"}>
                  {browseWritable
                    ? "This directory is writable"
                    : "This directory is not writable — choose a different location"}
                </span>
              </div>

              {/* Manual path entry toggle */}
              <div className="mb-4">
                <button
                  type="button"
                  onClick={() => setShowManualInput((v) => !v)}
                  className="text-xs text-blue-600 hover:text-blue-800 dark:hover:text-blue-300 dark:text-blue-300 flex items-center gap-1"
                >
                  <svg className={`w-3 h-3 transition-transform ${showManualInput ? "rotate-90" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
                  </svg>
                  Enter path manually
                </button>
                {showManualInput && (
                  <div className="flex gap-2 mt-2">
                    <input
                      type="text"
                      value={manualPathInput}
                      onChange={(e) => setManualPathInput(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") handleManualPathGo();
                      }}
                      className="flex-1 border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2 font-mono text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                      placeholder="/path/to/storage"
                    />
                    <button
                      type="button"
                      onClick={handleManualPathGo}
                      disabled={browseLoading}
                      className="px-4 py-2 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors disabled:opacity-50"
                    >
                      Go
                    </button>
                  </div>
                )}
              </div>

              {error && (
                <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg mb-4">
                  {error}
                </div>
              )}

              {/* Action buttons */}
              <div className="flex gap-3">
                <button
                  type="button"
                  onClick={handleSelectStoragePath}
                  disabled={loading || !browseWritable || browsePath === storagePath}
                  className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
                >
                  {loading
                    ? "Saving\u2026"
                    : browsePath === storagePath
                      ? "Current location selected"
                      : "Use This Location"}
                </button>
              </div>

              {/* Continue button — always visible after confirming or if using default */}
              <button
                onClick={() => {
                  setError("");
                  setStep("users");
                }}
                className="w-full mt-3 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors"
              >
                {storageConfirmed ? "Continue →" : "Keep Default & Continue →"}
              </button>
            </div>
          )}

          {/* ═══════════════ ADDITIONAL USERS ══════════════════════════════ */}
          {step === "users" && (
            <div>
              <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
                Additional Users
              </h2>
              <p className="text-gray-500 dark:text-gray-400 text-sm mb-4">
                Create accounts for family members or other users. You can always
                add more later in Settings.
              </p>

              {/* List of created users */}
              {createdUsers.length > 0 && (
                <div className="mb-4 space-y-2">
                  {createdUsers.map((u) => (
                    <div
                      key={u.user_id}
                      className="flex items-center justify-between bg-green-50 dark:bg-green-900/30 rounded-lg px-4 py-2.5"
                    >
                      <div>
                        <span className="font-medium text-gray-800 dark:text-gray-200">
                          {u.username}
                        </span>
                        <span
                          className={`ml-2 text-xs px-2 py-0.5 rounded-full ${
                            u.role === "admin"
                              ? "bg-purple-100 dark:bg-purple-900/40 text-purple-700 dark:text-purple-300"
                              : "bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300"
                          }`}
                        >
                          {u.role}
                        </span>
                      </div>
                      <span className="text-green-600 dark:text-green-400 text-sm">
                        {"\u2713"} Created
                      </span>
                    </div>
                  ))}
                </div>
              )}

              {/* User creation form */}
              {showUserForm ? (
                <form
                  onSubmit={handleCreateUser}
                  className="space-y-4 border rounded-lg p-4"
                >
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Username
                    </label>
                    <input
                      type="text"
                      value={newUsername}
                      onChange={(e) => setNewUsername(e.target.value)}
                      className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                      required
                      minLength={3}
                      maxLength={50}
                      autoFocus
                      placeholder="Username"
                    />
                    {newUsername.length > 0 && (
                      <ul className="text-xs mt-1 space-y-0.5">
                        <li>
                          <Checkmark ok={newUn.length} /> 3–50 characters
                        </li>
                        <li>
                          <Checkmark ok={newUn.chars} /> Letters, numbers,
                          underscores
                        </li>
                      </ul>
                    )}
                  </div>

                  <PasswordField
                    value={newPassword}
                    onChange={setNewPassword}
                    pwData={newPw}
                  />

                  <ConfirmPasswordField
                    value={newConfirmPassword}
                    onChange={setNewConfirmPassword}
                    password={newPassword}
                  />

                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Role
                    </label>
                    <div className="flex gap-3">
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input
                          type="radio"
                          name="role"
                          value="user"
                          checked={newRole === "user"}
                          onChange={() => setNewRole("user")}
                          className="text-blue-600"
                        />
                        <span className="text-sm">
                          <span className="font-medium">User</span>
                          <span className="text-gray-500 dark:text-gray-400">
                            {" "}
                            — Upload & view own photos
                          </span>
                        </span>
                      </label>
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input
                          type="radio"
                          name="role"
                          value="admin"
                          checked={newRole === "admin"}
                          onChange={() => setNewRole("admin")}
                          className="text-blue-600"
                        />
                        <span className="text-sm">
                          <span className="font-medium">Admin</span>
                          <span className="text-gray-500 dark:text-gray-400">
                            {" "}
                            — Full control
                          </span>
                        </span>
                      </label>
                    </div>
                  </div>

                  {error && (
                    <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg">
                      {error}
                    </div>
                  )}

                  <div className="flex gap-3">
                    <button
                      type="button"
                      onClick={() => {
                        setShowUserForm(false);
                        setError("");
                      }}
                      className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium"
                    >
                      Cancel
                    </button>
                    <button
                      type="submit"
                      disabled={
                        loading ||
                        !newPw.core ||
                        !newUn.length ||
                        !newUn.chars
                      }
                      className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium"
                    >
                      {loading ? "Creating\u2026" : "Create User"}
                    </button>
                  </div>
                </form>
              ) : (
                <button
                  onClick={() => {
                    setShowUserForm(true);
                    setError("");
                  }}
                  className="w-full border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg py-3 text-gray-500 dark:text-gray-400 hover:border-blue-400 dark:hover:border-blue-500 dark:border-blue-400 hover:text-blue-600 dark:hover:text-blue-400 transition-colors text-sm font-medium"
                >
                  + Add a user
                </button>
              )}

              <div className="mt-6">
                <button
                  onClick={() => {
                    setError("");
                    setStep("android");
                  }}
                  className="w-full bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 text-sm font-medium transition-colors"
                >
                  {createdUsers.length > 0
                    ? "Continue →"
                    : "Skip — No additional users →"}
                </button>
              </div>
            </div>
          )}

          {/* ═══════════════ ANDROID SETUP ═════════════════════════════════ */}
          {step === "android" && (
            <div>
              <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
                Set Up Android App
              </h2>
              <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
                Install the Simple Photos app on your Android device for
                automatic photo backup.
              </p>

              <div className="space-y-4">
                {/* Download button */}
                <a
                  href="/api/downloads/android"
                  className="flex items-center justify-center gap-3 w-full bg-green-600 text-white py-3 rounded-lg hover:bg-green-700 text-sm font-medium transition-colors"
                >
                  <svg
                    className="w-6 h-6"
                    viewBox="0 0 24 24"
                    fill="currentColor"
                  >
                    <path d="M17.523 2.23a.75.75 0 00-1.06 0l-1.8 1.8A8.96 8.96 0 0012 3.5a8.96 8.96 0 00-2.663.53l-1.8-1.8a.75.75 0 10-1.06 1.06l1.56 1.56A8.981 8.981 0 003 12.5v.5h18v-.5a8.981 8.981 0 00-5.037-7.21l1.56-1.56a.75.75 0 000-1.06zM10 10.5a1 1 0 11-2 0 1 1 0 012 0zm6 0a1 1 0 11-2 0 1 1 0 012 0zM3 14.5h18v1a7 7 0 01-7 7h-4a7 7 0 01-7-7v-1z" />
                  </svg>
                  Download APK
                </a>

                {/* Sideloading instructions */}
                <div className="bg-gray-50 dark:bg-gray-900 rounded-lg p-4">
                  <h3 className="font-medium text-gray-800 dark:text-gray-200 text-sm mb-3">
                    How to install (sideload):
                  </h3>
                  <ol className="text-sm text-gray-600 dark:text-gray-400 space-y-3">
                    <li className="flex gap-3">
                      <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                        1
                      </span>
                      <div>
                        <p className="font-medium text-gray-700 dark:text-gray-300">
                          Download the APK
                        </p>
                        <p className="text-xs text-gray-500 dark:text-gray-400">
                          Click the button above or transfer the APK to your
                          phone via USB/email.
                        </p>
                      </div>
                    </li>
                    <li className="flex gap-3">
                      <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                        2
                      </span>
                      <div>
                        <p className="font-medium text-gray-700 dark:text-gray-300">
                          Enable "Install unknown apps"
                        </p>
                        <p className="text-xs text-gray-500 dark:text-gray-400">
                          Go to{" "}
                          <strong>
                            Settings → Apps → Special access → Install unknown
                            apps
                          </strong>{" "}
                          and enable it for your file manager or browser.
                        </p>
                      </div>
                    </li>
                    <li className="flex gap-3">
                      <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                        3
                      </span>
                      <div>
                        <p className="font-medium text-gray-700 dark:text-gray-300">
                          Open the APK
                        </p>
                        <p className="text-xs text-gray-500 dark:text-gray-400">
                          Tap the downloaded APK file and confirm the
                          installation prompt.
                        </p>
                      </div>
                    </li>
                    <li className="flex gap-3">
                      <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                        4
                      </span>
                      <div>
                        <p className="font-medium text-gray-700 dark:text-gray-300">
                          Connect to your server
                        </p>
                        <p className="text-xs text-gray-500 dark:text-gray-400">
                          Open the app, enter your server URL:
                        </p>
                        <code className="block mt-1 bg-gray-200 dark:bg-gray-600 px-2 py-1 rounded text-xs text-gray-800 dark:text-gray-200 break-all">
                          {window.location.origin}
                        </code>
                      </div>
                    </li>
                    <li className="flex gap-3">
                      <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                        5
                      </span>
                      <div>
                        <p className="font-medium text-gray-700 dark:text-gray-300">
                          Sign in & grant permissions
                        </p>
                        <p className="text-xs text-gray-500 dark:text-gray-400">
                          Log in with your account and allow the app to access
                          your photos and videos for automatic encrypted backup.
                        </p>
                      </div>
                    </li>
                  </ol>
                </div>

                <div className="bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-800 dark:text-amber-300">
                  <strong>Note:</strong> Keep "Install unknown apps" disabled
                  after installation for security. You can always re-enable it
                  when updating the app.
                </div>
              </div>

              <div className="flex gap-3 mt-6">
                <button
                  onClick={() => {
                    setError("");
                    setStep("users");
                  }}
                  className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors"
                >
                  ← Back
                </button>
                <button
                  onClick={() => {
                    setError("");
                    setStep("complete");
                  }}
                  className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 text-sm font-medium transition-colors"
                >
                  Continue →
                </button>
              </div>
            </div>
          )}

          {/* ═══════════════ COMPLETE ══════════════════════════════════════ */}
          {step === "complete" && (
            <div className="text-center">
              <img src="/logo.png" alt="Simple Photos" className="w-20 h-20 mx-auto mb-4" />
              <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-2">
                You're All Set!
              </h2>
              <p className="text-gray-600 dark:text-gray-400 mb-6">
                Your Simple Photos server is ready. Start uploading your
                encrypted photos and videos.
              </p>

              <div className="bg-green-50 dark:bg-green-900/30 rounded-lg p-4 mb-6 text-sm text-left space-y-2">
                <div className="flex items-center gap-2">
                  <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
                  <span className="text-gray-700 dark:text-gray-300">Admin account created</span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
                  <span className="text-gray-700 dark:text-gray-300">
                    Encryption key derived
                  </span>
                </div>
                {createdUsers.length > 0 && (
                  <div className="flex items-center gap-2">
                    <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
                    <span className="text-gray-700 dark:text-gray-300">
                      {createdUsers.length} additional user
                      {createdUsers.length > 1 ? "s" : ""} created
                    </span>
                  </div>
                )}
                <div className="flex items-center gap-2">
                  <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
                  <span className="text-gray-700 dark:text-gray-300">Ready to upload</span>
                </div>
              </div>

              <div className="space-y-3">
                <button
                  onClick={() => navigate("/gallery")}
                  className="w-full bg-blue-600 text-white py-3 rounded-lg hover:bg-blue-700 text-lg font-medium transition-colors"
                >
                  Go to Gallery →
                </button>
                <p className="text-gray-400 text-xs">
                  You can manage users, 2FA, and storage in Settings.
                </p>
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <p className="text-center text-gray-400 text-xs mt-6">
          Simple Photos v{status?.version ?? "0.1.0"} — End-to-end encrypted
        </p>
      </div>
    </div>
  );
}
