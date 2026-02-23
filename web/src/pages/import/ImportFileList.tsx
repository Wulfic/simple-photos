import { formatBytes } from "../../utils/media";
import type { ImportItem } from "../../utils/importTypes";

interface ImportFileListProps {
  items: ImportItem[];
  removeItem: (index: number) => void;
}

export default function ImportFileList({ items, removeItem }: ImportFileListProps) {
  if (items.length === 0) return null;

  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg shadow overflow-hidden">
      <table className="w-full text-sm">
        <thead className="bg-gray-50 dark:bg-gray-900 border-b dark:border-gray-700">
          <tr>
            <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">
              File
            </th>
            <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">
              Size
            </th>
            <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">
              Type
            </th>
            <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">
              Status
            </th>
            <th className="px-4 py-2"></th>
          </tr>
        </thead>
        <tbody className="divide-y divide-gray-100 dark:divide-gray-700">
          {items.map((item, i) => (
            <tr
              key={`${item.name}-${i}`}
              className="hover:bg-gray-50 dark:hover:bg-gray-700/50"
            >
              <td className="px-4 py-2.5">
                <div className="flex items-center gap-2">
                  <span className="text-base">
                    {item.mimeType?.startsWith("video/") ? "\uD83C\uDFAC" : "\uD83D\uDDBC\uFE0F"}
                  </span>
                  <span
                    className="truncate max-w-[250px] dark:text-gray-200"
                    title={item.name}
                  >
                    {item.name}
                  </span>
                </div>
              </td>
              <td className="px-4 py-2.5 text-gray-500 dark:text-gray-400">
                {formatBytes(item.size)}
              </td>
              <td className="px-4 py-2.5 text-gray-500 dark:text-gray-400 text-xs">
                {item.mimeType?.split("/")[1]?.toUpperCase() || "\u2014"}
              </td>
              <td className="px-4 py-2.5">
                {item.status === "pending" && (
                  <span className="text-gray-500 dark:text-gray-400 text-xs">
                    Pending
                  </span>
                )}
                {item.status === "uploading" && (
                  <span className="text-blue-600 text-xs flex items-center gap-1">
                    <div className="w-3 h-3 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
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
                      className="text-gray-400 hover:text-red-500 dark:hover:text-red-400 text-xs"
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
