/**
 * Modal dialog for choosing between downloading the converted
 * (browser-native) version or the original source file.
 */
import { Modal } from "../ui";

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
    <Modal onClose={onCancel} size="sm">
      <div className="p-6">
        <h3 className="text-fg text-lg font-semibold mb-2">Download</h3>
        <p className="text-fg-muted text-sm mb-5">
          This file was converted to a browser-friendly format during import.
          Which version would you like to download?
        </p>
        <div className="flex flex-col gap-3">
          <button
            onClick={onConvertedDownload}
            className="btn btn-primary btn-md w-full"
          >
            Converted file
          </button>
          <button
            onClick={onSourceDownload}
            className="btn btn-secondary btn-md w-full"
          >
            Original source file
          </button>
          <button
            onClick={onCancel}
            className="w-full px-4 py-2 text-fg-muted text-sm hover:text-fg transition-colors"
          >
            Cancel
          </button>
        </div>
      </div>
    </Modal>
  );
}
