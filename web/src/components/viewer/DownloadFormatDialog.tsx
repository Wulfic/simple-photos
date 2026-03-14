/**
 * Download format chooser dialog — lets the user pick between the original
 * file (e.g. HEIC, MKV) or the browser-native converted version (JPEG, MP4)
 * when both are available.
 */
import { getConvertedFormat, formatLabel } from "../../utils/mediaFormats";

interface DownloadFormatDialogProps {
  filename: string;
  onDownloadOriginal: () => void;
  onDownloadConverted: () => void;
  onCancel: () => void;
}

/**
 * Modal dialog asking the user whether to download the original file format
 * or the browser-compatible converted version.
 */
export default function DownloadFormatDialog({
  filename,
  onDownloadOriginal,
  onDownloadConverted,
  onCancel,
}: DownloadFormatDialogProps) {
  const originalExt = filename.split(".").pop()?.toLowerCase() ?? "";
  const convertedExt = getConvertedFormat(filename) ?? "";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70">
      <div className="bg-gray-900 rounded-2xl p-6 max-w-sm w-full mx-4 shadow-2xl border border-white/10">
        <h3 className="text-white text-lg font-semibold mb-1">Download Format</h3>
        <p className="text-gray-400 text-sm mb-5">
          This file has been converted for browser playback. Choose which version to download.
        </p>

        <div className="space-y-3">
          <button
            onClick={onDownloadOriginal}
            className="w-full flex items-center gap-3 px-4 py-3 rounded-xl bg-white/5 hover:bg-white/10 transition-colors text-left"
          >
            <div className="w-10 h-10 rounded-lg bg-blue-600/20 flex items-center justify-center flex-shrink-0">
              <span className="text-blue-400 text-xs font-bold">{formatLabel(originalExt)}</span>
            </div>
            <div>
              <div className="text-white text-sm font-medium">Original Format</div>
              <div className="text-gray-500 text-xs">.{originalExt} — preserves full quality</div>
            </div>
          </button>

          <button
            onClick={onDownloadConverted}
            className="w-full flex items-center gap-3 px-4 py-3 rounded-xl bg-white/5 hover:bg-white/10 transition-colors text-left"
          >
            <div className="w-10 h-10 rounded-lg bg-green-600/20 flex items-center justify-center flex-shrink-0">
              <span className="text-green-400 text-xs font-bold">{formatLabel(convertedExt)}</span>
            </div>
            <div>
              <div className="text-white text-sm font-medium">Converted Format</div>
              <div className="text-gray-500 text-xs">.{convertedExt} — compatible with most devices</div>
            </div>
          </button>
        </div>

        <button
          onClick={onCancel}
          className="w-full mt-4 px-4 py-2 text-gray-400 text-sm hover:text-white transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
