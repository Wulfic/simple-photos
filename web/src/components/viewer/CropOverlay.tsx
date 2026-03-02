/**
 * Reusable crop overlay with 4 draggable corner dots and connecting lines.
 *
 * Used by the Viewer for both photo and video crop editing. The overlay
 * renders on top of the media element and allows the user to visually
 * define a crop region.
 */
import React from "react";

interface CropOverlayProps {
  /** Bounding rect of the media element (image or video) */
  mediaRect: DOMRect;
  /** Bounding rect of the container element */
  containerRect: DOMRect;
  /** Crop corners as fractions (0–1) of the media dimensions */
  cropCorners: { x: number; y: number; w: number; h: number };
  /** Callback when a corner drag starts */
  onCornerPointerDown: (corner: string) => (e: React.PointerEvent) => void;
}

export default function CropOverlay({
  mediaRect,
  containerRect,
  cropCorners,
  onCornerPointerDown,
}: CropOverlayProps) {
  const ox = mediaRect.left - containerRect.left;
  const oy = mediaRect.top - containerRect.top;
  const iw = mediaRect.width;
  const ih = mediaRect.height;
  const c = cropCorners;
  const tl = { x: ox + c.x * iw, y: oy + c.y * ih };
  const tr = { x: ox + (c.x + c.w) * iw, y: oy + c.y * ih };
  const bl = { x: ox + c.x * iw, y: oy + (c.y + c.h) * ih };
  const br = { x: ox + (c.x + c.w) * iw, y: oy + (c.y + c.h) * ih };
  const dotSize = 16;
  const corners = [
    { key: "tl", ...tl },
    { key: "tr", ...tr },
    { key: "bl", ...bl },
    { key: "br", ...br },
  ];

  return (
    <>
      {/* Darkened area outside crop */}
      <div
        className="absolute pointer-events-none z-20"
        style={{
          left: tl.x, top: tl.y,
          width: tr.x - tl.x, height: bl.y - tl.y,
          boxShadow: "0 0 0 9999px rgba(0,0,0,0.5)",
        }}
      />
      {/* White border lines */}
      <svg className="absolute inset-0 w-full h-full pointer-events-none z-30">
        <line x1={tl.x} y1={tl.y} x2={tr.x} y2={tr.y} stroke="white" strokeWidth={2} />
        <line x1={tr.x} y1={tr.y} x2={br.x} y2={br.y} stroke="white" strokeWidth={2} />
        <line x1={br.x} y1={br.y} x2={bl.x} y2={bl.y} stroke="white" strokeWidth={2} />
        <line x1={bl.x} y1={bl.y} x2={tl.x} y2={tl.y} stroke="white" strokeWidth={2} />
      </svg>
      {/* 4 draggable corner dots */}
      {corners.map((corner) => (
        <div
          key={corner.key}
          onPointerDown={onCornerPointerDown(corner.key)}
          className="absolute z-40 rounded-full bg-white border-2 border-white shadow-lg cursor-grab active:cursor-grabbing"
          style={{
            width: dotSize, height: dotSize,
            left: corner.x - dotSize / 2,
            top: corner.y - dotSize / 2,
            touchAction: "none",
          }}
        />
      ))}
    </>
  );
}
