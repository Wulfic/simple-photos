/** Minimal className joiner — drops falsy entries and joins with a space.
 * Avoids a clsx dependency for the handful of UI primitives that need it. */
export function cn(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}
