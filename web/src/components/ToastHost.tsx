/**
 * Global toast host — renders the stack of active notifications from the
 * toast store (#8). Mounted once in `ProtectedLayout` so every authenticated
 * page can surface dismissible popups instead of inline under-navbar error
 * bars (which shifted layout and were easy to miss).
 *
 * Positioned top-center, above all banners and the upload FAB, with
 * pointer-events confined to the cards so it never blocks the page behind it.
 */
import { useEffect } from "react";
import { useToastStore, type Toast, type ToastKind } from "../store/toast";

const KIND_STYLES: Record<ToastKind, { bar: string; icon: string; ring: string }> = {
  error: {
    bar: "bg-red-500 dark:bg-red-400",
    icon: "text-red-500 dark:text-red-400",
    ring: "border-red-200 dark:border-red-900/60",
  },
  success: {
    bar: "bg-green-500 dark:bg-green-400",
    icon: "text-green-500 dark:text-green-400",
    ring: "border-green-200 dark:border-green-900/60",
  },
  info: {
    bar: "bg-blue-500 dark:bg-blue-400",
    icon: "text-blue-500 dark:text-blue-400",
    ring: "border-blue-200 dark:border-blue-900/60",
  },
};

function KindIcon({ kind, className }: { kind: ToastKind; className: string }) {
  // Minimal inline SVGs — keep parity with the existing banner icon style.
  if (kind === "success") {
    return (
      <svg className={className} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
      </svg>
    );
  }
  if (kind === "info") {
    return (
      <svg className={className} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
      </svg>
    );
  }
  return (
    <svg className={className} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
      <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
    </svg>
  );
}

function ToastItem({ toast }: { toast: Toast }) {
  const dismiss = useToastStore((s) => s.dismiss);
  const styles = KIND_STYLES[toast.kind];

  useEffect(() => {
    if (toast.duration <= 0) return;
    const t = setTimeout(() => dismiss(toast.id), toast.duration);
    return () => clearTimeout(t);
  }, [toast.id, toast.duration, dismiss]);

  return (
    <div
      role="alert"
      className={`toast-in pointer-events-auto flex items-start gap-3 w-full max-w-md bg-white dark:bg-gray-800 border ${styles.ring} rounded-lg shadow-lg overflow-hidden`}
    >
      <div className={`w-1 self-stretch flex-shrink-0 ${styles.bar}`} />
      <KindIcon kind={toast.kind} className={`w-5 h-5 mt-2.5 flex-shrink-0 ${styles.icon}`} />
      <p className="flex-1 py-2.5 text-sm text-gray-800 dark:text-gray-100 break-words">{toast.message}</p>
      <button
        onClick={() => dismiss(toast.id)}
        className="p-2.5 text-gray-600 hover:text-gray-600 dark:text-gray-500 dark:hover:text-gray-300 transition-colors flex-shrink-0"
        aria-label="Dismiss notification"
      >
        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}

export default function ToastHost() {
  const toasts = useToastStore((s) => s.toasts);

  if (toasts.length === 0) return null;

  return (
    <div className="fixed top-4 left-1/2 -translate-x-1/2 z-[100] w-[calc(100%-2rem)] max-w-md flex flex-col gap-2 pointer-events-none">
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} />
      ))}
    </div>
  );
}
