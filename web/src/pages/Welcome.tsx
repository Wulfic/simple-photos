/**
 * First-run setup wizard — multi-step onboarding flow for new server instances.
 *
 * Steps branch based on server role (primary vs. backup) and install type
 * (fresh vs. restore). Persists the current step to sessionStorage so
 * accidental refreshes don't restart the wizard.
 */
import { useState, useEffect, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { api, TotpSetupResponse } from "../api/client";
import { useAuthStore } from "../store/auth";
import { deriveKey } from "../crypto/crypto";
import ThemeToggle from "../components/ThemeToggle";
import { checkPasswordStrength, checkUsername } from "../utils/validation";

import type { WizardStep, SetupStatus, CreatedUser, ServerRole, InstallType, RestoreSource } from "./welcome/types";
import StepIndicator from "./welcome/StepIndicator";
import WelcomeStep from "./welcome/WelcomeStep";
import ServerRoleStep from "./welcome/ServerRoleStep";
import InstallTypeStep from "./welcome/InstallTypeStep";
import RestoreStep from "./welcome/RestoreStep";
import PairStep from "./welcome/PairStep";
import AccountStep from "./welcome/AccountStep";
import TwoFactorStep from "./welcome/TwoFactorStep";
import ServerConfigStep from "./welcome/ServerConfigStep";
import SslStep from "./welcome/SslStep";
import UsersStep from "./welcome/UsersStep";
import AndroidStep from "./welcome/AndroidStep";
import CompleteStep from "./welcome/CompleteStep";
// BackupStep removed from primary flow — server role is now handled by ServerRoleStep

// ── Session persistence helpers ──────────────────────────────────────────────
const WIZARD_STEP_KEY = "sp_wizard_step";
const WIZARD_ACTIVE_KEY = "sp_wizard_active";
const WIZARD_ROLE_KEY = "sp_wizard_server_role";
const WIZARD_INSTALL_KEY = "sp_wizard_install_type";

function saveWizardStep(step: WizardStep) {
  try {
    sessionStorage.setItem(WIZARD_STEP_KEY, step);
    sessionStorage.setItem(WIZARD_ACTIVE_KEY, "1");
  } catch { /* quota / private mode — non-critical */ }
}

function saveWizardChoice(role: ServerRole, install: InstallType) {
  try {
    if (role) sessionStorage.setItem(WIZARD_ROLE_KEY, role);
    if (install) sessionStorage.setItem(WIZARD_INSTALL_KEY, install);
  } catch { /* non-critical */ }
}

function loadWizardStep(): WizardStep | null {
  try {
    const active = sessionStorage.getItem(WIZARD_ACTIVE_KEY);
    if (active !== "1") return null;
    return (sessionStorage.getItem(WIZARD_STEP_KEY) as WizardStep) || null;
  } catch { return null; }
}

function loadWizardChoices(): { role: ServerRole; install: InstallType } {
  try {
    const role = sessionStorage.getItem(WIZARD_ROLE_KEY) as ServerRole;
    const install = sessionStorage.getItem(WIZARD_INSTALL_KEY) as InstallType;
    return { role: role || null, install: install || null };
  } catch { return { role: null, install: null }; }
}

function clearWizardStep() {
  try {
    sessionStorage.removeItem(WIZARD_STEP_KEY);
    sessionStorage.removeItem(WIZARD_ACTIVE_KEY);
    sessionStorage.removeItem(WIZARD_ROLE_KEY);
    sessionStorage.removeItem(WIZARD_INSTALL_KEY);
  } catch { /* ignore */ }
}

export default function Welcome() {
  const [step, setStepRaw] = useState<WizardStep>("loading");
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  // Wrap setStep to persist to sessionStorage
  function setStep(next: WizardStep) {
    setStepRaw(next);
    if (next !== "loading") saveWizardStep(next);
  }

  // ── Server role (primary vs backup) ─────────────────────────────────
  const [serverRole, setServerRoleRaw] = useState<ServerRole>(null);
  const [installType, setInstallTypeRaw] = useState<InstallType>(null);

  // Wrap setters to persist choices to sessionStorage
  function setServerRole(role: ServerRole) {
    setServerRoleRaw(role);
    saveWizardChoice(role, installType);
  }
  function setInstallType(type: InstallType) {
    setInstallTypeRaw(type);
    saveWizardChoice(serverRole, type);
  }
  const [mainServerUrl, setMainServerUrl] = useState("");
  const [restoreSource, setRestoreSource] = useState<RestoreSource | null>(null);
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
  const [storageConfirmed, setStorageConfirmed] = useState(false);
  const [pendingStoragePath, setPendingStoragePath] = useState("");

  // ── Server port ─────────────────────────────────────────────────────────
  const [serverPort, setServerPort] = useState<number>(0);
  const [originalPort, setOriginalPort] = useState<number>(0);
  const [portInput, setPortInput] = useState("");
  const [portSaved, setPortSaved] = useState(false);


  // ── Additional users ────────────────────────────────────────────────────
  const [createdUsers, setCreatedUsers] = useState<CreatedUser[]>([]);
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [newConfirmPassword, setNewConfirmPassword] = useState("");
  const [newRole, setNewRole] = useState<"user" | "admin">("user");
  const [showUserForm, setShowUserForm] = useState(false);
  const [pendingTotpUser, setPendingTotpUser] = useState<CreatedUser | null>(null);

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
        // Check if the wizard was still in progress (page refresh mid-setup)
        const savedStep = loadWizardStep();
        if (savedStep && savedStep !== "loading" && savedStep !== "complete") {
          // Restore choices so the step indicator and back-navigation work
          const { role, install } = loadWizardChoices();
          if (role) setServerRoleRaw(role);
          if (install) setInstallTypeRaw(install);
          // Resume the wizard where the user left off
          setStepRaw(savedStep);
        } else {
          // Wizard truly finished — go to login
          clearWizardStep();
          navigate("/login", { replace: true });
        }
      } else {
        // Also restore choices for the case where setup_complete is still false
        const savedStep = loadWizardStep();
        if (savedStep && savedStep !== "loading" && savedStep !== "complete") {
          const { role, install } = loadWizardChoices();
          if (role) setServerRoleRaw(role);
          if (install) setInstallTypeRaw(install);
          setStepRaw(savedStep);
        } else {
          setStep("welcome");
        }
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
      setError("Username must be 3-50 characters: letters, numbers, underscores only.");
      return;
    }
    if (!pw.core) {
      setError("Password must be at least 8 characters with uppercase, lowercase, and a digit.");
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

  // ── 2FA setup ───────────────────────────────────────────────────────────

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
      setError(err instanceof Error ? err.message : "Failed to start 2FA setup");
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
      const [storageData, portData] = await Promise.all([
        api.admin.getStorage(),
        api.admin.getPort(),
      ]);
      setStoragePath(storageData.storage_path);
      setPendingStoragePath(storageData.storage_path);
      // Use the external port (from the Host header) when available — this
      // is the port the user actually reaches the server on, which may
      // differ from the internal container port in Docker setups.
      const effectivePort = portData.external_port ?? portData.port;
      setServerPort(effectivePort);
      setOriginalPort(effectivePort);
      // During setup, offer the external port the server is already on, or
      // the first available port starting at 8080 if different.
      const defaultInput =
        portData.suggested_port != null
          ? String(portData.suggested_port)
          : String(effectivePort);
      setPortInput(defaultInput);
      setPortSaved(false);
      setStep("storage");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load storage info");
      setStep("storage");
    } finally {
      setLoading(false);
    }
  }

  async function handleSavePort() {
    const port = parseInt(portInput, 10);
    if (isNaN(port) || port < 1024 || port > 65535) {
      setError("Port must be between 1024 and 65535");
      return;
    }
    setLoading(true);
    setError("");
    try {
      await api.admin.updatePort(port);
      setServerPort(port);
      setPortSaved(true);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to update port");
    } finally {
      setLoading(false);
    }
  }

  async function handleSelectStoragePath() {
    if (!pendingStoragePath.trim()) return;
    setError("");
    setLoading(true);
    try {
      const res = await api.admin.updateStorage(pendingStoragePath.trim());
      setStoragePath(res.storage_path);
      setStorageConfirmed(true);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to update storage path");
    } finally {
      setLoading(false);
    }
  }

  // ── Backup pairing ─────────────────────────────────────────────────────

  // ── Restore from backup (primary) ───────────────────────────────────────
  // After verifying the backup server, create a temporary admin account so
  // the server considers setup complete, then skip straight to server config.
  // The recovery engine will import all real user accounts from the backup.

  async function handleRestoreInit(source: RestoreSource) {
    setError("");
    setLoading(true);
    try {
      // Create a temporary admin account — the setup/init endpoint requires
      // this before any authenticated API calls work.
      const tempUser = "restore_admin";
      const arr = new Uint8Array(8);
      crypto.getRandomValues(arr);
      const tempPass = "Restore_Temp_" + Array.from(arr, (b) => b.toString(16).padStart(2, "0")).join("") + "1!";

      await fetch("/api/setup/init", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username: tempUser, password: tempPass }),
      }).then(async (res) => {
        if (!res.ok) {
          const err = await res.json().catch(() => ({ error: "Setup init failed" }));
          throw new Error(err.error || `HTTP ${res.status}`);
        }
      });

      // Log in to get an auth token
      const loginRes = await api.auth.login(tempUser, tempPass);
      if (!loginRes.access_token || !loginRes.refresh_token) {
        throw new Error("Unexpected login response");
      }
      setTokens(loginRes.access_token, loginRes.refresh_token);
      storeSetUsername(tempUser);
      setUsername(tempUser);
      setPassword(tempPass);

      // Derive encryption key (will be replaced when real users are restored)
      try {
        await deriveKey(tempPass, tempUser);
      } catch (err) {
        console.warn("Failed to derive encryption key:", err);
      }

      // Skip account, 2FA, and users — go straight to server config
      loadStoragePath();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to initialize restore");
    } finally {
      setLoading(false);
    }
  }

  async function handlePaired(data: {
    access_token: string;
    refresh_token: string;
    username: string;
    main_server_url: string;
    password?: string;
  }) {
    // Store tokens — the pair endpoint already created a local admin
    setTokens(data.access_token, data.refresh_token);
    storeSetUsername(data.username);
    setUsername(data.username);
    setMainServerUrl(data.main_server_url);

    // Derive the encryption key from the admin credentials
    try {
      await deriveKey(data.password || "", data.username);
    } catch (err) {
      console.warn("Failed to derive encryption key:", err);
    }

    // Go directly to storage config (skip account + 2FA for backup servers)
    loadStoragePath();
  }

  // ── Additional user creation ────────────────────────────────────────────

  async function handleCreateUser(e: React.FormEvent) {
    e.preventDefault();
    setError("");

    if (!newUn.length || !newUn.chars) {
      setError("Username must be 3-50 characters: letters, numbers, underscores only.");
      return;
    }
    if (!newPw.core) {
      setError("Password must be at least 8 characters with uppercase, lowercase, and a digit.");
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

      setPendingTotpUser(newUser);
      setStep("user-2fa");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to create user");
    } finally {
      setLoading(false);
    }
  }

  // ── Render ────────────────────────────────────────────────────────────────

  if (step === "loading") {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gradient-to-br from-blue-50 dark:from-gray-900 to-indigo-100 dark:to-gray-800">
        <ThemeToggle />
        <div className="text-center">
          <div className="w-12 h-12 border-4 border-blue-600 border-t-transparent rounded-full animate-spin mx-auto mb-4" />
          <p className="text-gray-600 dark:text-gray-400">Connecting to server…</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gradient-to-br from-blue-50 dark:from-gray-900 to-indigo-100 dark:to-gray-800 p-4">
      <ThemeToggle />
      <div className="max-w-lg w-full">
        <StepIndicator step={step} serverRole={serverRole} installType={installType} />

        <div className="bg-white dark:bg-gray-800 rounded-xl shadow-lg p-8">
          {step === "welcome" && (
            <WelcomeStep setStep={setStep} status={status} error={error} />
          )}

          {step === "server-role" && (
            <ServerRoleStep
              setStep={setStep}
              setServerRole={setServerRole}
              setError={setError}
            />
          )}

          {step === "install-type" && (
            <InstallTypeStep
              setStep={setStep}
              setInstallType={setInstallType}
              setError={setError}
            />
          )}

          {step === "restore" && (
            <RestoreStep
              setStep={setStep}
              setError={setError}
              error={error}
              onVerified={(source) => {
                setRestoreSource(source);
                setError("");
                // Skip account/2FA/users — restore imports everything from backup
                handleRestoreInit(source);
              }}
            />
          )}

          {step === "pair" && (
            <PairStep
              setStep={setStep}
              setError={setError}
              error={error}
              onPaired={handlePaired}
            />
          )}

          {step === "account" && (
            <AccountStep
              username={username}
              setUsername={setUsername}
              password={password}
              setPassword={setPassword}
              confirmPassword={confirmPassword}
              setConfirmPassword={setConfirmPassword}
              pw={pw}
              un={un}
              loading={loading}
              error={error}
              handleCreateAccount={handleCreateAccount}
              setStep={setStep}
              setError={setError}
            />
          )}

          {(step === "admin-2fa" || step === "user-2fa") && (
            <TwoFactorStep
              totpData={totpData}
              totpCode={totpCode}
              setTotpCode={setTotpCode}
              backupCodes={backupCodes}
              totpConfirmed={totpConfirmed}
              loading={loading}
              error={error}
              startTotpSetup={startTotpSetup}
              confirmTotp={confirmTotp}
              finishTotpStep={finishTotpStep}
              skipTotpStep={skipTotpStep}
              step={step}
              pendingTotpUser={pendingTotpUser}
              setStep={setStep}
              setError={setError}
            />
          )}

          {step === "storage" && (
            <ServerConfigStep
              portInput={portInput}
              setPortInput={setPortInput}
              portSaved={portSaved}
              setPortSaved={setPortSaved}
              serverPort={serverPort}
              originalPort={originalPort}
              handleSavePort={handleSavePort}
              storagePath={storagePath}
              storageConfirmed={storageConfirmed}
              handleSelectStoragePath={handleSelectStoragePath}
              loading={loading}
              error={error}
              setStep={setStep}
              setError={setError}
              setStoragePathDirect={setPendingStoragePath}
              serverRole={serverRole}
              installType={installType}
            />
          )}

          {step === "ssl" && (
            <SslStep
              setStep={setStep}
              setError={setError}
              error={error}
            />
          )}

          {step === "users" && (
            <UsersStep
              createdUsers={createdUsers}
              newUsername={newUsername}
              setNewUsername={setNewUsername}
              newPassword={newPassword}
              setNewPassword={setNewPassword}
              newConfirmPassword={newConfirmPassword}
              setNewConfirmPassword={setNewConfirmPassword}
              newRole={newRole}
              setNewRole={setNewRole}
              showUserForm={showUserForm}
              setShowUserForm={setShowUserForm}
              newPw={newPw}
              newUn={newUn}
              loading={loading}
              error={error}
              handleCreateUser={handleCreateUser}
              setStep={setStep}
              setError={setError}
            />
          )}

          {step === "android" && (
            <AndroidStep setStep={setStep} setError={setError} />
          )}

          {step === "complete" && (
            <CompleteStep
              setStep={setStep}
              setError={setError}
              loading={loading}
              setLoading={setLoading}
              error={error}
              createdUsers={createdUsers}
              serverPort={serverPort}
              originalPort={originalPort}
              serverRole={serverRole}
              mainServerUrl={mainServerUrl}
              installType={installType}
              restoreSource={restoreSource}
            />
          )}
        </div>

        <p className="text-center text-gray-400 text-xs mt-6">
          Simple Photos v{status?.version ?? "0.6.9"} — End-to-end encrypted
        </p>
      </div>
    </div>
  );
}
