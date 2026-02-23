import { useState } from "react";
import { type PasswordStrength } from "../utils/validation";

// ── Checkmark ─────────────────────────────────────────────────────────────────

export const Checkmark = ({ ok }: { ok: boolean }) => (
  <span className={ok ? "text-green-600 dark:text-green-400" : "text-gray-400"}>
    {ok ? "\u2713" : "\u25CB"}
  </span>
);

// ── Eye icon SVG ──────────────────────────────────────────────────────────────

export const EyeIcon = ({ open }: { open: boolean }) => (
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

// ── Password input with strength meter ────────────────────────────────────────

export function PasswordField({
  value,
  onChange,
  pwData,
  label = "Password",
}: {
  value: string;
  onChange: (v: string) => void;
  pwData: PasswordStrength;
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

// ── Confirm password input with match indicator ───────────────────────────────

export function ConfirmPasswordField({
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
