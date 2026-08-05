/**
 * Subscribe to the server's real-time album/gallery change stream (item #11)
 * and bump the {@link useSyncSignal} so data hooks refetch within seconds.
 *
 * `EventSource` can't set headers, so auth uses `?token=<jwt>` (same pattern as
 * the audit-log stream). Reconnects with exponential backoff on error; the
 * server filters events to the authenticated user, so we just react to any
 * event by bumping the signal (the actual data is pulled authoritatively).
 *
 * Mount once, high in the authenticated tree.
 */
import { useEffect } from "react";
import { BASE } from "../api/core";
import { useAuthStore } from "../store/auth";
import { useSyncSignal } from "../store/syncSignal";

export default function useSyncEvents() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  const bump = useSyncSignal((s) => s.bump);

  useEffect(() => {
    if (!isAuthenticated) return;

    let es: EventSource | null = null;
    let retry = 0;
    let closed = false;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    function connect() {
      if (closed) return;
      const token = useAuthStore.getState().accessToken;
      if (!token) return;

      es = new EventSource(`${BASE}/sync/events?token=${encodeURIComponent(token)}`);

      const onChange = (kind: string) => () => bump(kind);
      // Named events (server sets `event:` to the change kind) + a fallback for
      // the generic message channel.
      es.addEventListener("photo", onChange("photo"));
      es.addEventListener("album", onChange("album"));
      es.addEventListener("trash", onChange("trash"));
      es.addEventListener("resync", onChange("resync"));
      es.onmessage = onChange("message");

      es.onopen = () => {
        retry = 0;
      };
      es.onerror = () => {
        es?.close();
        es = null;
        if (closed) return;
        // Exponential backoff, capped at 30 s.
        const delay = Math.min(1000 * 2 ** retry, 30_000);
        retry += 1;
        reconnectTimer = setTimeout(connect, delay);
      };
    }

    connect();

    return () => {
      closed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      es?.close();
    };
  }, [isAuthenticated, bump]);
}
