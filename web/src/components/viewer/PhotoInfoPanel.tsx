/** Slide-in panel showing photo metadata (dimensions, size, camera, dates). */
import { formatBytes } from "../../utils/formatters";

interface PhotoInfoPanelProps {
  show: boolean;
  onClose: () => void;
  photoInfo: {
    filename: string;
    mimeType: string;
    width?: number;
    height?: number;
    takenAt?: string | null;
    sizeBytes?: number;
    latitude?: number | null;
    longitude?: number | null;
    createdAt?: string;
    durationSecs?: number | null;
    cameraModel?: string | null;
    albumNames?: string[];
  } | null;
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-4">
      <span className="text-gray-400 shrink-0">{label}</span>
      <span className="text-white text-right break-all">{value}</span>
    </div>
  );
}

export default function PhotoInfoPanel({ show, onClose, photoInfo }: PhotoInfoPanelProps) {
  return (
    <div
      className={`fixed bottom-0 left-0 right-0 z-40 transition-transform duration-300 ease-out ${
        show ? "translate-y-0" : "translate-y-full"
      }`}
    >
      <div className="bg-gray-900/95 backdrop-blur-sm border-t border-white/10 rounded-t-2xl max-h-[60vh] overflow-y-auto">
        <div className="flex items-center justify-between px-5 py-3 border-b border-white/10">
          <h3 className="text-white text-sm font-semibold">Photo Details</h3>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-white transition-colors"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div className="px-5 py-4 space-y-3 text-sm">
          {photoInfo ? (
            <>
              <InfoRow label="Filename" value={photoInfo.filename} />
              <InfoRow label="Type" value={photoInfo.mimeType} />
              {photoInfo.width != null && photoInfo.height != null && photoInfo.width > 0 && photoInfo.height > 0 && (
                <InfoRow label="Dimensions" value={`${photoInfo.width} × ${photoInfo.height}`} />
              )}
              {photoInfo.sizeBytes != null && photoInfo.sizeBytes > 0 && (
                <InfoRow label="Size" value={formatBytes(photoInfo.sizeBytes)} />
              )}
              {photoInfo.takenAt && (
                <InfoRow label="Taken" value={new Date(photoInfo.takenAt).toLocaleString()} />
              )}
              {photoInfo.createdAt && (
                <InfoRow label="Uploaded" value={new Date(photoInfo.createdAt).toLocaleString()} />
              )}
              {photoInfo.durationSecs != null && (
                <InfoRow label="Duration" value={`${photoInfo.durationSecs.toFixed(1)}s`} />
              )}
              {photoInfo.cameraModel && (
                <InfoRow label="Device" value={photoInfo.cameraModel} />
              )}
              {photoInfo.latitude != null && photoInfo.longitude != null && (
                <div className="flex justify-between items-start">
                  <span className="text-gray-400 shrink-0 w-24">Location</span>
                  <a
                    href={`https://www.google.com/maps?q=${photoInfo.latitude},${photoInfo.longitude}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-400 hover:text-blue-300 text-right break-all"
                  >
                    {photoInfo.latitude.toFixed(5)}, {photoInfo.longitude.toFixed(5)} ↗
                  </a>
                </div>
              )}
              {photoInfo.albumNames && photoInfo.albumNames.length > 0 && (
                <InfoRow label="Albums" value={photoInfo.albumNames.join(", ")} />
              )}
            </>
          ) : (
            <p className="text-gray-400 italic">No metadata available</p>
          )}
        </div>
      </div>
    </div>
  );
}
