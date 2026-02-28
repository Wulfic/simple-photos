import { useRef, useCallback } from "react";

export default function useLongPress(callback: () => void, delay = 500) {
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const triggeredRef = useRef(false);

  const onTouchStart = useCallback(() => {
    triggeredRef.current = false;
    timerRef.current = setTimeout(() => {
      triggeredRef.current = true;
      callback();
    }, delay);
  }, [callback, delay]);

  const onTouchEnd = useCallback(() => {
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = null;
  }, []);

  const onTouchMove = useCallback(() => {
    // Cancel long press if user moves finger (it's a scroll, not a hold)
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = null;
  }, []);

  // Returns true if the long press was triggered (so onClick can be suppressed)
  const wasLongPress = useCallback(() => triggeredRef.current, []);

  return { onTouchStart, onTouchEnd, onTouchMove, wasLongPress };
}
