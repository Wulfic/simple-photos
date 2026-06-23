/**
 * Shared modal primitive. Replaces the 16 hand-rolled `fixed inset-0` overlays
 * scattered across the app — each of which re-implemented (inconsistently) the
 * backdrop, panel, close button, Escape handling and body-scroll lock.
 *
 * Behaviour, now uniform for every modal:
 *   • semantic-token panel (`card shadow-pop`) — no more hardcoded bg-gray-*
 *     that broke light mode
 *   • click-outside-to-close (opt-out via `closeOnBackdrop={false}`)
 *   • Escape-to-close (opt-out via `closeOnEscape={false}`)
 *   • body-scroll lock while open
 *   • optional standard header (icon + title + close button) via `title`
 *
 * Body padding is intentionally the caller's responsibility — modals vary from
 * a single padded prompt to a flex-column with a scrolling list — so wrap your
 * content in whatever layout you need.
 */
import { useEffect, useRef, type ReactNode } from "react";
import { cn } from "./cn";

const SIZES = {
  sm: "max-w-sm",
  md: "max-w-md",
  lg: "max-w-lg",
  xl: "max-w-xl",
} as const;

export interface ModalProps {
  /** Visibility. Defaults to `true` so callers may also just mount conditionally. */
  open?: boolean;
  onClose: () => void;
  /** When set, renders the standard header row (optional icon + title + close). */
  title?: ReactNode;
  /** Leading icon for the standard header. Ignored when `title` is unset. */
  titleIcon?: ReactNode;
  children: ReactNode;
  size?: keyof typeof SIZES;
  /** Click on the backdrop closes the modal. Default `true`. */
  closeOnBackdrop?: boolean;
  /** Escape key closes the modal. Default `true`. */
  closeOnEscape?: boolean;
  /** Extra panel classes (e.g. `max-h-[80vh] flex flex-col`). */
  panelClassName?: string;
  /** Stacking context. Default `z-50`. */
  zClassName?: string;
  /** Accessible label when no string `title` is rendered. */
  ariaLabel?: string;
  /** Test hook applied to the dialog panel. */
  testId?: string;
}

export function Modal({
  open = true,
  onClose,
  title,
  titleIcon,
  children,
  size = "sm",
  closeOnBackdrop = true,
  closeOnEscape = true,
  panelClassName,
  zClassName = "z-50",
  ariaLabel,
  testId,
}: ModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);

  // Escape-to-close.
  useEffect(() => {
    if (!open || !closeOnEscape) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, closeOnEscape, onClose]);

  // Body-scroll lock while open.
  useEffect(() => {
    if (!open) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, [open]);

  if (!open) return null;

  return (
    <div
      className={cn(
        "fixed inset-0 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4",
        zClassName,
      )}
      onClick={closeOnBackdrop ? onClose : undefined}
      role="dialog"
      aria-modal="true"
      aria-label={typeof title === "string" ? title : ariaLabel}
    >
      <div
        ref={panelRef}
        className={cn("card shadow-pop w-full", SIZES[size], panelClassName)}
        onClick={(e) => e.stopPropagation()}
        data-testid={testId}
      >
        {title != null && (
          <div className="flex items-center gap-2 px-4 py-3 border-b border-edge">
            {titleIcon}
            <h3 className="text-base font-semibold text-fg flex-1 min-w-0">{title}</h3>
            <button onClick={onClose} aria-label="Close" className="icon-btn -mr-1.5">
              <svg
                className="w-5 h-5"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                strokeWidth={2}
              >
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        )}
        {children}
      </div>
    </div>
  );
}
