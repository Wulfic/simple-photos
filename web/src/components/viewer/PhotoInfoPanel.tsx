/** Slide-in panel showing photo metadata with inline edit mode. */
import { useState, useEffect, useCallback } from "react";
import { formatBytes } from "../../utils/formatters";
import { metadataApi, type FullMetadataResponse, type MetadataUpdateRequest } from "../../api/metadata";

interface PhotoInfoPanelProps {
  show: boolean;
  onClose: () => void;
  photoId?: string;
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

function EditRow({ label, value, onChange, placeholder, type }: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
}) {
  return (
    <div className="flex justify-between items-center gap-4">
      <span className="text-gray-400 shrink-0 text-xs">{label}</span>
      <input
        type={type ?? "text"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="bg-gray-800 text-white text-xs px-2 py-1 rounded border border-white/10 w-48 text-right"
      />
    </div>
  );
}

export default function PhotoInfoPanel({ show, onClose, photoId, photoInfo }: PhotoInfoPanelProps) {
  const [editing, setEditing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fullMeta, setFullMeta] = useState<FullMetadataResponse | null>(null);
  const [showExif, setShowExif] = useState(false);

  // Edit form state — basic
  const [editFilename, setEditFilename] = useState("");
  const [editTakenAt, setEditTakenAt] = useState("");
  const [editLat, setEditLat] = useState("");
  const [editLon, setEditLon] = useState("");
  const [editCamera, setEditCamera] = useState("");
  // Edit form state — extended EXIF
  const [editCameraMake, setEditCameraMake] = useState("");
  const [editLens, setEditLens] = useState("");
  const [editIso, setEditIso] = useState("");
  const [editFNumber, setEditFNumber] = useState("");
  const [editExposureTime, setEditExposureTime] = useState("");
  const [editFocalLength, setEditFocalLength] = useState("");
  const [editFlash, setEditFlash] = useState("");
  const [editWhiteBalance, setEditWhiteBalance] = useState("");
  const [editExposureProgram, setEditExposureProgram] = useState("");
  const [editMeteringMode, setEditMeteringMode] = useState("");
  const [editOrientation, setEditOrientation] = useState("");
  const [editSoftware, setEditSoftware] = useState("");
  const [editArtist, setEditArtist] = useState("");
  const [editCopyright, setEditCopyright] = useState("");
  const [editDescription, setEditDescription] = useState("");
  const [editUserComment, setEditUserComment] = useState("");
  const [editColorSpace, setEditColorSpace] = useState("");
  const [editExposureBias, setEditExposureBias] = useState("");
  const [editSceneType, setEditSceneType] = useState("");
  const [editDigitalZoom, setEditDigitalZoom] = useState("");

  const loadFullMetadata = useCallback(async () => {
    if (!photoId) return;
    try {
      const meta = await metadataApi.getFull(photoId);
      setFullMeta(meta);
    } catch {
      // Silently fail — full EXIF is optional display
    }
  }, [photoId]);

  useEffect(() => {
    if (show && photoId) {
      loadFullMetadata();
    }
    if (!show) {
      setEditing(false);
      setShowExif(false);
      setError(null);
    }
  }, [show, photoId, loadFullMetadata]);

  const startEdit = () => {
    setEditFilename(photoInfo?.filename ?? "");
    setEditTakenAt(photoInfo?.takenAt ?? "");
    setEditLat(photoInfo?.latitude != null ? String(photoInfo.latitude) : "");
    setEditLon(photoInfo?.longitude != null ? String(photoInfo.longitude) : "");
    setEditCamera(photoInfo?.cameraModel ?? "");
    // Populate extended fields from fullMeta
    setEditCameraMake(fullMeta?.camera_make ?? "");
    setEditLens(fullMeta?.lens_model ?? "");
    setEditIso(fullMeta?.iso_speed != null ? String(fullMeta.iso_speed) : "");
    setEditFNumber(fullMeta?.f_number != null ? String(fullMeta.f_number) : "");
    setEditExposureTime(fullMeta?.exposure_time ?? "");
    setEditFocalLength(fullMeta?.focal_length != null ? String(fullMeta.focal_length) : "");
    setEditFlash(fullMeta?.flash ?? "");
    setEditWhiteBalance(fullMeta?.white_balance ?? "");
    setEditExposureProgram(fullMeta?.exposure_program ?? "");
    setEditMeteringMode(fullMeta?.metering_mode ?? "");
    setEditOrientation(fullMeta?.orientation != null ? String(fullMeta.orientation) : "");
    setEditSoftware(fullMeta?.software ?? "");
    setEditArtist(fullMeta?.artist ?? "");
    setEditCopyright(fullMeta?.copyright ?? "");
    setEditDescription(fullMeta?.description ?? "");
    setEditUserComment(fullMeta?.user_comment ?? "");
    setEditColorSpace(fullMeta?.color_space ?? "");
    setEditExposureBias(fullMeta?.exposure_bias != null ? String(fullMeta.exposure_bias) : "");
    setEditSceneType(fullMeta?.scene_type ?? "");
    setEditDigitalZoom(fullMeta?.digital_zoom != null ? String(fullMeta.digital_zoom) : "");
    setEditing(true);
    setError(null);
  };

  const cancelEdit = () => {
    setEditing(false);
    setError(null);
  };

  const saveEdit = async () => {
    if (!photoId) return;
    setSaving(true);
    setError(null);
    try {
      const patch: MetadataUpdateRequest = {};
      if (editFilename !== (photoInfo?.filename ?? "")) patch.filename = editFilename;
      if (editTakenAt !== (photoInfo?.takenAt ?? "")) patch.taken_at = editTakenAt || undefined;
      if (editCamera !== (photoInfo?.cameraModel ?? "")) patch.camera_model = editCamera;

      const hadGps = photoInfo?.latitude != null;
      const hasGps = editLat.trim() !== "" && editLon.trim() !== "";

      if (hadGps && !hasGps) {
        patch.clear_gps = true;
      } else if (hasGps) {
        const lat = parseFloat(editLat);
        const lon = parseFloat(editLon);
        if (isNaN(lat) || isNaN(lon)) {
          setError("Invalid coordinate values");
          setSaving(false);
          return;
        }
        if (lat < -90 || lat > 90) {
          setError("Latitude must be between -90 and 90");
          setSaving(false);
          return;
        }
        if (lon < -180 || lon > 180) {
          setError("Longitude must be between -180 and 180");
          setSaving(false);
          return;
        }
        const origLat = photoInfo?.latitude;
        const origLon = photoInfo?.longitude;
        if (lat !== origLat || lon !== origLon) {
          patch.latitude = lat;
          patch.longitude = lon;
        }
      }

      // Helper to set string field only if changed
      const diffStr = (editVal: string, origVal: string | null | undefined, key: keyof MetadataUpdateRequest) => {
        if (editVal !== (origVal ?? "")) {
          (patch as Record<string, unknown>)[key] = editVal || undefined;
        }
      };
      // Helper to set numeric field only if changed
      const diffNum = (editVal: string, origVal: number | null | undefined, key: keyof MetadataUpdateRequest) => {
        const origStr = origVal != null ? String(origVal) : "";
        if (editVal !== origStr) {
          if (editVal.trim() === "") {
            // Clear the field — send empty string to let the server handle it
          } else {
            const num = parseFloat(editVal);
            if (!isNaN(num)) (patch as Record<string, unknown>)[key] = num;
          }
        }
      };

      diffStr(editCameraMake, fullMeta?.camera_make, "camera_make");
      diffStr(editLens, fullMeta?.lens_model, "lens_model");
      diffNum(editIso, fullMeta?.iso_speed, "iso_speed");
      diffNum(editFNumber, fullMeta?.f_number, "f_number");
      diffStr(editExposureTime, fullMeta?.exposure_time, "exposure_time");
      diffNum(editFocalLength, fullMeta?.focal_length, "focal_length");
      diffStr(editFlash, fullMeta?.flash, "flash");
      diffStr(editWhiteBalance, fullMeta?.white_balance, "white_balance");
      diffStr(editExposureProgram, fullMeta?.exposure_program, "exposure_program");
      diffStr(editMeteringMode, fullMeta?.metering_mode, "metering_mode");
      diffNum(editOrientation, fullMeta?.orientation, "orientation");
      diffStr(editSoftware, fullMeta?.software, "software");
      diffStr(editArtist, fullMeta?.artist, "artist");
      diffStr(editCopyright, fullMeta?.copyright, "copyright");
      diffStr(editDescription, fullMeta?.description, "description");
      diffStr(editUserComment, fullMeta?.user_comment, "user_comment");
      diffStr(editColorSpace, fullMeta?.color_space, "color_space");
      diffNum(editExposureBias, fullMeta?.exposure_bias, "exposure_bias");
      diffStr(editSceneType, fullMeta?.scene_type, "scene_type");
      diffNum(editDigitalZoom, fullMeta?.digital_zoom, "digital_zoom");

      if (Object.keys(patch).length > 0) {
        await metadataApi.update(photoId, patch);
        await loadFullMetadata();
      }
      setEditing(false);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  const writeExif = async () => {
    if (!photoId) return;
    setSaving(true);
    setError(null);
    try {
      await metadataApi.writeExif(photoId);
      await loadFullMetadata();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Failed to write EXIF");
    } finally {
      setSaving(false);
    }
  };

  const isJpegOrTiff = photoInfo?.mimeType &&
    (photoInfo.mimeType.includes("jpeg") || photoInfo.mimeType.includes("jpg") || photoInfo.mimeType.includes("tiff"));

  return (
    <div
      className={`fixed bottom-0 left-0 right-0 z-40 transition-transform duration-300 ease-out ${
        show ? "translate-y-0" : "translate-y-full"
      }`}
    >
      <div className="bg-gray-900/95 backdrop-blur-sm border-t border-white/10 rounded-t-2xl max-h-[60vh] overflow-y-auto">
        <div className="flex items-center justify-between px-5 py-3 border-b border-white/10">
          <h3 className="text-white text-sm font-semibold">
            {editing ? "Edit Metadata" : "Photo Details"}
          </h3>
          <div className="flex items-center gap-2">
            {!editing && photoId && (
              <button
                onClick={startEdit}
                className="text-blue-400 hover:text-blue-300 text-xs transition-colors"
              >
                Edit
              </button>
            )}
            <button
              onClick={onClose}
              className="text-gray-400 hover:text-white transition-colors"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        </div>
        <div className="px-5 py-4 space-y-3 text-sm">
          {error && (
            <div className="bg-red-900/50 text-red-300 px-3 py-2 rounded text-xs">{error}</div>
          )}
          {photoInfo ? (
            editing ? (
              <>
                {/* ── Basic ── */}
                <div className="text-gray-500 text-[10px] uppercase tracking-wider pt-1">File</div>
                <EditRow label="Filename" value={editFilename} onChange={setEditFilename} />
                <EditRow label="Date Taken" value={editTakenAt} onChange={setEditTakenAt}
                  placeholder="2024-01-15T14:30:00Z" />
                <EditRow label="Description" value={editDescription} onChange={setEditDescription} />
                <EditRow label="Comment" value={editUserComment} onChange={setEditUserComment} />

                {/* ── GPS ── */}
                <div className="text-gray-500 text-[10px] uppercase tracking-wider pt-3">Location</div>
                <EditRow label="Latitude" value={editLat} onChange={setEditLat}
                  placeholder="-90 to 90" type="number" />
                <EditRow label="Longitude" value={editLon} onChange={setEditLon}
                  placeholder="-180 to 180" type="number" />

                {/* ── Camera / Lens ── */}
                <div className="text-gray-500 text-[10px] uppercase tracking-wider pt-3">Camera / Lens</div>
                <EditRow label="Camera Model" value={editCamera} onChange={setEditCamera} />
                <EditRow label="Camera Make" value={editCameraMake} onChange={setEditCameraMake} />
                <EditRow label="Lens" value={editLens} onChange={setEditLens} />

                {/* ── Exposure settings ── */}
                <div className="text-gray-500 text-[10px] uppercase tracking-wider pt-3">Exposure</div>
                <EditRow label="ISO" value={editIso} onChange={setEditIso} type="number" />
                <EditRow label="F-Number" value={editFNumber} onChange={setEditFNumber}
                  placeholder="e.g. 2.8" type="number" />
                <EditRow label="Exposure Time" value={editExposureTime} onChange={setEditExposureTime}
                  placeholder="e.g. 1/250" />
                <EditRow label="Focal Length" value={editFocalLength} onChange={setEditFocalLength}
                  placeholder="mm" type="number" />
                <EditRow label="Exposure Bias" value={editExposureBias} onChange={setEditExposureBias}
                  placeholder="EV" type="number" />
                <EditRow label="Exposure Prog" value={editExposureProgram} onChange={setEditExposureProgram}
                  placeholder="e.g. Aperture priority" />
                <EditRow label="Metering" value={editMeteringMode} onChange={setEditMeteringMode}
                  placeholder="e.g. Multi-segment" />
                <EditRow label="Flash" value={editFlash} onChange={setEditFlash}
                  placeholder="e.g. No Flash" />
                <EditRow label="White Balance" value={editWhiteBalance} onChange={setEditWhiteBalance}
                  placeholder="e.g. Auto" />

                {/* ── Other metadata ── */}
                <div className="text-gray-500 text-[10px] uppercase tracking-wider pt-3">Other</div>
                <EditRow label="Orientation" value={editOrientation} onChange={setEditOrientation}
                  placeholder="1-8" type="number" />
                <EditRow label="Color Space" value={editColorSpace} onChange={setEditColorSpace}
                  placeholder="e.g. sRGB" />
                <EditRow label="Scene Type" value={editSceneType} onChange={setEditSceneType} />
                <EditRow label="Digital Zoom" value={editDigitalZoom} onChange={setEditDigitalZoom}
                  type="number" />
                <EditRow label="Software" value={editSoftware} onChange={setEditSoftware} />
                <EditRow label="Artist" value={editArtist} onChange={setEditArtist} />
                <EditRow label="Copyright" value={editCopyright} onChange={setEditCopyright} />

                <div className="flex gap-2 pt-3">
                  <button
                    onClick={saveEdit}
                    disabled={saving}
                    className="flex-1 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white px-3 py-1.5 rounded text-xs"
                  >
                    {saving ? "Saving..." : "Save"}
                  </button>
                  <button
                    onClick={cancelEdit}
                    disabled={saving}
                    className="flex-1 bg-gray-700 hover:bg-gray-600 disabled:opacity-50 text-white px-3 py-1.5 rounded text-xs"
                  >
                    Cancel
                  </button>
                </div>
              </>
            ) : (
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
                {fullMeta?.description && (
                  <InfoRow label="Description" value={fullMeta.description} />
                )}

                {/* Camera/Lens section */}
                {(photoInfo.cameraModel || fullMeta?.camera_make || fullMeta?.lens_model) && (
                  <div className="pt-2 border-t border-white/10 space-y-1">
                    {photoInfo.cameraModel && (
                      <InfoRow label="Camera" value={
                        fullMeta?.camera_make
                          ? `${fullMeta.camera_make} ${photoInfo.cameraModel}`
                          : photoInfo.cameraModel
                      } />
                    )}
                    {fullMeta?.lens_model && (
                      <InfoRow label="Lens" value={fullMeta.lens_model} />
                    )}
                  </div>
                )}

                {/* Exposure section */}
                {(fullMeta?.iso_speed != null || fullMeta?.f_number != null || fullMeta?.exposure_time || fullMeta?.focal_length != null) && (
                  <div className="pt-2 border-t border-white/10 space-y-1">
                    {fullMeta?.iso_speed != null && (
                      <InfoRow label="ISO" value={String(fullMeta.iso_speed)} />
                    )}
                    {fullMeta?.f_number != null && (
                      <InfoRow label="Aperture" value={`f/${fullMeta.f_number}`} />
                    )}
                    {fullMeta?.exposure_time && (
                      <InfoRow label="Shutter" value={fullMeta.exposure_time} />
                    )}
                    {fullMeta?.focal_length != null && (
                      <InfoRow label="Focal Length" value={`${fullMeta.focal_length}mm`} />
                    )}
                    {fullMeta?.flash && (
                      <InfoRow label="Flash" value={fullMeta.flash} />
                    )}
                    {fullMeta?.white_balance && (
                      <InfoRow label="White Balance" value={fullMeta.white_balance} />
                    )}
                    {fullMeta?.metering_mode && (
                      <InfoRow label="Metering" value={fullMeta.metering_mode} />
                    )}
                  </div>
                )}

                {/* Location section */}
                {fullMeta?.geo_city && (
                  <InfoRow label="Location" value={
                    [fullMeta.geo_city, fullMeta.geo_state, fullMeta.geo_country]
                      .filter(Boolean).join(", ")
                  } />
                )}
                {photoInfo.latitude != null && photoInfo.longitude != null && (
                  <div className="flex justify-between items-start">
                    <span className="text-gray-400 shrink-0 w-24">GPS</span>
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

                {/* Other metadata */}
                {(fullMeta?.artist || fullMeta?.copyright || fullMeta?.software) && (
                  <div className="pt-2 border-t border-white/10 space-y-1">
                    {fullMeta?.artist && <InfoRow label="Artist" value={fullMeta.artist} />}
                    {fullMeta?.copyright && <InfoRow label="Copyright" value={fullMeta.copyright} />}
                    {fullMeta?.software && <InfoRow label="Software" value={fullMeta.software} />}
                  </div>
                )}

                {photoInfo.albumNames && photoInfo.albumNames.length > 0 && (
                  <InfoRow label="Albums" value={photoInfo.albumNames.join(", ")} />
                )}
                {/* EXIF section */}
                {fullMeta?.exif_tags && Object.keys(fullMeta.exif_tags).length > 0 && (
                  <div className="pt-2 border-t border-white/10">
                    <button
                      onClick={() => setShowExif(!showExif)}
                      className="text-gray-400 hover:text-white text-xs w-full text-left"
                    >
                      {showExif ? "▼" : "▶"} Raw EXIF ({Object.keys(fullMeta.exif_tags).length} tags)
                    </button>
                    {showExif && (
                      <div className="mt-2 space-y-1 max-h-40 overflow-y-auto">
                        {Object.entries(fullMeta.exif_tags).sort().map(([key, val]) => (
                          <div key={key} className="flex justify-between gap-2 text-xs">
                            <span className="text-gray-500 shrink-0">{key}</span>
                            <span className="text-gray-300 text-right break-all truncate max-w-[200px]">{val}</span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
                {/* Write EXIF button */}
                {isJpegOrTiff && photoId && (
                  <div className="pt-2 border-t border-white/10">
                    <button
                      onClick={writeExif}
                      disabled={saving}
                      className="text-xs text-gray-400 hover:text-white disabled:opacity-50"
                    >
                      {saving ? "Writing..." : "Write to File EXIF"}
                    </button>
                  </div>
                )}
              </>
            )
          ) : (
            <p className="text-gray-400 italic">No metadata available</p>
          )}
        </div>
      </div>
    </div>
  );
}
