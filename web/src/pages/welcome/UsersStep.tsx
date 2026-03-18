/** Wizard step — create additional user accounts during initial setup. */
import type { PasswordStrength } from "../../utils/validation";
import { Checkmark, PasswordField, ConfirmPasswordField } from "../../components/PasswordFields";
import type { WizardStep, CreatedUser } from "./types";

export interface UsersStepProps {
  createdUsers: CreatedUser[];
  newUsername: string;
  setNewUsername: (v: string) => void;
  newPassword: string;
  setNewPassword: (v: string) => void;
  newConfirmPassword: string;
  setNewConfirmPassword: (v: string) => void;
  newRole: "user" | "admin";
  setNewRole: (v: "user" | "admin") => void;
  showUserForm: boolean;
  setShowUserForm: (v: boolean) => void;
  newPw: PasswordStrength;
  newUn: { length: boolean; chars: boolean };
  loading: boolean;
  error: string;
  handleCreateUser: (e: React.FormEvent) => void;
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
}

export default function UsersStep({
  createdUsers,
  newUsername,
  setNewUsername,
  newPassword,
  setNewPassword,
  newConfirmPassword,
  setNewConfirmPassword,
  newRole,
  setNewRole,
  showUserForm,
  setShowUserForm,
  newPw,
  newUn,
  loading,
  error,
  handleCreateUser,
  setStep,
  setError,
}: UsersStepProps) {
  return (
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
              className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium"
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
          className="w-full border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg py-3 text-gray-500 dark:text-gray-400 hover:border-blue-400 dark:hover:border-blue-500 hover:text-blue-600 dark:hover:text-blue-400 transition-colors text-sm font-medium"
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
  );
}
