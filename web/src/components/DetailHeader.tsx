/**
 * DetailHeader — the shared "sub-header" row used by the album / smart-album
 * detail pages: a back arrow, a (truncating) title, an optional item count, an
 * optional inline trailing slot (e.g. slideshow triggers), and an optional
 * right-aligned action group. Each detail page used to hand-roll this same
 * flexbox scaffold.
 *
 * Not used by the full-screen viewer top bar (ViewerTopBar) or the visually
 * distinct Secure-gallery / Diagnostics headers — those are intentionally their
 * own thing.
 */
import { type ReactNode } from "react";
import { useAppNavigate } from "../hooks/useAppNavigate";
import AppIcon from "./AppIcon";

interface DetailHeaderProps {
  /** Back-arrow destination. */
  backTo: string;
  /** Back-arrow tooltip (`title`). */
  backTitle?: string;
  title: ReactNode;
  /** Extra classes for the title (e.g. "capitalize"). */
  titleClassName?: string;
  /** Item-count label rendered after the title (e.g. "12 items"). */
  count?: ReactNode;
  /** Inline content after the count, inside the left group (e.g. SlideshowTriggers). */
  children?: ReactNode;
  /** Right-aligned action group; its presence right-justifies the row. */
  actions?: ReactNode;
  /** Outer wrapper classes (margins etc.). Defaults to "mb-4". */
  className?: string;
}

export default function DetailHeader({
  backTo,
  backTitle,
  title,
  titleClassName,
  count,
  children,
  actions,
  className = "mb-4",
}: DetailHeaderProps) {
  const navigate = useAppNavigate();
  return (
    <div className={`flex items-center ${actions ? "justify-between " : ""}gap-3 ${className}`}>
      <div className="flex items-center gap-3 min-w-0">
        <button
          onClick={() => navigate(backTo)}
          className="text-fg-muted hover:text-fg transition-colors shrink-0"
          title={backTitle}
        >
          <AppIcon name="back-arrow" size="w-5 h-5" />
        </button>
        <h2 className={`text-xl font-semibold truncate ${titleClassName ?? ""}`}>{title}</h2>
        {count != null && <span className="text-fg-muted text-sm shrink-0">{count}</span>}
        {children}
      </div>
      {actions && <div className="flex items-center gap-2 shrink-0">{actions}</div>}
    </div>
  );
}
