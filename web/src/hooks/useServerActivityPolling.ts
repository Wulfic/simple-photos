/**
 * Poll `/api/status/activity` and mirror the server's AI / geo busy flags
 * into the local processing store so the profile-avatar spinner in
 * `AppHeader` rotates whenever the server is doing background work for
 * the current user (face/object detection, reverse-geocoding, etc.).
 *
 * Polling happens on a 3-second interval while the page is visible and
 * the user is authenticated; it stops when the tab is hidden to avoid
 * waste, and resumes on visibility change.
 */
import { useEffect } from "react";
import { request } from "../api/core";
import { useAuthStore } from "../store/auth";
import { useProcessingStore } from "../store/processing";

interface ActivityResponse {
  ai: boolean;
  geo: boolean;
  active: boolean;
}

const POLL_INTERVAL_MS = 3000;

export default function useServerActivityPolling() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  const startTask = useProcessingStore((s) => s.startTask);
  const endTask = useProcessingStore((s) => s.endTask);

  useEffect(() => {
    if (!isAuthenticated) return;

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    // Track which server flags we currently have "started" so we don't
    // call startTask on every poll cycle.
    const started = { ai: false, geo: false };

    async function tick() {
      if (cancelled) return;
      try {
        const res = await request<ActivityResponse>("/status/activity");
        if (cancelled) return;

        if (res.ai && !started.ai) { startTask("ai"); started.ai = true; }
        else if (!res.ai && started.ai) { endTask("ai"); started.ai = false; }

        if (res.geo && !started.geo) { startTask("geo"); started.geo = true; }
        else if (!res.geo && started.geo) { endTask("geo"); started.geo = false; }
      } catch {
        // Network blip or auth refresh in progress — silently retry.
      } finally {
        if (!cancelled && !document.hidden) {
          timer = setTimeout(tick, POLL_INTERVAL_MS);
        }
      }
    }

    function onVisibility() {
      if (document.hidden) {
        if (timer) { clearTimeout(timer); timer = null; }
      } else if (!timer) {
        // Resume polling immediately on tab focus.
        tick();
      }
    }

    document.addEventListener("visibilitychange", onVisibility);
    tick();

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
      // Make sure we don't leave the spinner stuck on after unmount.
      if (started.ai) endTask("ai");
      if (started.geo) endTask("geo");
    };
  }, [isAuthenticated, startTask, endTask]);
}
