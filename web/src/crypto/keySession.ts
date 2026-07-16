/**
 * Cross-window key handoff for the "just open the app twice" split-screen
 * story (#21).
 *
 * The E2E master key is a non-extractable CryptoKey in IndexedDB (per-origin,
 * so every tab can *reach* it), while the `sp_key` sessionStorage flag is what
 * scopes it to a browser session: no flag ⇒ the app re-derives from the
 * password. A `window.open`ed tab inherits a COPY of sessionStorage and so
 * inherits the flag, but a manually-opened tab (Ctrl+T, paste the URL) does
 * not — it would bounce to the password screen while an unlocked tab sits right
 * next to it.
 *
 * Blindly reading IndexedDB when the flag is missing would fix that and quietly
 * destroy the session scoping: the key would then also be adopted after a
 * browser restart, days later, with no password — turning the master key into a
 * permanently-stored credential on that device.
 *
 * So we ask instead. A tab with no flag broadcasts "is any tab of this app
 * alive and holding the key?" and only adopts the key from IndexedDB if one
 * answers. A live unlocked tab is proof the session is still running; a browser
 * restart leaves nobody to answer, so the password is still required. Same
 * trust boundary as Android's process-scoped UnlockSession.
 */
import { randomUuid } from "../utils/uuid";

const CHANNEL_NAME = "sp-key-session";
const PING = "sp-key-session-ping";
const PONG = "sp-key-session-pong";

/**
 * How long to wait for a peer to vouch. A same-origin BroadcastChannel
 * round-trip is sub-millisecond; the budget is for a peer whose main thread is
 * mid-decrypt. Only ever paid on a fresh tab of an authenticated session.
 */
const REPLY_TIMEOUT_MS = 500;

interface PingMessage {
  type: typeof PING;
  /** Tab that wants the key — replies are addressed back to it. */
  from: string;
}

interface PongMessage {
  type: typeof PONG;
  to: string;
}

type ChannelFactory = () => BroadcastChannel;

/**
 * Identifies THIS tab. Both the responder and the requester in a single tab
 * share it, which is what stops a tab from answering its own ping: two
 * BroadcastChannel objects in one tab do hear each other, and a self-pong would
 * "prove" a peer exists when none does.
 *
 * Via [randomUuid], not `crypto.randomUUID()` — the latter is undefined outside
 * a secure context, and this module is imported at boot on a plain-HTTP LAN.
 */
const TAB_ID = randomUuid();

const defaultFactory: ChannelFactory = () => new BroadcastChannel(CHANNEL_NAME);

/** Whether this browser can do the handoff at all. */
export function supportsKeySession(): boolean {
  return typeof BroadcastChannel !== "undefined";
}

function isPing(data: unknown): data is PingMessage {
  return (
    typeof data === "object" &&
    data !== null &&
    (data as PingMessage).type === PING &&
    typeof (data as PingMessage).from === "string"
  );
}

function isPong(data: unknown): data is PongMessage {
  return (
    typeof data === "object" &&
    data !== null &&
    (data as PongMessage).type === PONG &&
    typeof (data as PongMessage).to === "string"
  );
}

interface ResponderOptions {
  tabId?: string;
  factory?: ChannelFactory;
}

/**
 * Start answering "is anyone unlocked?" pings from other tabs, for as long as
 * [hasKey] says this tab holds the key. Returns a teardown function.
 *
 * Note this vouches for the *session*, never for the key itself — no key
 * material, and no handle to it, is ever put on the channel. The asking tab
 * reads the key from its own IndexedDB.
 */
export function startKeySessionResponder(
  hasKey: () => boolean,
  { tabId = TAB_ID, factory = defaultFactory }: ResponderOptions = {},
): () => void {
  if (!supportsKeySession()) return () => {};

  let channel: BroadcastChannel;
  try {
    channel = factory();
  } catch (err) {
    console.error("Key session: could not open the handoff channel", err);
    return () => {};
  }

  const onMessage = (event: MessageEvent) => {
    if (!isPing(event.data)) return;
    if (event.data.from === tabId) return; // our own requester — never self-vouch
    if (!hasKey()) return;
    const pong: PongMessage = { type: PONG, to: event.data.from };
    try {
      channel.postMessage(pong);
    } catch (err) {
      console.error("Key session: failed to answer a peer's key-session ping", err);
    }
  };

  channel.addEventListener("message", onMessage);
  return () => {
    channel.removeEventListener("message", onMessage);
    channel.close();
  };
}

interface RequestOptions {
  timeoutMs?: number;
  tabId?: string;
  factory?: ChannelFactory;
}

/**
 * Whether another tab of this app is alive and still holds the encryption key.
 *
 * Resolves false (never rejects) on timeout, on an unsupported browser, or if
 * the channel can't be opened — the caller then falls back to asking for the
 * password, which is the safe direction to fail in.
 */
export function peerSessionAlive({
  timeoutMs = REPLY_TIMEOUT_MS,
  tabId = TAB_ID,
  factory = defaultFactory,
}: RequestOptions = {}): Promise<boolean> {
  if (!supportsKeySession()) return Promise.resolve(false);

  return new Promise<boolean>((resolve) => {
    let channel: BroadcastChannel;
    try {
      channel = factory();
    } catch (err) {
      console.error("Key session: could not open the handoff channel", err);
      resolve(false);
      return;
    }

    let settled = false;
    const finish = (alive: boolean) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      channel.removeEventListener("message", onMessage);
      channel.close();
      resolve(alive);
    };

    const onMessage = (event: MessageEvent) => {
      if (isPong(event.data) && event.data.to === tabId) finish(true);
    };
    channel.addEventListener("message", onMessage);

    const timer = setTimeout(() => finish(false), timeoutMs);

    const ping: PingMessage = { type: PING, from: tabId };
    try {
      channel.postMessage(ping);
    } catch (err) {
      console.error("Key session: failed to ask peers for the key session", err);
      finish(false);
    }
  });
}
