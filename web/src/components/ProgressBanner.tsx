/**
 * ProgressBanner — shared floating progress card used by every global
 * background-task banner (encryption, conversion, AI, geo, save-copy).
 *
 * Unifies what were five near-identical hand-rolled cards onto the `#14`
 * facelift design language: the `card shadow-card-hover` surface, a tinted
 * spinner, an optional ETA, an optional progress bar, and the standard
 * `icon-btn` dismiss button. Banners stack at the bottom-right via the
 * `position` prop (a Tailwind bottom-* utility).
 *
 * Pass `pct` for a determinate bar; omit it for a spinner-only notice. Pass
 * `onDismiss` to render the dismiss button; omit it for non-dismissible ones.
 */

export type ProgressTone = "accent" | "orange" | "purple" | "emerald";

interface ToneClasses {
  /** Spinner top-border color (the spinning segment). */
  spinner: string;
  /** Progress-bar fill color. */
  bar: string;
}

// Full literal class strings so Tailwind's content scan keeps them — never
// build these by interpolation.
const TONE: Record<ProgressTone, ToneClasses> = {
  accent: {
    spinner: "border-t-accent-500 dark:border-t-accent-400",
    bar: "bg-accent-500 dark:bg-accent-400",
  },
  orange: {
    spinner: "border-t-orange-500 dark:border-t-orange-400",
    bar: "bg-orange-500 dark:bg-orange-400",
  },
  purple: {
    spinner: "border-t-purple-500 dark:border-t-purple-400",
    bar: "bg-purple-500 dark:bg-purple-400",
  },
  emerald: {
    spinner: "border-t-emerald-500 dark:border-t-emerald-400",
    bar: "bg-emerald-500 dark:bg-emerald-400",
  },
};

export interface ProgressBannerProps {
  /** Tailwind bottom-* utility controlling the stack slot, e.g. "bottom-6". */
  position: string;
  tone?: ProgressTone;
  /** Primary status line. */
  label: string;
  /** Optional secondary description line (smaller, muted). */
  description?: string;
  /** Optional ETA string; rendered as "{eta} remaining". */
  eta?: string | null;
  /** 0–100 for a determinate bar. Omit for a spinner-only banner. */
  pct?: number;
  /** When provided, renders a dismiss button wired to this handler. */
  onDismiss?: () => void;
}

export function ProgressBanner({
  position,
  tone = "accent",
  label,
  description,
  eta,
  pct,
  onDismiss,
}: ProgressBannerProps) {
  const t = TONE[tone];

  return (
    <div className={`fixed ${position} left-4 right-4 z-50 pointer-events-none`}>
      <div className="card shadow-card-hover pointer-events-auto max-w-md mx-auto flex items-center gap-3 px-4 py-3">
        <div
          className={`w-5 h-5 border-2 border-edge-strong ${t.spinner} rounded-full animate-spin flex-shrink-0`}
        />
        <div className="flex-1 min-w-0">
          <div className="flex items-center justify-between">
            <p className="text-sm font-medium text-fg-muted">{label}</p>
            {eta && (
              <span className="text-xs tabular-nums text-fg-muted ml-2 flex-shrink-0">
                {eta} remaining
              </span>
            )}
          </div>
          {description && (
            <p className="text-xs text-fg-muted mt-0.5">{description}</p>
          )}
          {pct !== undefined && (
            <div className="mt-1.5 h-1.5 bg-edge rounded-full overflow-hidden">
              <div
                className={`h-full ${t.bar} rounded-full transition-all duration-500`}
                style={{ width: `${pct}%` }}
              />
            </div>
          )}
        </div>
        {onDismiss && (
          <button
            onClick={onDismiss}
            className="icon-btn p-1 flex-shrink-0"
            aria-label="Dismiss"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        )}
      </div>
    </div>
  );
}
