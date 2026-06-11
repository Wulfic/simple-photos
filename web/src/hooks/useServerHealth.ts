/**
 * useServerHealth — polls `/health` to detect server outages at runtime.
 *
 * Behaviour:
 *  - Polls every `ONLINE_INTERVAL_MS` while the server is reachable.
 *  - On failure, switches to exponential backoff (3 s → 5 s → 10 s → 20 s → 30 s max).
 *  - Pauses polling when the tab is hidden; resumes and checks immediately on show.
 *  - Returns:
 *      isOnline          — current reachability
 *      justRecovered     — true for RECOVERED_FLASH_MS after an outage clears
 *      secondsUntilRetry — countdown to next poll (null while online)
 *      retryCount        — how many consecutive failures have occurred
 */
import { useEffect, useRef, useState, useCallback } from "react";

const ONLINE_INTERVAL_MS   = 15_000;   // poll every 15 s while healthy
const BACKOFF_STEPS_MS     = [3_000, 5_000, 10_000, 20_000, 30_000];
const RECOVERED_FLASH_MS   = 3_000;    // how long to show the "Reconnected" state
const HEALTH_TIMEOUT_MS    = 8_000;    // abort the request after 8 s

export interface ServerHealthState {
  isOnline: boolean;
  justRecovered: boolean;
  secondsUntilRetry: number | null;
  retryCount: number;
}

export function useServerHealth(): ServerHealthState {
  const [isOnline, setIsOnline]                 = useState(true);
  const [justRecovered, setJustRecovered]       = useState(false);
  const [secondsUntilRetry, setSecondsUntilRetry] = useState<number | null>(null);
  const [retryCount, setRetryCount]             = useState(0);

  const retryCountRef    = useRef(0);
  const timerRef         = useRef<ReturnType<typeof setTimeout> | null>(null);
  const countdownRef     = useRef<ReturnType<typeof setInterval> | null>(null);
  const recoveryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cancelledRef     = useRef(false);

  const clearTimers = useCallback(() => {
    if (timerRef.current)         { clearTimeout(timerRef.current);   timerRef.current = null; }
    if (countdownRef.current)     { clearInterval(countdownRef.current); countdownRef.current = null; }
  }, []);

  const startCountdown = useCallback((delayMs: number) => {
    if (countdownRef.current) clearInterval(countdownRef.current);
    const target = Date.now() + delayMs;
    setSecondsUntilRetry(Math.ceil(delayMs / 1000));
    countdownRef.current = setInterval(() => {
      const remaining = Math.ceil((target - Date.now()) / 1000);
      if (remaining <= 0) {
        clearInterval(countdownRef.current!);
        countdownRef.current = null;
        setSecondsUntilRetry(null);
      } else {
        setSecondsUntilRetry(remaining);
      }
    }, 500);
  }, []);

  const scheduleNext = useCallback((delayMs: number, check: () => void) => {
    timerRef.current = setTimeout(check, delayMs);
    if (retryCountRef.current > 0) {
      // Only show countdown when we are in backoff mode (server was offline).
      startCountdown(delayMs);
    }
  }, [startCountdown]);

  const check = useCallback(async () => {
    if (cancelledRef.current || document.hidden) return;

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), HEALTH_TIMEOUT_MS);

    try {
      const res = await fetch("/health", {
        method: "GET",
        cache: "no-store",
        signal: controller.signal,
      });
      clearTimeout(timeout);

      if (!res.ok) throw new Error(`HTTP ${res.status}`);

      // ── Server is reachable ──────────────────────────────────────────────
      if (retryCountRef.current > 0) {
        // Was offline — flash "Reconnected"
        setJustRecovered(true);
        if (recoveryTimerRef.current) clearTimeout(recoveryTimerRef.current);
        recoveryTimerRef.current = setTimeout(() => {
          if (!cancelledRef.current) setJustRecovered(false);
        }, RECOVERED_FLASH_MS);
      }
      retryCountRef.current = 0;
      setRetryCount(0);
      setIsOnline(true);
      setSecondsUntilRetry(null);
      if (countdownRef.current) { clearInterval(countdownRef.current); countdownRef.current = null; }

      scheduleNext(ONLINE_INTERVAL_MS, check);
    } catch {
      clearTimeout(timeout);
      if (cancelledRef.current) return;

      // ── Server unreachable ───────────────────────────────────────────────
      retryCountRef.current += 1;
      setRetryCount(retryCountRef.current);
      setIsOnline(false);

      const backoffIndex = Math.min(retryCountRef.current - 1, BACKOFF_STEPS_MS.length - 1);
      const delay = BACKOFF_STEPS_MS[backoffIndex];
      scheduleNext(delay, check);
    }
  }, [scheduleNext]);

  // Kick off first check and wire up visibility listener.
  useEffect(() => {
    cancelledRef.current = false;
    // Small delay so the app can finish loading before the first check.
    timerRef.current = setTimeout(check, 5_000);

    function onVisibility() {
      if (!document.hidden) {
        // Tab became visible — clear any pending timer and check immediately.
        clearTimers();
        check();
      } else {
        // Tab hidden — pause polling to save resources.
        clearTimers();
      }
    }

    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelledRef.current = true;
      clearTimers();
      if (recoveryTimerRef.current) clearTimeout(recoveryTimerRef.current);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [check, clearTimers]);

  return { isOnline, justRecovered, secondsUntilRetry, retryCount };
}
