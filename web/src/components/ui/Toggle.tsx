/**
 * Toggle — accessible on/off switch (settings panels).
 *
 * Composes the `.toggle` / `.toggle-knob` recipes: a recessed track with a
 * raised knob, accent gradient when on. Requires an accessible `label`
 * (mapped to aria-label). Use for the bespoke switches that were previously
 * hand-rolled `<button role="switch">` blocks.
 */
import { forwardRef } from "react";
import { cn } from "./cn";

export interface ToggleProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "onChange"> {
  checked: boolean;
  /** Accessible name — rendered as aria-label (switches have no text). */
  label: string;
}

export const Toggle = forwardRef<HTMLButtonElement, ToggleProps>(function Toggle(
  { checked, label, className, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={cn("toggle", checked && "toggle-on", className)}
      {...rest}
    >
      <span
        className={cn("toggle-knob", checked ? "translate-x-6" : "translate-x-1")}
      />
    </button>
  );
});
