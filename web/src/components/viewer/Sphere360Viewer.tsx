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
    const loader = new THREE.TextureLoader();
    loader.setCrossOrigin("anonymous");
    loader.load(
      mediaUrl,
      (tex) => {
        // Clamp to texture max size — extremely wide 360 images can exceed
        // the GPU limit; let three.js downscale via the internal format.
        tex.colorSpace = THREE.SRGBColorSpace;
        tex.minFilter = THREE.LinearFilter;
        tex.magFilter = THREE.LinearFilter;
        tex.wrapS = THREE.ClampToEdgeWrapping;
        tex.wrapT = THREE.ClampToEdgeWrapping;
        const mat = new THREE.MeshBasicMaterial({ map: tex });
        if (stateRef.current.mesh) {
          (stateRef.current.mesh.material as THREE.Material).dispose();
          stateRef.current.mesh.material = mat;
        }
        stateRef.current.texture = tex;
        setReady(true);
      },
      undefined,
      () => setError("Failed to load panorama image."),
    );

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

    // ── Cleanup ─────────────────────────────────────────────────────────
    return () => {
      ro.disconnect();
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

  const onWheel = useCallback((e: React.WheelEvent) => {
    const s = stateRef.current;
    if (!s.camera) return;
    e.preventDefault();
    // Positive deltaY = zoom out (wider FOV); negative = zoom in.
    const next = s.camera.fov + e.deltaY * 0.05;
    s.camera.fov = Math.max(MIN_FOV, Math.min(MAX_FOV, next));
    s.camera.updateProjectionMatrix();
  }, []);

  // Pinch-to-zoom for touch.
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
      e.preventDefault();
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
      onWheel={onWheel}
      onTouchStart={onTouchStart}
      onTouchMove={onTouchMove}
      onTouchEnd={onTouchEnd}
      style={{ touchAction: "none", cursor: "grab" }}
    >
      <div ref={mountRef} className="absolute inset-0" />

      {!ready && !error && (
        <div className="absolute inset-0 flex items-center justify-center text-white text-sm">
          Loading 360° view…
        </div>
      )}
      {error && (
        <div className="absolute inset-0 flex items-center justify-center text-red-400 text-sm">
          {error}
        </div>
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
