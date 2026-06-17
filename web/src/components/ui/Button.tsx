/**
 * Button — the single button primitive for the app (#14 facelift).
 *
 * Composes the semantic `.btn`/`.btn-{variant}`/`.btn-{size}` recipes defined
 * in index.css so depth, hover/active/focus states and spacing are consistent
 * everywhere. Spreads all native <button> props; `className` is appended last
 * so call sites can still tweak layout (e.g. `w-full`, `mt-2`).
 *
 * Defaults `type="button"` (the safe React default — a bare <button> inside a
 * <form> submits). Pass `type="submit"` explicitly for form submits.
 */
import { forwardRef } from "react";
import { cn } from "./cn";

export type ButtonVariant =
  | "primary"
  | "secondary"
  | "ghost"
  | "danger"
  | "success";
export type ButtonSize = "sm" | "md" | "lg";

const VARIANT_CLASS: Record<ButtonVariant, string> = {
  primary: "btn-primary",
  secondary: "btn-secondary",
  ghost: "btn-ghost",
  danger: "btn-danger",
  success: "btn-success",
};

const SIZE_CLASS: Record<ButtonSize, string> = {
  sm: "btn-sm",
  md: "btn-md",
  lg: "btn-lg",
};

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "primary", size = "md", className, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      className={cn("btn", VARIANT_CLASS[variant], SIZE_CLASS[size], className)}
      {...rest}
    />
  );
});
