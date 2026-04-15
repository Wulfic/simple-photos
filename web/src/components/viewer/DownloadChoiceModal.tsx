/**
 * Modal dialog for choosing between downloading the converted
 * (browser-native) version or the original source file.
 */

interface DownloadChoiceModalProps {
  onConvertedDownload: () => void;
  onSourceDownload: () => void;
  onCancel: () => void;
}

export default function DownloadChoiceModal({
  onConvertedDownload,
  onSourceDownload,
  onCancel,
}: DownloadChoiceModalProps) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={onCancel}
    >
      <div
        className="bg-gray-900 rounded-xl p-6 max-w-sm mx-4 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-white text-lg font-semibold mb-2">Download</h3>
        <p className="text-gray-400 text-sm mb-5">
          This file was converted to a browser-friendly format during import.
          Which version would you like to download?
        </p>
        <div className="flex flex-col gap-3">
          <button
            onClick={onConvertedDownload}
            className="w-full px-4 py-2.5 bg-blue-600 text-white text-sm font-medium rounded-lg hover:bg-blue-700 transition-colors"
          >
            Converted file
          </button>
          <button
            onClick={onSourceDownload}
            className="w-full px-4 py-2.5 bg-gray-700 text-white text-sm font-medium rounded-lg hover:bg-gray-600 transition-colors"
          >
            Original source file
          </button>
          <button
            onClick={onCancel}
            className="w-full px-4 py-2 text-gray-400 text-sm hover:text-white transition-colors"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
