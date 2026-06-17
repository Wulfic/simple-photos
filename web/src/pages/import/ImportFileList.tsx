/** File list component for the Import page — shows queued files with sizes and status icons. */
import { formatBytes } from "../../utils/formatters";
import type { ImportItem } from "../../utils/importTypes";

interface ImportFileListProps {
  items: ImportItem[];
  removeItem: (index: number) => void;
}

export default function ImportFileList({ items, removeItem }: ImportFileListProps) {
  if (items.length === 0) return null;

  return (
    <div className="card overflow-hidden">
      <table className="w-full text-sm">
        <thead className="bg-canvas border-b dark:border-gray-700">
          <tr>
            <th className="text-left px-4 py-2 font-medium text-fg-muted">
              File
            </th>
            <th className="text-left px-4 py-2 font-medium text-fg-muted">
              Size
            </th>
            <th className="text-left px-4 py-2 font-medium text-fg-muted">
              Type
            </th>
            <th className="text-left px-4 py-2 font-medium text-fg-muted">
              Status
            </th>
            <th className="px-4 py-2"></th>
          </tr>
        </thead>
        <tbody className="divide-y divide-edge">
          {items.map((item, i) => (
            <tr
              key={`${item.name}-${i}`}
              className="hover:bg-surface-sunken dark:hover:bg-white/10/50"
            >
              <td className="px-4 py-2.5">
                <div className="flex items-center gap-2">
                  <span className="text-base">
                    {item.mimeType?.startsWith("video/") ? "\uD83C\uDFAC" : item.mimeType?.startsWith("audio/") ? "\uD83C\uDFB5" : "\uD83D\uDDBC\uFE0F"}
                  </span>
                  <span
                    className="truncate max-w-[250px] dark:text-gray-200"
                    title={item.name}
                  >
                    {item.name}
                  </span>
                </div>
              </td>
              <td className="px-4 py-2.5 text-fg-muted">
                {formatBytes(item.size)}
              </td>
              <td className="px-4 py-2.5 text-fg-muted text-xs">
                {item.mimeType?.split("/")[1]?.toUpperCase() || "\u2014"}
              </td>
              <td className="px-4 py-2.5">
                {item.status === "pending" && (
                  <span className="text-fg-muted text-xs">
                    Pending
                  </span>
                )}
                {item.status === "uploading" && (
                  <span className="text-accent-600 text-xs flex items-center gap-1">
                    <div className="w-3 h-3 border-2 border-accent-600 border-t-transparent rounded-full animate-spin" />
                    Importing
                  </span>
                )}
                {item.status === "done" && (
                  <span className="text-green-600 dark:text-green-400 text-xs">
                    {"\u2713"} Done
                  </span>
                )}
                {item.status === "error" && (
                  <span
                    className="text-red-600 dark:text-red-400 text-xs cursor-help"
                    title={item.error}
                  >
                    {"\u2717"} {item.error || "Error"}
                  </span>
                )}
              </td>
              <td className="px-4 py-2.5">
                {item.status !== "uploading" &&
                  item.status !== "done" && (
                    <button
                      onClick={() => removeItem(i)}
                      className="text-fg-muted hover:text-red-500 dark:hover:text-red-400 text-xs"
                    >
                      {"\u2715"}
                    </button>
                  )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
