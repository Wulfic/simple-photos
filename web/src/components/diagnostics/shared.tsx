import type React from "react";

/** Reusable collapsible section wrapper */
export function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
      <h2 className="text-base font-semibold text-gray-900 dark:text-white mb-4">
        {title}
      </h2>
      {children}
    </section>
  );
}

/** Small metric card used throughout the diagnostics dashboard */
export function StatCard({
  label,
  value,
  subtitle,
  color,
}: {
  label: string;
  value: string;
  subtitle?: string;
  color?: "green" | "yellow" | "red";
}) {
  const colorClass = color
    ? {
        green: "text-green-600 dark:text-green-400",
        yellow: "text-yellow-600 dark:text-yellow-400",
        red: "text-red-600 dark:text-red-400",
      }[color]
    : "text-gray-900 dark:text-white";

  return (
    <div className="bg-gray-50 dark:bg-gray-700/50 rounded-lg p-3">
      <p className="text-xs text-gray-500 dark:text-gray-400 mb-0.5">
        {label}
      </p>
      <p className={`text-sm font-bold ${colorClass}`}>{value}</p>
      {subtitle && (
        <p className="text-xs text-gray-400 dark:text-gray-500 mt-0.5">
          {subtitle}
        </p>
      )}
    </div>
  );
}

/** Compute an ISO date string N hours/days in the past */
export function getDateCutoff(range: string): string {
  const now = new Date();
  switch (range) {
    case "1h":
      return new Date(now.getTime() - 3600_000).toISOString();
    case "24h":
      return new Date(now.getTime() - 86400_000).toISOString();
    case "7d":
      return new Date(now.getTime() - 7 * 86400_000).toISOString();
    case "30d":
      return new Date(now.getTime() - 30 * 86400_000).toISOString();
    default:
      return "";
  }
}

/** Try to pretty-print a JSON string, falling back to the original */
export function tryPrettyJson(str: string): string {
  try {
    return JSON.stringify(JSON.parse(str), null, 2);
  } catch {
    return str;
  }
}
