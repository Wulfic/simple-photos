import { useState, useEffect } from "react";
import { QRCodeSVG } from "qrcode.react";
import { api } from "../../api/client";
import { useIsAdmin } from "../../hooks/useIsAdmin";
import AppIcon from "../AppIcon";

type ManagedUser = {
  id: string;
  username: string;
  role: string;
  totp_enabled: boolean;
  created_at: string;
};

interface UserManagementProps {
  error: string;
  setError: (e: string) => void;
  success: string;
  setSuccess: (s: string) => void;
}

export default function UserManagement({ setError, setSuccess }: UserManagementProps) {
  const isAdmin = useIsAdmin();

  const [managedUsers, setManagedUsers] = useState<ManagedUser[]>([]);
  const [usersLoaded, setUsersLoaded] = useState(false);
  const [showAddUser, setShowAddUser] = useState(false);
  const [newUsername, setNewUsername] = useState("");
  const [newUserPassword, setNewUserPassword] = useState("");
  const [newUserRole, setNewUserRole] = useState<"user" | "admin">("user");
  const [resetPwUserId, setResetPwUserId] = useState<string | null>(null);
  const [resetPwValue, setResetPwValue] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const [setup2faUserId, setSetup2faUserId] = useState<string | null>(null);
  const [setup2faUri, setSetup2faUri] = useState<string | null>(null);
  const [setup2faBackupCodes, setSetup2faBackupCodes] = useState<string[]>([]);
  const [setup2faCode, setSetup2faCode] = useState("");
  const [setup2faLoading, setSetup2faLoading] = useState(false);

  useEffect(() => {
    loadManagedUsers();
  }, []);

  async function loadManagedUsers() {
    try {
      const users = await api.admin.listUsers();
      setManagedUsers(users);
      setUsersLoaded(true);
    } catch {
      // Not admin — silently skip
    }
  }

  async function handleAddUser(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.admin.createUser(newUsername, newUserPassword, newUserRole);
      setSuccess(`User "${newUsername}" created.`);
      setNewUsername("");
      setNewUserPassword("");
      setNewUserRole("user");
      setShowAddUser(false);
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleDeleteUser(userId: string) {
    setError("");
    try {
      await api.admin.deleteUser(userId);
      setSuccess("User deleted.");
      setConfirmDeleteId(null);
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleChangeRole(userId: string, role: "admin" | "user") {
    setError("");
    try {
      await api.admin.updateUserRole(userId, role);
      setSuccess("Role updated.");
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleResetUserPassword(userId: string) {
    setError("");
    if (!resetPwValue || resetPwValue.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }
    try {
      await api.admin.resetUserPassword(userId, resetPwValue);
      setSuccess("Password reset.");
      setResetPwUserId(null);
      setResetPwValue("");
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleResetUser2fa(userId: string) {
    setError("");
    try {
      await api.admin.resetUser2fa(userId);
      setSuccess("2FA disabled for user.");
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleAdminSetup2fa(userId: string) {
    setError("");
    setSetup2faLoading(true);
    try {
      const res = await api.admin.setupUser2fa(userId);
      setSetup2faUserId(userId);
      setSetup2faUri(res.otpauth_uri);
      setSetup2faBackupCodes(res.backup_codes);
      setSetup2faCode("");
    } catch (err: any) {
      setError(err.message || "Failed to start 2FA setup");
    } finally {
      setSetup2faLoading(false);
    }
  }

  async function handleAdminConfirm2fa() {
    if (!setup2faUserId || !setup2faCode.trim()) return;
    setError("");
    setSetup2faLoading(true);
    try {
      await api.admin.confirmUser2fa(setup2faUserId, setup2faCode.trim());
      setSuccess("2FA enabled for user.");
      setSetup2faUserId(null);
      setSetup2faUri(null);
      setSetup2faBackupCodes([]);
      setSetup2faCode("");
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message || "Invalid TOTP code");
    } finally {
      setSetup2faLoading(false);
    }
  }

  function cancelAdminSetup2fa() {
    setSetup2faUserId(null);
    setSetup2faUri(null);
    setSetup2faBackupCodes([]);
    setSetup2faCode("");
  }

  if (!usersLoaded || !isAdmin) return null;

  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold">Manage Users</h2>
        <button
          onClick={() => setShowAddUser(!showAddUser)}
          className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
          Add User
        </button>
      </div>

      {/* Add user form */}
      {showAddUser && (
        <form onSubmit={handleAddUser} className="mb-4 p-4 bg-gray-50 dark:bg-gray-700/50 rounded-lg space-y-3">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Username</label>
            <input
              type="text"
              value={newUsername}
              onChange={(e) => setNewUsername(e.target.value)}
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              required
              minLength={3}
              autoFocus
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Password</label>
            <input
              type="password"
              value={newUserPassword}
              onChange={(e) => setNewUserPassword(e.target.value)}
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              required
              minLength={8}
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Role</label>
            <div className="flex gap-4">
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="radio"
                  checked={newUserRole === "user"}
                  onChange={() => setNewUserRole("user")}
                  className="accent-blue-600"
                />
                User
              </label>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="radio"
                  checked={newUserRole === "admin"}
                  onChange={() => setNewUserRole("admin")}
                  className="accent-blue-600"
                />
                Admin
              </label>
            </div>
          </div>
          <div className="flex gap-2">
            <button type="submit" className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm">
              Create User
            </button>
            <button type="button" onClick={() => setShowAddUser(false)} className="px-4 py-2 rounded-md text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700">
              Cancel
            </button>
          </div>
        </form>
      )}

      {/* User table */}
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-gray-200 dark:border-gray-700 text-left">
              <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Username</th>
              <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Role</th>
              <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">2FA</th>
              <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Created</th>
              <th className="pb-2 font-medium text-gray-500 dark:text-gray-400 text-right">Actions</th>
            </tr>
          </thead>
          <tbody>
            {managedUsers.map((u) => (
              <tr key={u.id} className="border-b border-gray-100 dark:border-gray-700/50">
                <td className="py-2.5 font-medium">{u.username}</td>
                <td className="py-2.5">
                  {managedUsers.length > 1 && managedUsers.some(mu => mu.role === "admin") ? (
                    <select
                      value={u.role}
                      onChange={(e) => handleChangeRole(u.id, e.target.value as "admin" | "user")}
                      className="text-xs border rounded px-2 py-1 bg-transparent focus:outline-none focus:ring-1 focus:ring-blue-500"
                    >
                      <option value="user">User</option>
                      <option value="admin">Admin</option>
                    </select>
                  ) : (
                    <span className="text-xs capitalize text-gray-600 dark:text-gray-400">{u.role}</span>
                  )}
                </td>
                <td className="py-2.5">
                  {u.totp_enabled ? (
                    <span className="inline-flex items-center gap-1 text-green-600 dark:text-green-400 text-xs">
                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                      </svg>
                      Enabled
                    </span>
                  ) : (
                    <button
                      onClick={() => handleAdminSetup2fa(u.id)}
                      disabled={setup2faLoading}
                      className="text-xs text-blue-600 dark:text-blue-400 hover:text-blue-800 dark:hover:text-blue-300 font-medium transition-colors disabled:opacity-50"
                    >
                      Enable
                    </button>
                  )}
                </td>
                <td className="py-2.5 text-xs text-gray-500 dark:text-gray-400">
                  {new Date(u.created_at).toLocaleDateString()}
                </td>
                <td className="py-2.5 text-right">
                  <div className="flex items-center justify-end gap-1">
                    {/* Reset Password */}
                    <button
                      onClick={() => { setResetPwUserId(resetPwUserId === u.id ? null : u.id); setResetPwValue(""); }}
                      className="p-1.5 rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-gray-500 dark:text-gray-400"
                      title="Reset password"
                    >
                      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 5.25a3 3 0 013 3m3 0a6 6 0 01-7.029 5.912c-.563-.097-1.159.026-1.563.43L10.5 17.25H8.25v2.25H6v2.25H2.25v-2.818c0-.597.237-1.17.659-1.591l6.499-6.499c.404-.404.527-1 .43-1.563A6 6 0 1121.75 8.25z" />
                      </svg>
                    </button>
                    {/* Reset 2FA (only if enabled) */}
                    {u.totp_enabled && (
                      <button
                        onClick={() => handleResetUser2fa(u.id)}
                        className="p-1.5 rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-gray-500 dark:text-gray-400"
                        title="Reset 2FA"
                      >
                        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z" />
                        </svg>
                      </button>
                    )}
                    {/* Delete */}
                    <button
                      onClick={() => setConfirmDeleteId(confirmDeleteId === u.id ? null : u.id)}
                      className="p-1.5 rounded hover:bg-red-50 dark:hover:bg-red-900/20 text-red-500"
                      title="Delete user"
                    >
                      <AppIcon name="trashcan" />
                    </button>
                  </div>
                  {/* Reset Password inline form */}
                  {resetPwUserId === u.id && (
                    <div className="flex gap-1 mt-2 justify-end">
                      <input
                        type="password"
                        value={resetPwValue}
                        onChange={(e) => setResetPwValue(e.target.value)}
                        placeholder="New password"
                        className="border rounded px-2 py-1 text-xs w-36 focus:outline-none focus:ring-1 focus:ring-blue-500"
                        autoFocus
                      />
                      <button
                        onClick={() => handleResetUserPassword(u.id)}
                        className="bg-blue-600 text-white px-2 py-1 rounded text-xs hover:bg-blue-700"
                      >
                        Set
                      </button>
                    </div>
                  )}
                  {/* Delete confirmation */}
                  {confirmDeleteId === u.id && (
                    <div className="flex items-center gap-1 mt-2 justify-end">
                      <span className="text-xs text-red-600 dark:text-red-400">Delete?</span>
                      <button
                        onClick={() => handleDeleteUser(u.id)}
                        className="bg-red-600 text-white px-2 py-1 rounded text-xs hover:bg-red-700"
                      >
                        Yes
                      </button>
                      <button
                        onClick={() => setConfirmDeleteId(null)}
                        className="px-2 py-1 rounded text-xs text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700"
                      >
                        No
                      </button>
                    </div>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* 2FA Setup Modal */}
      {setup2faUserId && setup2faUri && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4">
          <div className="bg-white dark:bg-gray-800 rounded-xl shadow-2xl w-full max-w-md p-6">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-2">
              Enable 2FA for {managedUsers.find(u => u.id === setup2faUserId)?.username}
            </h3>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
              Scan this QR code with an authenticator app (Google Authenticator, Authy, etc.), then enter the 6-digit code to confirm.
            </p>

            <div className="flex justify-center mb-4">
              <QRCodeSVG value={setup2faUri} size={200} />
            </div>

            <div className="mb-4">
              <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">
                Verification Code
              </label>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={setup2faCode}
                  onChange={(e) => setSetup2faCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
                  onKeyDown={(e) => { if (e.key === "Enter") handleAdminConfirm2fa(); }}
                  placeholder="000000"
                  className="flex-1 border border-gray-300 dark:border-gray-600 rounded-md px-3 py-2 text-center font-mono text-lg tracking-widest focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700"
                  maxLength={6}
                  autoFocus
                />
                <button
                  onClick={handleAdminConfirm2fa}
                  disabled={setup2faLoading || setup2faCode.length !== 6}
                  className="px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
                >
                  {setup2faLoading ? "Verifying…" : "Confirm"}
                </button>
              </div>
            </div>

            {setup2faBackupCodes.length > 0 && (
              <details className="mb-4">
                <summary className="text-xs text-gray-500 dark:text-gray-400 cursor-pointer hover:text-gray-700 dark:hover:text-gray-300">
                  Backup codes (save these!)
                </summary>
                <div className="mt-2 grid grid-cols-2 gap-1 p-3 bg-gray-50 dark:bg-gray-900 rounded-md font-mono text-xs">
                  {setup2faBackupCodes.map((code, i) => (
                    <span key={i} className="text-gray-700 dark:text-gray-300">{code}</span>
                  ))}
                </div>
              </details>
            )}

            <button
              onClick={cancelAdminSetup2fa}
              className="w-full mt-2 px-4 py-2 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-800 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-md transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
