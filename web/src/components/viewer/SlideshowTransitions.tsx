/**
 * CSS-based transition effects for the slideshow.
 *
 * Each transition animates between the outgoing and incoming photo.
 * All use CSS transitions / animations for GPU-accelerated rendering.
 */
import { useEffect, useState, type ReactNode } from "react";
import type { SlideshowTransition } from "../../hooks/useSlideshow";

interface Props {
  /** Unique key that changes on each slide change. */
  slideKey: string;
  /** Which transition effect to use. */
  transition: SlideshowTransition;
  /** +1 for forward, -1 for backward. */
  direction: 1 | -1;
  children: ReactNode;
}

const DURATION = {
  fade: 600,
  slide: 500,
  zoom: 700,
  dissolve: 600,
};

export default function SlideshowTransitions({ slideKey, transition, direction, children }: Props) {
  const [visible, setVisible] = useState(false);

  // Trigger enter animation on mount / key change.
  useEffect(() => {
    // Start hidden, then reveal on next frame.
    setVisible(false);
    const raf = requestAnimationFrame(() => {
      requestAnimationFrame(() => setVisible(true));
    });
    return () => cancelAnimationFrame(raf);
  }, [slideKey]);

  const dur = DURATION[transition];

  const baseStyle: React.CSSProperties = {
    position: "absolute",
    inset: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    willChange: "opacity, transform",
  };

  let style: React.CSSProperties;

  switch (transition) {
    case "slide":
      style = {
        ...baseStyle,
        transition: `transform ${dur}ms ease, opacity ${dur}ms ease`,
        transform: visible ? "translateX(0)" : `translateX(${direction * 100}%)`,
        opacity: visible ? 1 : 0,
      };
      break;

    case "zoom":
      style = {
        ...baseStyle,
        transition: `opacity ${dur}ms ease, transform ${dur}ms ease`,
        opacity: visible ? 1 : 0,
        transform: visible ? "scale(1)" : "scale(1.08)",
      };
      break;

    case "dissolve":
      style = {
        ...baseStyle,
        transition: `opacity ${dur}ms steps(12)`,
        opacity: visible ? 1 : 0,
      };
      break;

    case "fade":
    default:
      style = {
        ...baseStyle,
        transition: `opacity ${dur}ms ease`,
        opacity: visible ? 1 : 0,
      };
      break;
  }

  return (
    <div key={slideKey} style={style}>
      {children}
    </div>
  );
}
