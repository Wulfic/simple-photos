/**
 * IconButton — square, icon-only button (toolbars, close buttons, etc.).
 *
 * Composes the `.icon-btn` recipe. Requires an accessible `label` (mapped to
 * aria-label) so icon-only controls are never unlabeled.
 */
import { forwardRef } from "react";
import { cn } from "./cn";

export interface IconButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** Accessible name — rendered as aria-label (icon buttons have no text). */
  label: string;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(
  function IconButton({ label, className, type = "button", ...rest }, ref) {
    return (
      <button
        ref={ref}
        type={type}
        aria-label={label}
        className={cn("icon-btn", className)}
        {...rest}
      />
    );
  },
);
