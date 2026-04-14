/**
 * Web client diagnostic logger.
 *
 * Captures errors, performance metrics, and API issues, then batches and sends
 * them to the server via `POST /api/client-logs`.
 *
 * Respects a server-controlled enable/disable flag. When disabled, logs are
 * still written to the browser console but never sent to the server.
 *
 * This is entirely best-effort — failures in the logging pipeline are silently
 * swallowed to avoid interfering with the actual app experience.
 */

import { request } from "../api/core";

// ── Types ────────────────────────────────────────────────────────────────────

interface LogEntry {
  level: "debug" | "info" | "warn" | "error";
  tag: string;
  message: string;
  context?: Record<string, unknown>;
  client_ts: string;
}

interface LogBatch {
  session_id: string;
  entries: LogEntry[];
}

type LogLevel = LogEntry["level"];

// ── Session ID ───────────────────────────────────────────────────────────────

function generateSessionId(): string {
  const arr = new Uint8Array(8);
  crypto.getRandomValues(arr);
  const hex = Array.from(arr, (b) => b.toString(16).padStart(2, "0")).join("");
  return `web-${Date.now()}-${hex}`;
}

// ── Logger Singleton ─────────────────────────────────────────────────────────

class DiagnosticLogger {
  private static instance: DiagnosticLogger | null = null;

  private enabled = false;
  private sessionId = generateSessionId();
  private buffer: LogEntry[] = [];
  private flushTimer: ReturnType<typeof setInterval> | null = null;
  private readonly MAX_BUFFER = 200;
  private readonly FLUSH_INTERVAL_MS = 30_000; // flush every 30s when enabled

  // Global error handlers we need to clean up
  private errorHandler: ((e: ErrorEvent) => void) | null = null;
  private rejectionHandler: ((e: PromiseRejectionEvent) => void) | null = null;

  private constructor() {}

  static getInstance(): DiagnosticLogger {
    if (!DiagnosticLogger.instance) {
      DiagnosticLogger.instance = new DiagnosticLogger();
    }
    return DiagnosticLogger.instance;
  }

  // ── Enable / Disable ────────────────────────────────────────────────────

  /**
   * Enable diagnostic logging. Installs global error handlers and starts
   * periodic flush timer.
   */
  enable(): void {
    if (this.enabled) return;
    this.enabled = true;

    // Install global error capturing
    this.errorHandler = (e: ErrorEvent) => {
      this.error("GlobalError", e.message || "Unhandled error", {
        filename: e.filename,
        lineno: e.lineno,
        colno: e.colno,
      });
    };
    window.addEventListener("error", this.errorHandler);

    this.rejectionHandler = (e: PromiseRejectionEvent) => {
      const reason =
        e.reason instanceof Error
          ? e.reason.message
          : String(e.reason ?? "Unknown rejection");
      this.error("UnhandledRejection", reason);
    };
    window.addEventListener("unhandledrejection", this.rejectionHandler);

    // Start periodic flush
    this.flushTimer = setInterval(() => this.flush(), this.FLUSH_INTERVAL_MS);

    this.info("DiagnosticLogger", "Client diagnostics enabled", {
      userAgent: navigator.userAgent,
      url: location.href,
      sessionId: this.sessionId,
    });
  }

  /**
   * Disable diagnostic logging. Removes global error handlers, flushes
   * remaining buffer, and stops the flush timer.
   */
  disable(): void {
    if (!this.enabled) return;
    this.enabled = false;

    // Remove global handlers
    if (this.errorHandler) {
      window.removeEventListener("error", this.errorHandler);
      this.errorHandler = null;
    }
    if (this.rejectionHandler) {
      window.removeEventListener("unhandledrejection", this.rejectionHandler);
      this.rejectionHandler = null;
    }

    // Stop timer and flush remaining
    if (this.flushTimer) {
      clearInterval(this.flushTimer);
      this.flushTimer = null;
    }
    this.flush();
  }

