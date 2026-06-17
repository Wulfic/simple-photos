/**
 * Card — the standard elevated surface (settings sections, list rows, tiles).
 *
 * Composes the `.card` recipe (white/gray-900 surface, hairline border, soft
 * layered shadow). Set `interactive` for clickable cards that should lift on
 * hover. Renders a <div> by default; pass `as` to use another element (e.g.
 * "section", "li", "button") while keeping the card styling.
 */
import { cn } from "./cn";

type CardElement = "div" | "section" | "article" | "li" | "button";

export interface CardProps extends React.HTMLAttributes<HTMLElement> {
  as?: CardElement;
  interactive?: boolean;
}

export function Card({
  as = "div",
  interactive = false,
  className,
  children,
  ...rest
}: CardProps) {
  const Tag = as;
  return (
    <Tag
      className={cn("card", interactive && "card-interactive", className)}
      {...rest}
    >
      {children}
    </Tag>
  );
}
