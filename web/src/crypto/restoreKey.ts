/**
 * The one decision every page load makes: does this tab already have the E2E
 * key, can it get it from a peer, or must the user re-enter the password?
 *
 * Lives outside App.tsx so the sequence is testable without a DOM — it gates
 * the first render, and getting it wrong means either a login bounce with the
 * key sitting right there (#21) or, worse, silently loosening how long the key
 * survives. See keySession.ts for the trust argument.
 */
import { adoptKeyFromKeystore, loadKeyFromSession } from "./crypto";
import { peerSessionAlive } from "./keySession";

export type KeyRestoreOutcome =
  /** The session flag was present — a reload, or a window.open'd tab. */
  | "loaded"
  /** A live peer window vouched; the key came from the keystore. */
  | "adopted"
  /** No key: the app will ask for the password. */
  | "absent";

interface RestoreOptions {
  /** Whether a session token was found. A logged-out visitor has nothing to adopt. */
  isAuthenticated: boolean;
  /** Injectable for tests. */
  peerAlive?: () => Promise<boolean>;
}

/**
 * Restore the encryption key for this tab, returning what happened.
 *
 * Never throws: any failure resolves to "absent", which asks for the password —
 * the safe direction to fail in.
 */
export async function restoreKeyOnBoot({
  isAuthenticated,
  peerAlive = () => peerSessionAlive(),
}: RestoreOptions): Promise<KeyRestoreOutcome> {
  try {
    if (await loadKeyFromSession()) return "loaded";

    // Skip the handshake (and its timeout) for anyone who couldn't adopt a key
    // anyway — no token means no session for a peer to vouch for.
    if (!isAuthenticated) return "absent";

    if (!(await peerAlive())) return "absent";

    return (await adoptKeyFromKeystore()) ? "adopted" : "absent";
  } catch (err) {
    console.error("Failed to restore the encryption key on boot", err);
    return "absent";
  }
}
