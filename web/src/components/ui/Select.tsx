/**
 * Select — native <select> styled to match the design system.
 *
 * Composes the `.select` recipe (the recessed `.input` look) and overlays a
 * custom chevron, since `appearance-none` strips the native arrow. Stays a
 * real <select> for free keyboard/accessibility/mobile behaviour — it is not a
 * custom listbox.
 *
 * Width is caller-controlled: inline toolbar filters size to their content
 * (default); form fields pass `fullWidth`. Extra classes via `className` go to
 * the <select> itself (e.g. `text-xs` for compact controls).
 */
import { forwardRef } from "react";
import { cn } from "./cn";

export interface SelectProps
  extends React.SelectHTMLAttributes<HTMLSelectElement> {
  /** Stretch to fill the container (form fields). Defaults to content width. */
  fullWidth?: boolean;
  /** Classes for the positioning wrapper (margins, explicit min-width, etc.). */
  wrapperClassName?: string;
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(function Select(
  { className, children, fullWidth = false, wrapperClassName, ...rest },
  ref,
) {
  return (
    <div
      className={cn(
        "relative",
        fullWidth ? "block w-full" : "inline-block",
        wrapperClassName,
      )}
    >
      <select
        ref={ref}
        className={cn("select", fullWidth && "w-full", className)}
        {...rest}
      >
        {children}
      </select>
      <svg
        aria-hidden="true"
        className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-fg-subtle"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
        strokeWidth={2}
      >
        <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
      </svg>
    </div>
  );
});
