import { describe, it, expect } from "vitest";
import {
  buildAuditLogQuery,
  matchesAuditFilters,
  FAILURE_EVENTS,
  type AuditStreamFilters,
} from "./auditLogQuery";
import type { AuditLogEntry } from "./types";

/** Parse the built query back into a plain object for readable assertions. */
function parse(qs: string): Record<string, string> {
  return Object.fromEntries(new URLSearchParams(qs).entries());
}

describe("buildAuditLogQuery", () => {
  it("returns an empty string when there are no filters", () => {
    expect(buildAuditLogQuery()).toBe("");
    expect(buildAuditLogQuery({})).toBe("");
  });

  it("serializes the pre-existing filters", () => {
    const qs = parse(
      buildAuditLogQuery({
        event_type: "login_failure",
        user_id: "u-1",
        ip_address: "192.168.1.1",
        after: "2026-07-01T00:00:00Z",
        before: "2026-07-21T00:00:00Z",
        limit: 100,
      })
    );
    expect(qs).toEqual({
      event_type: "login_failure",
      user_id: "u-1",
      ip_address: "192.168.1.1",
      after: "2026-07-01T00:00:00Z",
      before: "2026-07-21T00:00:00Z",
      limit: "100",
    });
  });

  // Regression: ServerLogsTab has always passed `source_server`, and the client
  // silently dropped it. The Source dropdown filtered nothing.
  it("sends source_server so the Source dropdown is not decorative", () => {
    expect(parse(buildAuditLogQuery({ source_server: "backup-01" })))
      .toEqual({ source_server: "backup-01" });
  });

  // "local" is a server-side sentinel meaning `source_server IS NULL`. If the
  // client ever "helpfully" translated it, this-server-only would break.
  it("passes the 'local' sentinel through untouched", () => {
    expect(parse(buildAuditLogQuery({ source_server: "local" })))
      .toEqual({ source_server: "local" });
  });

  it("omits source_server when it is empty", () => {
    expect(buildAuditLogQuery({ source_server: "" })).toBe("");
  });

  // #45 — the failures filter must reach the server, because filtering the
  // fetched page client-side reports "no failures" whenever the newest 100
  // events are logins.
  it("sends failures_only=true when the filter is on", () => {
    expect(parse(buildAuditLogQuery({ failures_only: true })))
      .toEqual({ failures_only: "true" });
  });

  it("omits failures_only when off, rather than sending false", () => {
    expect(buildAuditLogQuery({ failures_only: false })).toBe("");
  });

  it("combines failures_only with the other filters", () => {
    const qs = parse(
      buildAuditLogQuery({
        failures_only: true,
        source_server: "local",
        after: "2026-07-20T00:00:00Z",
        limit: 100,
      })
    );
    expect(qs).toEqual({
      failures_only: "true",
      source_server: "local",
      after: "2026-07-20T00:00:00Z",
      limit: "100",
    });
  });

  it("percent-encodes values instead of injecting raw query syntax", () => {
    const qs = buildAuditLogQuery({ ip_address: "a&b=c" });
    expect(qs).toBe("ip_address=a%26b%3Dc");
    expect(parse(qs)).toEqual({ ip_address: "a&b=c" });
  });
});

const NO_FILTERS: AuditStreamFilters = {
  eventFilter: "",
  ipFilter: "",
  serverFilter: "",
  failuresOnly: false,
};

function entry(over: Partial<AuditLogEntry> = {}): AuditLogEntry {
  return {
    id: "a-1",
    event_type: "login_success",
    user_id: "u-1",
    username: "tyler",
    ip_address: "192.168.1.50",
    user_agent: "test",
    details: "{}",
    created_at: "2026-07-21T00:00:00Z",
    source_server: null,
    ...over,
  };
}

describe("matchesAuditFilters", () => {
  it("admits everything when no filter is set", () => {
    expect(matchesAuditFilters(entry(), NO_FILTERS)).toBe(true);
  });

  // The whole point: the SSE stream carries every event, so a success must not
  // slip into a failures-only view.
  it("rejects a success event while failures-only is on", () => {
    const f = { ...NO_FILTERS, failuresOnly: true };
    expect(matchesAuditFilters(entry({ event_type: "login_success" }), f)).toBe(false);
  });

  it("admits pipeline failures while failures-only is on", () => {
    const f = { ...NO_FILTERS, failuresOnly: true };
    for (const ev of [
      "media_convert_failure",
      "import_failure",
      "encryption_failure",
      "encryption_parked",
      "thumbnail_failure",
      "conversion_retired",
    ]) {
      expect(matchesAuditFilters(entry({ event_type: ev }), f)).toBe(true);
    }
  });

  it("covers the pipeline events in FAILURE_EVENTS", () => {
    expect(FAILURE_EVENTS.has("media_convert_failure")).toBe(true);
    expect(FAILURE_EVENTS.has("import_failure")).toBe(true);
    expect(FAILURE_EVENTS.has("encryption_failure")).toBe(true);
    expect(FAILURE_EVENTS.has("thumbnail_failure")).toBe(true);
    // #40. A retirement is the one row that explains a file which stopped
    // appearing entirely, so it must survive "Failures only" — that filter is
    // exactly where a user goes to ask why.
    expect(FAILURE_EVENTS.has("conversion_retired")).toBe(true);
    // B3a. The other terminal event, and the one with a confidentiality
    // consequence: a parked photo's original stays unencrypted on disk and
    // nothing retries it. It is invisible in the encryption banner by design
    // (it would wedge the bar), so the logs are the only place it can surface —
    // dropping it from this set makes it invisible everywhere.
    expect(FAILURE_EVENTS.has("encryption_parked")).toBe(true);
    expect(FAILURE_EVENTS.has("media_convert")).toBe(false);
  });

  it("matches the event-type dropdown exactly, not by prefix", () => {
    const f = { ...NO_FILTERS, eventFilter: "media_convert" };
    expect(matchesAuditFilters(entry({ event_type: "media_convert" }), f)).toBe(true);
    expect(matchesAuditFilters(entry({ event_type: "media_convert_failure" }), f)).toBe(false);
  });

  it("matches IP by substring so a partial prefix is usable", () => {
    const f = { ...NO_FILTERS, ipFilter: "192.168.1." };
    expect(matchesAuditFilters(entry({ ip_address: "192.168.1.50" }), f)).toBe(true);
    expect(matchesAuditFilters(entry({ ip_address: "10.0.0.5" }), f)).toBe(false);
  });

  // "local" is the sentinel for this server, which is stored as NULL.
  it("treats the 'local' source filter as source_server === null", () => {
    const f = { ...NO_FILTERS, serverFilter: "local" };
    expect(matchesAuditFilters(entry({ source_server: null }), f)).toBe(true);
    expect(matchesAuditFilters(entry({ source_server: "backup-01" }), f)).toBe(false);
  });

  it("matches a named source server exactly", () => {
    const f = { ...NO_FILTERS, serverFilter: "backup-01" };
    expect(matchesAuditFilters(entry({ source_server: "backup-01" }), f)).toBe(true);
    expect(matchesAuditFilters(entry({ source_server: null }), f)).toBe(false);
  });

  it("requires every active filter to pass, not just one", () => {
    const f = { ...NO_FILTERS, failuresOnly: true, serverFilter: "local" };
    const failureFromBackup = entry({
      event_type: "import_failure",
      source_server: "backup-01",
    });
    expect(matchesAuditFilters(failureFromBackup, f)).toBe(false);
    expect(
      matchesAuditFilters(entry({ event_type: "import_failure", source_server: null }), f)
    ).toBe(true);
  });
});
