/** Slide-up panel for viewing / adding / removing tags on a photo. */
import { useEffect, useState, useRef } from "react";
import { tagsApi } from "../../api/tags";

interface TagPanelProps {
  show: boolean;
  onClose: () => void;
  photoId: string | undefined;
}

export default function TagPanel({ show, onClose, photoId }: TagPanelProps) {
  const [tags, setTags] = useState<string[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (show && photoId) {
      setLoading(true);
      tagsApi.getPhotoTags(photoId)
        .then(r => setTags(r.tags ?? []))
        .catch(() => setTags([]))
        .finally(() => setLoading(false));
    }
  }, [show, photoId]);

  useEffect(() => {
    if (show) setTimeout(() => inputRef.current?.focus(), 300);
  }, [show]);

  const handleAdd = async () => {
    const cleaned = input.trim().toLowerCase();
    if (!cleaned || !photoId) return;
    try {
      await tagsApi.add(photoId, cleaned);
      if (!tags.includes(cleaned)) setTags(prev => [...prev, cleaned].sort());
      setInput("");
    } catch {}
  };

  const handleRemove = async (tag: string) => {
    if (!photoId) return;
    try {
      await tagsApi.remove(photoId, tag);
      setTags(prev => prev.filter(t => t !== tag));
    } catch {}
  };

  return (
    <div
      className={`fixed bottom-0 left-0 right-0 z-40 transition-transform duration-300 ease-out ${
        show ? "translate-y-0" : "translate-y-full"
      }`}
    >
      <div className="bg-gray-900/95 backdrop-blur-sm border-t border-white/10 rounded-t-2xl max-h-[60vh] overflow-y-auto">
        <div className="flex items-center justify-between px-5 py-3 border-b border-white/10">
          <h3 className="text-white text-sm font-semibold">Tags</h3>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-white transition-colors"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div className="px-5 py-4 space-y-3">
          {loading ? (
            <p className="text-gray-400 text-sm italic">Loading…</p>
          ) : tags.length === 0 ? (
            <p className="text-gray-400 text-sm italic">No tags yet</p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {tags.map(tag => (
                <span
                  key={tag}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-white/10 text-white text-sm"
                >
                  {tag}
                  <button
                    onClick={() => handleRemove(tag)}
                    className="text-gray-400 hover:text-red-400 transition-colors"
                    title={`Remove "${tag}"`}
                  >
                    <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                    </svg>
                  </button>
                </span>
              ))}
            </div>
          )}
          <form
            onSubmit={e => { e.preventDefault(); handleAdd(); }}
            className="flex gap-2"
          >
            <input
              ref={inputRef}
              type="text"
              value={input}
              onChange={e => setInput(e.target.value)}
              placeholder="Add a tag…"
              className="flex-1 bg-white/10 text-white text-sm rounded-lg px-3 py-2 border border-white/10 focus:border-blue-500 focus:outline-none placeholder-gray-500"
            />
            <button
              type="submit"
              disabled={!input.trim()}
              className="px-4 py-2 rounded-lg bg-blue-600 text-white text-sm font-medium hover:bg-blue-500 disabled:opacity-40 disabled:cursor-default transition-colors"
            >
              Add
            </button>
          </form>
        </div>
      </div>
    </div>
  );
}
