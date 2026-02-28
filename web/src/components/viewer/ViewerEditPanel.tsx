interface ViewerEditPanelProps {
  editTab: "crop" | "brightness";
  setEditTab: (tab: "crop" | "brightness") => void;
  brightness: number;
  setBrightness: (v: number) => void;
  cropData: { x: number; y: number; width: number; height: number; rotate: number; brightness?: number } | null;
  onSave: () => void;
  onClear: () => void;
  onCancel: () => void;
}

export default function ViewerEditPanel({
  editTab,
  setEditTab,
  brightness,
  setBrightness,
  cropData,
  onSave,
  onClear,
  onCancel,
}: ViewerEditPanelProps) {
  return (
    <div className="absolute bottom-0 left-0 right-0 z-30 bg-black/90 border-t border-white/10 px-4 py-3 space-y-3">
      {/* Tab switcher */}
      <div className="flex items-center justify-center gap-2">
        <button
          onClick={() => setEditTab("crop")}
          className={`px-4 py-1.5 rounded-full text-sm font-medium transition-colors ${
            editTab === "crop"
              ? "bg-white text-black"
              : "bg-white/10 text-white hover:bg-white/20"
          }`}
        >
          Crop
        </button>
        <button
          onClick={() => setEditTab("brightness")}
          className={`px-4 py-1.5 rounded-full text-sm font-medium transition-colors ${
            editTab === "brightness"
              ? "bg-white text-black"
              : "bg-white/10 text-white hover:bg-white/20"
          }`}
        >
          Brightness
        </button>
      </div>

      {/* Brightness slider */}
      {editTab === "brightness" && (
        <div className="flex items-center gap-3 max-w-sm mx-auto">
          <svg className="w-5 h-5 text-gray-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <circle cx="12" cy="12" r="4" />
          </svg>
          <input
            type="range"
            min={-100}
            max={100}
            value={brightness}
            onChange={(e) => setBrightness(Number(e.target.value))}
            className="flex-1 h-1.5 rounded-full appearance-none bg-white/20 accent-white cursor-pointer"
          />
          <svg className="w-5 h-5 text-yellow-300 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <circle cx="12" cy="12" r="4" />
            <path strokeLinecap="round" d="M12 2v2m0 16v2m-7.07-3.93l1.41-1.41m9.9-9.9l1.41-1.41M2 12h2m16 0h2M4.93 4.93l1.41 1.41m9.9 9.9l1.41 1.41" />
          </svg>
        </div>
      )}

      {/* Action buttons */}
      <div className="flex items-center justify-center gap-2">
        <button
          onClick={onSave}
          className="px-5 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700 transition-colors"
        >
          Save
        </button>
        {cropData && (
          <button
            onClick={onClear}
            className="px-4 py-2 bg-gray-600 text-white rounded-lg text-sm font-medium hover:bg-gray-500 transition-colors"
          >
            Reset
          </button>
        )}
        <button
          onClick={onCancel}
          className="px-4 py-2 bg-gray-700 text-white rounded-lg text-sm font-medium hover:bg-gray-600 transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
