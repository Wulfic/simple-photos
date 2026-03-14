/**
 * Client-side input validation utilities — password strength checking and
 * username format validation. Rules mirror the server-side validation in
 * `auth/validation.rs` to provide instant feedback before submission.
 */

/** Password strength result returned by `checkPasswordStrength`. */
export interface PasswordStrength {
  checks: {
    length: boolean;
    uppercase: boolean;
    lowercase: boolean;
    digit: boolean;
    long: boolean;
    special: boolean;
  };
  core: boolean;
  score: number;
  label: string;
  color: string;
  max: number;
}

/** Evaluate password strength against multiple criteria. */
export function checkPasswordStrength(pw: string): PasswordStrength {
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

/** Validate a username: alphanumeric + underscore, 3–50 chars. */
export function checkUsername(name: string) {
  return {
    length: name.length >= 3 && name.length <= 50,
    chars: /^[a-zA-Z0-9_]+$/.test(name),
  };
}
