interface TagsBarProps {
  show: boolean;
  isPlainMode: boolean;
  tags: string[];
  showTagInput: boolean;
  tagInput: string;
  setTagInput: (v: string) => void;
  setShowTagInput: (v: boolean) => void;
  tagSuggestions: string[];
  onAddTag: () => void;
  onRemoveTag: (tag: string) => void;
  tagInputRef: React.RefObject<HTMLInputElement>;
}

export default function TagsBar({
  show,
  isPlainMode,
  tags,
  showTagInput,
  tagInput,
  setTagInput,
  setShowTagInput,
  tagSuggestions,
  onAddTag,
  onRemoveTag,
  tagInputRef,
}: TagsBarProps) {
  return (
    <div className={`absolute bottom-0 left-0 right-0 z-20 transition-opacity duration-300 ${
      show ? 'opacity-100' : 'opacity-0 pointer-events-none'
    } px-4 py-2 bg-black/60 text-gray-400 text-xs space-y-2`}>
      {/* Tags section — plain mode only */}
      {isPlainMode && (
        <div className="flex items-center gap-2 flex-wrap">
          {tags.map((tag) => (
            <span
              key={tag}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-blue-600/30 text-blue-300 text-xs"
            >
              {tag}
              <button
                onClick={() => onRemoveTag(tag)}
                className="hover:text-white ml-0.5"
                title={`Remove tag "${tag}"`}
              >
                ✕
              </button>
            </span>
          ))}

          {/* Add tag button / inline input */}
          {showTagInput ? (
            <div className="relative inline-flex items-center">
              <input
                ref={tagInputRef}
                type="text"
                value={tagInput}
                onChange={(e) => setTagInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") onAddTag();
                  if (e.key === "Escape") { setShowTagInput(false); setTagInput(""); }
                }}
                placeholder="tag name"
                className="w-28 px-2 py-0.5 rounded bg-gray-700 text-white text-xs border border-gray-600 focus:outline-none focus:border-blue-500"
              />
              <button
                onClick={onAddTag}
                className="ml-1 text-blue-400 hover:text-blue-300 text-xs font-medium"
              >
                Add
              </button>
              <button
                onClick={() => { setShowTagInput(false); setTagInput(""); }}
                className="ml-1 text-gray-500 hover:text-gray-300 text-xs"
              >
                ✕
              </button>
              {/* Suggestions dropdown */}
              {tagInput && tagSuggestions.length > 0 && (
                <div className="absolute bottom-full left-0 mb-1 bg-gray-800 border border-gray-600 rounded shadow-lg z-50 min-w-[8rem]">
                  {tagSuggestions.map((s) => (
                    <button
                      key={s}
                      className="block w-full text-left px-2 py-1 text-xs text-gray-300 hover:bg-gray-700 hover:text-white"
                      onClick={() => { setTagInput(s); }}
                    >
                      {s}
                    </button>
                  ))}
                </div>
              )}
            </div>
          ) : (
            <button
              onClick={() => setShowTagInput(true)}
              className="px-2 py-0.5 rounded-full border border-dashed border-gray-500 text-gray-400 hover:text-white hover:border-gray-300 text-xs transition-colors"
            >
              + Tag
            </button>
          )}
        </div>
      )}
    </div>
  );
}
