/**
 * Skeleton — content-loading placeholder primitive.
 *
 * Composes the `.skeleton` recipe (neutral gray block, dark-mode aware) plus
 * the reduced-motion-gated `.skeleton-shimmer` sweep. Reduced-motion users get
 * a flat static block (no animation) automatically via the media query in
 * index.css.
 *
 * Variants control the shape:
 *   - `text`   — short rounded bar, default height ~1em (use for lines of text).
 *   - `rect`   — rounded rectangle (cards, thumbnails, buttons).
 *   - `circle` — perfect circle (avatars, icons); pass a single `size`.
 *
 * Size is fully configurable via `width`/`height` (number → px, or any CSS
 * length string). Prefer composing several <Skeleton>s into a page-specific
 * skeleton that mirrors the real layout rather than dropping generic boxes.
 */
import { cn } from "./cn";

export type SkeletonVariant = "text" | "rect" | "circle";

export interface SkeletonProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, "children"> {
  variant?: SkeletonVariant;
  /** Width — number (px) or CSS length. Defaults to 100% (text/rect). */
  width?: number | string;
  /** Height — number (px) or CSS length. Defaults per-variant. */
  height?: number | string;
  /** Convenience for circle/square: sets both width and height. */
  size?: number | string;
  /** Disable the shimmer sweep (e.g. dense lists where motion is noisy). */
  noShimmer?: boolean;
}

function toLength(value: number | string | undefined): string | undefined {
  if (value === undefined) return undefined;
  return typeof value === "number" ? `${value}px` : value;
}

const VARIANT_RADIUS: Record<SkeletonVariant, string> = {
  text: "rounded",
  rect: "rounded-lg",
  circle: "rounded-full",
};

export function Skeleton({
  variant = "rect",
  width,
  height,
  size,
  noShimmer = false,
  className,
  style,
  ...rest
}: SkeletonProps) {
  const resolvedWidth = toLength(size ?? width);
  const resolvedHeight = toLength(size ?? height);

  // Sensible per-variant defaults so a bare <Skeleton variant="text" /> looks
  // right without callers specifying dimensions.
  const fallbackHeight =
    variant === "text" ? "1em" : variant === "circle" ? "2.5rem" : "100%";
  const fallbackWidth = variant === "circle" ? "2.5rem" : "100%";

  return (
    <div
      aria-hidden="true"
      className={cn(
        "skeleton",
        !noShimmer && "skeleton-shimmer",
        VARIANT_RADIUS[variant],
        className,
      )}
      style={{
        width: resolvedWidth ?? fallbackWidth,
        height: resolvedHeight ?? fallbackHeight,
        ...style,
      }}
      {...rest}
    />
  );
}
