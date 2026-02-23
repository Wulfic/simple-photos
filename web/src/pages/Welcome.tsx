import { useState, useEffect, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { api, TotpSetupResponse } from "../api/client";
import { useAuthStore } from "../store/auth";
import { deriveKey } from "../crypto/crypto";
import ThemeToggle from "../components/ThemeToggle";
import { checkPasswordStrength, checkUsername } from "../utils/validation";

import type { WizardStep, SetupStatus, CreatedUser, ServerRole } from "./welcome/types";
import StepIndicator from "./welcome/StepIndicator";
import WelcomeStep from "./welcome/WelcomeStep";
import ServerRoleStep from "./welcome/ServerRoleStep";
import PairStep from "./welcome/PairStep";
import AccountStep from "./welcome/AccountStep";
import TwoFactorStep from "./welcome/TwoFactorStep";
import ServerConfigStep from "./welcome/ServerConfigStep";
import EncryptionStep from "./welcome/EncryptionStep";
import SslStep from "./welcome/SslStep";
import UsersStep from "./welcome/UsersStep";
import AndroidStep from "./welcome/AndroidStep";
import CompleteStep from "./welcome/CompleteStep";
// BackupStep removed from primary flow — server role is now handled by ServerRoleStep

export default function Welcome() {
  const [step, setStep] = useState<WizardStep>("loading");
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  // ── Server role (primary vs backup) ─────────────────────────────────
  const [serverRole, setServerRole] = useState<ServerRole>(null);
  const [mainServerUrl, setMainServerUrl] = useState("");
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

  // ── Encryption mode ─────────────────────────────────────────────────────
  const [encryptionMode, setEncryptionMode] = useState<"plain" | "encrypted">("plain");

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
      setServerPort(portData.port);
      setOriginalPort(portData.port);
      setPortInput(String(portData.port));
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

  async function handlePaired(data: {
    access_token: string;
    refresh_token: string;
    username: string;
    main_server_url: string;
  }) {
    // Store tokens — the pair endpoint already created a local admin
    setTokens(data.access_token, data.refresh_token);
    storeSetUsername(data.username);
    setUsername(data.username);
    setMainServerUrl(data.main_server_url);

    // Derive the encryption key from the admin credentials
    await deriveKey("", data.username);   // password already used server-side

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
        <StepIndicator step={step} serverRole={serverRole} />

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
            />
          )}

          {step === "ssl" && (
            <SslStep
              setStep={setStep}
              setError={setError}
              error={error}
            />
          )}

          {step === "encryption" && (
            <EncryptionStep
              encryptionMode={encryptionMode}
              setEncryptionMode={setEncryptionMode}
              setStep={setStep}
              setError={setError}
              loading={loading}
              setLoading={setLoading}
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
              encryptionMode={encryptionMode}
              createdUsers={createdUsers}
              serverPort={serverPort}
              originalPort={originalPort}
              serverRole={serverRole}
              mainServerUrl={mainServerUrl}
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
