/**
 * Sphere360Viewer — WebGL spherical viewer for equirectangular 360° photos.
 *
 * Uses three.js to project the panorama onto the inside of a sphere with
 * the camera at the centre, giving a true 360° photo-sphere experience
 * (look up/down/left/right with no fish-eye distortion at the poles).
 *
 * Controls:
 *   - Drag (pointer / touch) to look around.
 *   - Wheel / pinch to zoom (clamped to a sane FOV range).
 *   - "Full View" toggle drops back to the flat equirectangular preview.
 */
import { useEffect, useRef, useState, useCallback } from "react";
import * as THREE from "three";

interface Sphere360ViewerProps {
  /** Object URL or data URL of the equirectangular image. */
  mediaUrl: string;
  /** Called when the user clicks the toggle to leave 360° mode. */
  onExitToFull: () => void;
}

const MIN_FOV = 30;
const MAX_FOV = 100;
const INITIAL_FOV = 75;

export default function Sphere360Viewer({ mediaUrl, onExitToFull }: Sphere360ViewerProps) {
  const mountRef = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Mutable rendering state lives in refs so it survives re-renders without
  // tearing down the GL context.
  const stateRef = useRef<{
    renderer?: THREE.WebGLRenderer;
    scene?: THREE.Scene;
    camera?: THREE.PerspectiveCamera;
    texture?: THREE.Texture;
    mesh?: THREE.Mesh;
    raf?: number;
    // Look orientation in radians.
    lon: number;
    lat: number;
    // Pointer drag tracking.
    isDragging: boolean;
    dragStartX: number;
    dragStartY: number;
    lonStart: number;
    latStart: number;
  }>({
    lon: 0,
    lat: 0,
    isDragging: false,
    dragStartX: 0,
    dragStartY: 0,
    lonStart: 0,
    latStart: 0,
  });

  // ── Initialise scene ────────────────────────────────────────────────────
  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;
    setReady(false);
    setError(null);

    // Guard against environments without WebGL (older browsers / privacy modes).
    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
    } catch {
      setError("WebGL is not available in this browser.");
      return;
    }
    renderer.setPixelRatio(window.devicePixelRatio || 1);
    renderer.setSize(mount.clientWidth, mount.clientHeight, false);
    renderer.domElement.style.display = "block";
    renderer.domElement.style.width = "100%";
    renderer.domElement.style.height = "100%";
    renderer.domElement.style.touchAction = "none";
    mount.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(
      INITIAL_FOV,
      mount.clientWidth / Math.max(1, mount.clientHeight),
      0.1,
      1100,
    );
    camera.position.set(0, 0, 0);

    // Inverted sphere — flip on X so the texture is right-handed for a
    // viewer at the centre.
    const geometry = new THREE.SphereGeometry(500, 64, 32);
    geometry.scale(-1, 1, 1);

    // Placeholder material until the texture finishes loading.
    const mesh = new THREE.Mesh(
      geometry,
      new THREE.MeshBasicMaterial({ color: 0x000000 }),
    );
    scene.add(mesh);

    stateRef.current.renderer = renderer;
    stateRef.current.scene = scene;
    stateRef.current.camera = camera;
    stateRef.current.mesh = mesh;

    // Load texture asynchronously so we don't block first paint.
    //
    // We avoid `THREE.TextureLoader` because it sets `crossorigin="anonymous"`
    // on its internal <img>, which breaks `blob:` object URLs (the form
    // used by the gallery's decrypted media pipeline).  We also avoid a
    // plain `new Image()` because some browsers flag the resulting image
    // as cross-origin tainted when uploaded to WebGL, leaving the sphere
    // black.  `fetch` + `createImageBitmap` is the modern, CORS-safe path
    // for WebGL textures and is widely supported.
    let cancelled = false;
    const loadTexture = async () => {
      try {
        let source: ImageBitmap | HTMLImageElement | HTMLCanvasElement;
        if (typeof createImageBitmap === "function") {
          const resp = await fetch(mediaUrl);
          if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
          const blob = await resp.blob();
          source = await createImageBitmap(blob);
        } else {
          // Fallback for older browsers without createImageBitmap.
          source = await new Promise<HTMLImageElement>((resolve, reject) => {
            const img = new Image();
            img.decoding = "async";
            img.onload = () => resolve(img);
            img.onerror = () => reject(new Error("image load failed"));
            img.src = mediaUrl;
          });
        }
        if (cancelled) {
          if ("close" in source) (source as ImageBitmap).close();
          return;
        }

        // Photo spheres are commonly 5–8K wide; many mobile GPUs cap
        // MAX_TEXTURE_SIZE at 4096.  Uploading an oversized texture fails
        // silently and renders a black sphere — downscale through a canvas
        // when the source exceeds the device limit.
        const maxTex = renderer.capabilities.maxTextureSize || 4096;
        const srcW = source.width;
        const srcH = source.height;
        if (srcW > maxTex || srcH > maxTex) {
          const scale = Math.min(maxTex / srcW, maxTex / srcH);
          const canvas = document.createElement("canvas");
          canvas.width = Math.max(1, Math.floor(srcW * scale));
          canvas.height = Math.max(1, Math.floor(srcH * scale));
          const ctx = canvas.getContext("2d");
          if (ctx) {
            ctx.drawImage(source as CanvasImageSource, 0, 0, canvas.width, canvas.height);
            if ("close" in source) (source as ImageBitmap).close();
            source = canvas;
          } else {
            console.warn("[Sphere360Viewer] 2D context unavailable; uploading full-size texture");
          }
        }

        const tex = new THREE.Texture(source as HTMLImageElement);
        tex.colorSpace = THREE.SRGBColorSpace;
        tex.minFilter = THREE.LinearFilter;
        tex.magFilter = THREE.LinearFilter;
        tex.wrapS = THREE.ClampToEdgeWrapping;
        tex.wrapT = THREE.ClampToEdgeWrapping;
        tex.needsUpdate = true;
        const mat = new THREE.MeshBasicMaterial({ map: tex });
        if (stateRef.current.mesh) {
          (stateRef.current.mesh.material as THREE.Material).dispose();
          stateRef.current.mesh.material = mat;
        }
        stateRef.current.texture = tex;
        setReady(true);
      } catch (e) {
        if (!cancelled) {
          // eslint-disable-next-line no-console
          console.error("[Sphere360Viewer] texture load failed:", e);
          setError("Failed to load panorama image.");
        }
      }
    };
    loadTexture();

    // ── Render loop ───────────────────────────────────────────────────────
    const tick = () => {
      const s = stateRef.current;
      if (!s.renderer || !s.scene || !s.camera) return;
      // Clamp pitch to just under ±90° so we don't flip past the pole.
      s.lat = Math.max(-Math.PI / 2 + 0.01, Math.min(Math.PI / 2 - 0.01, s.lat));
      const target = new THREE.Vector3(
        Math.cos(s.lat) * Math.sin(s.lon),
        Math.sin(s.lat),
        Math.cos(s.lat) * Math.cos(s.lon),
      );
      s.camera.lookAt(target);
      s.renderer.render(s.scene, s.camera);
      s.raf = requestAnimationFrame(tick);
    };
    tick();

    // ── Resize handling ──────────────────────────────────────────────────
    const handleResize = () => {
      const s = stateRef.current;
      if (!mount || !s.renderer || !s.camera) return;
      const w = mount.clientWidth;
      const h = mount.clientHeight;
      s.renderer.setSize(w, h, false);
      s.camera.aspect = w / Math.max(1, h);
      s.camera.updateProjectionMatrix();
    };
    const ro = new ResizeObserver(handleResize);
    ro.observe(mount);

    // ── Wheel zoom ────────────────────────────────────────────────────────
    // Registered natively with `passive: false`: React marks its root-level
    // wheel listeners passive, so `preventDefault()` inside an `onWheel`
    // prop is silently ignored and the page scrolls while zooming.
    const handleWheel = (e: WheelEvent) => {
      const s = stateRef.current;
      if (!s.camera) return;
      e.preventDefault();
      // Positive deltaY = zoom out (wider FOV); negative = zoom in.
      const next = s.camera.fov + e.deltaY * 0.05;
      s.camera.fov = Math.max(MIN_FOV, Math.min(MAX_FOV, next));
      s.camera.updateProjectionMatrix();
    };
    mount.addEventListener("wheel", handleWheel, { passive: false });

    // ── Cleanup ─────────────────────────────────────────────────────────
    return () => {
      cancelled = true;
      ro.disconnect();
      mount.removeEventListener("wheel", handleWheel);
      const s = stateRef.current;
      if (s.raf) cancelAnimationFrame(s.raf);
      if (s.mesh) {
        s.mesh.geometry.dispose();
        const mat = s.mesh.material as THREE.Material | THREE.Material[];
        if (Array.isArray(mat)) mat.forEach((m) => m.dispose());
        else mat.dispose();
      }
      if (s.texture) s.texture.dispose();
      if (s.renderer) {
        s.renderer.dispose();
        if (s.renderer.domElement.parentNode === mount) {
          mount.removeChild(s.renderer.domElement);
        }
      }
      stateRef.current = {
        lon: 0,
        lat: 0,
        isDragging: false,
        dragStartX: 0,
        dragStartY: 0,
        lonStart: 0,
        latStart: 0,
      };
    };
  }, [mediaUrl]);

  // ── Pointer / touch interaction ───────────────────────────────────────
  const onPointerDown = useCallback((e: React.PointerEvent) => {
    const s = stateRef.current;
    s.isDragging = true;
    s.dragStartX = e.clientX;
    s.dragStartY = e.clientY;
    s.lonStart = s.lon;
    s.latStart = s.lat;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, []);

  const onPointerMove = useCallback((e: React.PointerEvent) => {
    const s = stateRef.current;
    if (!s.isDragging || !s.camera) return;
    const mount = mountRef.current;
    if (!mount) return;
    // Drag distance in radians proportional to FOV — feels natural at
    // any zoom level.
    const fovRad = (s.camera.fov * Math.PI) / 180;
    const radPerPixelX = fovRad / mount.clientWidth;
    const radPerPixelY = fovRad / mount.clientHeight;
    const dx = e.clientX - s.dragStartX;
    const dy = e.clientY - s.dragStartY;
    s.lon = s.lonStart - dx * radPerPixelX;
    s.lat = s.latStart + dy * radPerPixelY;
  }, []);

  const onPointerUp = useCallback((e: React.PointerEvent) => {
    const s = stateRef.current;
    s.isDragging = false;
    try {
      (e.target as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
  }, []);

  // Pinch-to-zoom for touch.  No preventDefault here — React touch
  // listeners are passive (the call would be a console error and a no-op);
  // the container's `touch-action: none` is what suppresses the browser's
  // own pinch/scroll gestures.
  const pinchRef = useRef<{ startDist: number; startFov: number } | null>(null);

  const onTouchStart = useCallback((e: React.TouchEvent) => {
    if (e.touches.length === 2 && stateRef.current.camera) {
      const dx = e.touches[0].clientX - e.touches[1].clientX;
      const dy = e.touches[0].clientY - e.touches[1].clientY;
      pinchRef.current = {
        startDist: Math.hypot(dx, dy),
        startFov: stateRef.current.camera.fov,
      };
    }
  }, []);

  const onTouchMove = useCallback((e: React.TouchEvent) => {
    if (e.touches.length === 2 && pinchRef.current && stateRef.current.camera) {
      const dx = e.touches[0].clientX - e.touches[1].clientX;
      const dy = e.touches[0].clientY - e.touches[1].clientY;
      const dist = Math.hypot(dx, dy);
      const ratio = pinchRef.current.startDist / Math.max(1, dist);
      const next = pinchRef.current.startFov * ratio;
      stateRef.current.camera.fov = Math.max(MIN_FOV, Math.min(MAX_FOV, next));
      stateRef.current.camera.updateProjectionMatrix();
    }
  }, []);

  const onTouchEnd = useCallback(() => {
    pinchRef.current = null;
  }, []);

  return (
    <div
      className="relative w-full h-full bg-black select-none"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onTouchStart={onTouchStart}
      onTouchMove={onTouchMove}
      onTouchEnd={onTouchEnd}
      style={{ touchAction: "none", cursor: "grab" }}
    >
      {/* WebGL mount.  Hidden (not unmounted) when an error occurs so the
          init useEffect can still keep its ref and run cleanup correctly. */}
      <div
        ref={mountRef}
        className="absolute inset-0"
        style={{ visibility: error ? "hidden" : "visible" }}
      />

      {!ready && !error && (
        <div className="absolute inset-0 flex items-center justify-center text-white text-sm">
          Loading 360° view…
        </div>
      )}
      {error && (
        <>
          {/* Flat fallback so the user still sees the image when WebGL or
              the texture load fails. */}
          <img
            src={mediaUrl}
            alt="Panorama"
            className="absolute inset-0 w-full h-full object-contain pointer-events-none"
          />
          <div className="absolute top-4 left-1/2 -translate-x-1/2 z-30 px-3 py-1 rounded-full bg-red-600/80 text-white text-xs">
            {error}
          </div>
        </>
      )}

      <button
        onClick={(e) => {
          e.stopPropagation();
          onExitToFull();
        }}
        className="absolute bottom-24 left-1/2 -translate-x-1/2 z-30 flex items-center gap-2 px-4 py-1.5 rounded-full bg-black/60 text-white text-sm font-medium hover:bg-black/80 transition-colors backdrop-blur-sm"
      >
        <span className="text-xs font-bold opacity-70">360°</span>
        <span>Full View</span>
      </button>
    </div>
  );
}