  isEnabled(): boolean {
    return this.enabled;
  }

  getSessionId(): string {
    return this.sessionId;
  }

  // ── Logging Methods ─────────────────────────────────────────────────────

  debug(tag: string, message: string, context?: Record<string, unknown>): void {
    this.add("debug", tag, message, context);
  }

  info(tag: string, message: string, context?: Record<string, unknown>): void {
    this.add("info", tag, message, context);
  }

  warn(tag: string, message: string, context?: Record<string, unknown>): void {
    this.add("warn", tag, message, context);
  }

  error(tag: string, message: string, context?: Record<string, unknown>): void {
    this.add("error", tag, message, context);
  }

  /**
   * Log an API call with timing information.
   * Call at the start of an API request and use the returned function to
   * record completion.
   */
  apiCall(method: string, path: string): (status: number, error?: string) => void {
    const start = performance.now();
    return (status: number, error?: string) => {
      const durationMs = Math.round(performance.now() - start);
      const level: LogLevel = status >= 500 ? "error" : status >= 400 ? "warn" : "debug";
      this.add(level, "API", `${method} ${path} → ${status} (${durationMs}ms)`, {
        method,
        path,
        status,
        durationMs,
        ...(error ? { error } : {}),
      });
    };
  }

  /**
   * Capture Web Vitals / performance metrics from the browser.
   * Call once after page load.
   */
  capturePerformance(): void {
    if (!this.enabled) return;

    try {
      const nav = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
      if (nav) {
        this.info("Performance", "Page load metrics", {
          domContentLoaded: Math.round(nav.domContentLoadedEventEnd - nav.startTime),
          loadComplete: Math.round(nav.loadEventEnd - nav.startTime),
          domInteractive: Math.round(nav.domInteractive - nav.startTime),
          ttfb: Math.round(nav.responseStart - nav.requestStart),
          transferSize: nav.transferSize,
        });
      }

      // Memory info (Chrome only)
      const mem = (performance as unknown as { memory?: { usedJSHeapSize: number; totalJSHeapSize: number; jsHeapSizeLimit: number } }).memory;
      if (mem) {
        this.info("Performance", "Memory usage", {
          usedHeapMB: Math.round(mem.usedJSHeapSize / 1048576),
          totalHeapMB: Math.round(mem.totalJSHeapSize / 1048576),
          heapLimitMB: Math.round(mem.jsHeapSizeLimit / 1048576),
        });
      }
    } catch {
      // Silently ignore — diagnostic logging must never break the app
    }
  }

  // ── Internals ───────────────────────────────────────────────────────────

  private add(
    level: LogLevel,
    tag: string,
    message: string,
    context?: Record<string, unknown>
  ): void {
    // Always log to console in development
    const consoleFn = level === "error" ? console.error
      : level === "warn" ? console.warn
      : level === "debug" ? console.debug
      : console.info;
    consoleFn(`[${tag}] ${message}`, context ?? "");

    // Only buffer if enabled
    if (!this.enabled) return;
    if (this.buffer.length >= this.MAX_BUFFER) return;

    this.buffer.push({
      level,
      tag,
      message: message.substring(0, 4096),
      context,
      client_ts: new Date().toISOString(),
    });
  }

  /**
   * Send buffered entries to the server. Best-effort — failures are swallowed.
   */
  async flush(): Promise<void> {
    if (this.buffer.length === 0) return;

    const entries = this.buffer.splice(0, this.buffer.length);
    const batch: LogBatch = {
      session_id: this.sessionId,
      entries,
    };

    try {
      await request("/client-logs", {
        method: "POST",
        body: JSON.stringify(batch),
      });
    } catch {
      // Best-effort — never let logging failures affect the app
    }
  }
}

// ── Public API ───────────────────────────────────────────────────────────────

/** Singleton diagnostic logger instance */
export const diagnosticLogger = DiagnosticLogger.getInstance();

export default diagnosticLogger;
