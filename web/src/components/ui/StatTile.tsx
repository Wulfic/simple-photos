/**
 * StatTile — small metric tile (settings / diagnostics panels).
 *
 * Composes the `.stat-tile` recipe (raised gradient surface + hairline + soft
 * shadow). `tone` colors the figure; defaults to the neutral text color.
 */
import { cn } from "./cn";

export type StatTone =
  | "accent"
  | "amber"
  | "green"
  | "purple"
  | "orange"
  | "red"
  | "neutral";

const TONE_CLASS: Record<StatTone, string> = {
  accent: "text-accent-600 dark:text-accent-400",
  amber: "text-amber-600 dark:text-amber-400",
  green: "text-green-600 dark:text-green-400",
  purple: "text-purple-600 dark:text-purple-400",
  orange: "text-orange-600 dark:text-orange-400",
  red: "text-red-600 dark:text-red-400",
  neutral: "text-gray-900 dark:text-gray-100",
};

export interface StatTileProps extends React.HTMLAttributes<HTMLDivElement> {
  value: React.ReactNode;
  label: React.ReactNode;
  tone?: StatTone;
}

export function StatTile({
  value,
  label,
  tone = "neutral",
  className,
  ...rest
}: StatTileProps) {
  return (
    <div className={cn("stat-tile", className)} {...rest}>
      <p className={cn("text-xl font-bold", TONE_CLASS[tone])}>{value}</p>
      <p className="text-xs text-gray-700 dark:text-gray-400">{label}</p>
    </div>
  );
}
