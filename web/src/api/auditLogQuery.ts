/**
 * Query-string construction for `GET /api/admin/audit-logs`.
 *
 * Lives in its own module, importing nothing at runtime, so it can be unit
 * tested without dragging in `./core` (which pulls Dexie, the auth store and
 * the crypto module along with it). `web/` has no jsdom — pure functions are
 * the only thing that can actually be covered here.
 */
import type { AuditLogEntry, AuditLogParams } from "./types";

/**
 * Event types the "Failures only" filter selects.
 *
 * Mirrors `FAILURE_EVENTS` in `server/src/audit.rs` — the server is
 * authoritative for what the *query* returns; this copy exists only so
 * live-streamed (SSE) entries can be filtered client-side with the same
 * verdict. If you add a failure variant server-side, add it here too, or
 * streamed rows will disagree with fetched ones.
 */
export const FAILURE_EVENTS: ReadonlySet<string> = new Set([
  "media_convert_failure",
  "import_failure",
  "encryption_failure",
  // Terminal (B3a). The per-attempt failure above can be transient; this one
  // means the server stopped trying and the original is still plaintext on
  // disk. "Failures only" is exactly where an operator goes to ask why.
  "encryption_parked",
  "thumbnail_failure",
  "conversion_retired",
  "login_failure",
  "totp_login_failure",
  "rate_limited",
  "account_locked",
]);

/** The subset of audit-log filters the server applies to a query. */
export interface AuditStreamFilters {
  eventFilter: string;
  ipFilter: string;
  serverFilter: string;
  failuresOnly: boolean;
}

/**
 * Whether a live-streamed audit entry belongs in the currently-filtered view.
 *
 * The SSE connection is opened once for the lifetime of the tab and delivers
 * *every* event, so without this an entry that the same filters would have
 * excluded from the fetched page gets prepended anyway — e.g. a `login_success`
 * appearing at the top of a "Failures only" list.
 *
 * Date-range filters are deliberately not checked: a just-streamed event is
 * always inside any "last N" window.
 */
export function matchesAuditFilters(
  entry: AuditLogEntry,
  filters: AuditStreamFilters
): boolean {
  if (filters.failuresOnly && !FAILURE_EVENTS.has(entry.event_type)) return false;
  if (filters.eventFilter && entry.event_type !== filters.eventFilter) return false;
  // Substring match, matching the server's LIKE-free equality on the fetch path
  // as closely as the text box's "e.g. 192.168.1.1" placeholder implies.
  if (filters.ipFilter && !entry.ip_address.includes(filters.ipFilter)) return false;
  if (filters.serverFilter) {
    // `"local"` means "this server", which the server stores as NULL.
    const matches =
      filters.serverFilter === "local"
        ? entry.source_server === null
        : entry.source_server === filters.serverFilter;
    if (!matches) return false;
  }
  return true;
}

/**
 * Serialize audit-log filters into a URL query string (no leading `?`).
 *
 * Every filter is applied **server-side**. Filtering the already-fetched page
 * in the browser would report "no failures" whenever the most recent 100
 * events happened to be logins — a worse answer than no filter at all,
 * because it looks authoritative.
 *
 * Falsy values are omitted rather than sent: the server treats an absent
 * `failures_only` as `false`, and an empty `source_server` as "all servers".
 */
export function buildAuditLogQuery(params?: AuditLogParams): string {
  const search = new URLSearchParams();
  if (params?.event_type) search.set("event_type", params.event_type);
  if (params?.user_id) search.set("user_id", params.user_id);
  if (params?.ip_address) search.set("ip_address", params.ip_address);
  if (params?.after) search.set("after", params.after);
  if (params?.before) search.set("before", params.before);
  if (params?.limit) search.set("limit", params.limit.toString());
  // `"local"` is a sentinel the server maps to `source_server IS NULL`, not a
  // server name — pass it through untouched.
  if (params?.source_server) search.set("source_server", params.source_server);
  if (params?.failures_only) search.set("failures_only", "true");
  return search.toString();
}
