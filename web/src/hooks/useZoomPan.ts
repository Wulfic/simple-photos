import { useState, useEffect, useRef } from "react";

export default function useZoomPan(
  id: string | undefined,
  editMode: boolean,
  viewerContainerRef: React.RefObject<HTMLDivElement | null>,
) {
  const [zoomScale, setZoomScale] = useState(1);
  const [zoomOrigin, setZoomOrigin] = useState<{ x: number; y: number }>({ x: 50, y: 50 });
  const [panOffset, setPanOffset] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const lastTapTime = useRef(0);
  const pinchStartDist = useRef<number | null>(null);
  const pinchStartScale = useRef(1);
  const panStart = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null);

  // Reset zoom when photo changes or entering edit mode
  useEffect(() => {
    setZoomScale(1);
    setPanOffset({ x: 0, y: 0 });
  }, [id, editMode]);

  function handleDoubleClickZoom(e: React.MouseEvent) {
    if (editMode) return;
    const rect = viewerContainerRef.current?.getBoundingClientRect();
    if (!rect) return;
    if (zoomScale > 1) {
      setZoomScale(1);
      setPanOffset({ x: 0, y: 0 });
    } else {
      const x = ((e.clientX - rect.left) / rect.width) * 100;
      const y = ((e.clientY - rect.top) / rect.height) * 100;
      setZoomOrigin({ x, y });
      setZoomScale(2);
      setPanOffset({ x: 0, y: 0 });
    }
  }

  function handleWheel(e: React.WheelEvent) {
    if (editMode) return;
    e.preventDefault();
    const rect = viewerContainerRef.current?.getBoundingClientRect();
    if (!rect) return;
    const x = ((e.clientX - rect.left) / rect.width) * 100;
    const y = ((e.clientY - rect.top) / rect.height) * 100;
    setZoomOrigin({ x, y });
    setZoomScale((prev) => {
      const next = prev - e.deltaY * 0.002;
      if (next <= 1) { setPanOffset({ x: 0, y: 0 }); return 1; }
      return Math.min(next, 5);
    });
  }

  return {
    zoomScale, setZoomScale,
    zoomOrigin, setZoomOrigin,
    panOffset, setPanOffset,
    lastTapTime,
    pinchStartDist,
    pinchStartScale,
    panStart,
    handleDoubleClickZoom,
    handleWheel,
  };
}
