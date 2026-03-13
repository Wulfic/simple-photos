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

// ── Input sanitization ──────────────────────────────────────────────────────

/**
 * Regex matching dangerous Unicode codepoints that should be stripped from
 * user-facing text inputs. Matches:
 * - C0 controls (U+0001–U+0008, U+000E–U+001F) except HT(09), LF(0A), CR(0D)
 * - DEL (U+007F)
 * - C1 controls (U+0080–U+009F)
 * - Bidi overrides (U+200E–U+200F, U+202A–U+202E, U+2066–U+2069)
 * - Zero-width chars (U+200B–U+200D, U+FEFF, U+FFFE)
 * - Interlinear annotation anchors (U+FFF9–U+FFFB)
 * - Object replacement (U+FFFC)
 *
 * Reference: https://github.com/minimaxir/big-list-of-naughty-strings
 */
const DANGEROUS_CHARS =
  // eslint-disable-next-line no-control-regex
  /[\u0001-\u0008\u000E-\u001F\u007F\u0080-\u009F\u200B-\u200F\u202A-\u202E\u2066-\u2069\uFEFF\uFFFE\uFFF9-\uFFFC]/g;

/**
 * Strip dangerous/invisible Unicode codepoints and trim whitespace.
 * Preserves normal printable Unicode (emoji, CJK, Arabic, etc.).
 *
 * NOTE: Currently unused — available for future sanitization needs
 * (e.g. album names, tag names, user display names).
 */
export function sanitizeText(input: string): string {
  return input.replace(DANGEROUS_CHARS, "").trim();
}

/**
 * Sanitize a display name (album, gallery, tag, server name, etc.):
 * strips dangerous chars, collapses whitespace, and truncates.
 */
export function sanitizeDisplayName(input: string, maxLen: number): string {
  return sanitizeText(input)
    .replace(/\s+/g, " ")
    .slice(0, maxLen);
}
