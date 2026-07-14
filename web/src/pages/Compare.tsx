/**
 * Split-screen Compare page (#21) — view two photos simultaneously.
 *
 * Entered from the gallery multi-select bar when exactly two items are
 * selected. Layout adapts to the viewport: side-by-side in landscape,
 * stacked in portrait. Each pane zooms/pans independently (see ComparePane).
 */
import { useEffect } from "react";
import { useLocation } from "react-router-dom";
import { useAppNavigate } from "../hooks/useAppNavigate";
import ComparePane from "../components/compare/ComparePane";
import { COMPARE_COUNT } from "../utils/compare";

interface CompareLocationState {
  /** The two blobIds to compare, in display order. */
  ids?: string[];
  /** Where the back button returns to (defaults to the gallery). */
  backTo?: string;
}

export default function Compare() {
  const location = useLocation();
  const navigate = useAppNavigate();
  const state = (location.state ?? {}) as CompareLocationState;
  const ids = state.ids ?? [];
  const backTo = state.backTo ?? "/gallery";
  const valid = ids.length === COMPARE_COUNT;

  // Deep-linked or reloaded without state → nothing to compare; bounce back.
  useEffect(() => {
    if (!valid) navigate(backTo, { replace: true });
  }, [valid, backTo, navigate]);

  if (!valid) return null;

  return (
    <div className="fixed inset-0 bg-black flex flex-col select-none">
      {/* Slim top bar */}
      <div className="relative z-30 flex items-center h-12 px-2 bg-black/80 text-white shrink-0">
        <button
          onClick={() => navigate(backTo)}
          className="w-9 h-9 flex items-center justify-center rounded-full hover:bg-white/10 transition-colors"
          aria-label="Back"
        >
          <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5" />
          </svg>
        </button>
        <span className="ml-1 text-sm font-medium">Compare</span>
      </div>

      {/* Panes — stacked in portrait, side-by-side in landscape */}
      <div className="flex-1 min-h-0 flex flex-col landscape:flex-row">
        <div className="flex-1 min-h-0 min-w-0 relative">
          <ComparePane photoId={ids[0]} badge="1" />
        </div>
        <div className="bg-white/15 shrink-0 h-px w-full landscape:h-full landscape:w-px" />
        <div className="flex-1 min-h-0 min-w-0 relative">
          <ComparePane photoId={ids[1]} badge="2" />
        </div>
      </div>
    </div>
  );
}
