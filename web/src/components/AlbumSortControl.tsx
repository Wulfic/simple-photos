/**
 * AlbumSortControl (#52) — the Date / Name sort toggle in the album-detail
 * header. Clicking the active field flips its direction; clicking the other
 * field switches to it. Purely presentational: all state lives in
 * {@link useAlbumSort}.
 */
import type { AlbumSort, SortField } from "../gallery/albumSort";

const FIELDS: { field: SortField; label: string }[] = [
  { field: "date", label: "Date" },
  { field: "name", label: "Name" },
];

export default function AlbumSortControl({
  sort,
  onSelectField,
}: {
  /** Concrete sort to reflect (the chosen one, or the default). */
  sort: AlbumSort;
  onSelectField: (field: SortField) => void;
}) {
  return (
    <div
      className="flex items-center gap-1 rounded-lg border border-edge-strong p-0.5"
      role="group"
      aria-label="Sort album"
    >
      {FIELDS.map(({ field, label }) => {
        const active = sort.field === field;
        const arrow = sort.dir === "asc" ? "↑" : "↓";
        return (
          <button
            key={field}
            type="button"
            onClick={() => onSelectField(field)}
            aria-pressed={active}
            title={
              active
                ? `Sorted by ${label}, ${sort.dir === "asc" ? "ascending" : "descending"} — click to reverse`
                : `Sort by ${label}`
            }
            className={`flex items-center gap-1 rounded-md px-2.5 py-1 text-sm font-medium transition-colors ${
              active
                ? "bg-accent-600 text-white dark:bg-accent-500"
                : "text-fg-muted hover:text-fg hover:bg-surface"
            }`}
          >
            {label}
            {active && (
              <span aria-hidden className="text-xs leading-none">
                {arrow}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
