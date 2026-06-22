/**
 * TEMP diagnostic overlay for the metadata-crop thumbnail bug.
 *
 * Renders a fixed on-page panel (gated by CROP_DEBUG) that joins the live DOM
 * geometry of every cropped/rotated gallery tile with the raw values captured
 * in window.__CROPLOG: what aspect the grid computed (gridAspect), what
 * tile/image size the transform was actually built from (cover), whether the
 * measured or fallback style won (applied), plus the on-screen gap and the
 * transform a correct object-cover would produce. Screenshot-friendly — CSP
 * blocks shipping the trace off-page.
 *
 * Delete this component and its <DebugCropOverlay/> mount once the bug is fixed.
 */
import { useEffect, useState } from "react";
import { CROP_DEBUG } from "../utils/thumbnailCss";

interface CropJson { x?: number; y?: number; width?: number; height?: number; rotate?: number }
type LogEntry = Record<string, unknown> & { tag?: string; filename?: string; cropData?: string; crop?: Record<string, number> };

interface Row {
  file: string;
  live: string;
  grid: string;
  cover: string;
  applied: string;
  ideal: string;
  bad: boolean;
}

function num(v: unknown): number | undefined { return typeof v === "number" ? v : undefined; }

function idealCover(c: CropJson, tW: number, tH: number, iW: number, iH: number): string {
  const rot = ((((c.rotate ?? 0) % 360) + 360) % 360);
  const cw = c.width ?? 1, ch = c.height ?? 1, x0 = c.x ?? 0, y0 = c.y ?? 0;
  if (!(cw < 0.999 || ch < 0.999) || rot !== 0 || !(tW > 0 && tH > 0 && iW > 0 && iH > 0)) return "(rot/none)";
  const arTile = tW / tH, arImg = iW / iH, s = Math.max(1, arTile / arImg);
  const cx = x0 + cw / 2, cy = y0 + ch / 2;
  const scale = Math.max(arTile / (cw * arImg * s), 1 / (ch * s));
  const tx = (-(cx - 0.5) * arImg * s) / arTile * 100, ty = -(cy - 0.5) * s * 100;
  return `scale(${scale.toFixed(2)}) tr(${tx.toFixed(1)}%, ${ty.toFixed(1)}%)`;
}

export default function DebugCropOverlay() {
  const [rows, setRows] = useState<Row[]>([]);

  useEffect(() => {
    if (!CROP_DEBUG) return;
    const tick = () => {
      const w = window as unknown as { __CROPLOG?: LogEntry[] };
      const log = w.__CROPLOG ?? [];

      // crop.width(4dp) → filename, and filename → parsed crop, from measure entries.
      const cwToFile = new Map<string, string>();
      const fileCrop = new Map<string, CropJson>();
      for (const e of log) {
        if (e.filename && e.cropData) {
          try {
            const c = JSON.parse(e.cropData) as CropJson;
            cwToFile.set((c.width ?? 1).toFixed(4), e.filename);
            fileCrop.set(e.filename, c);
          } catch { /* ignore */ }
        }
      }
      const fileOf = (e: LogEntry): string | undefined => {
        if (e.filename) return e.filename;
        const cw = e.crop?.w ?? e.crop?.width;
        return cw != null ? cwToFile.get(cw.toFixed(4)) : undefined;
      };

      // latest log per (file, kind)
      const grid = new Map<string, LogEntry>(), cover = new Map<string, LogEntry>(), applied = new Map<string, LogEntry>();
      for (const e of log) {
        const f = fileOf(e); if (!f) continue;
        const t = e.tag ?? "";
        if (t.startsWith("[CROPDBG:gridAspect]")) grid.set(f, e);
        else if (t.startsWith("[CROPDBG:cover]")) cover.set(f, e);
        else if (t.startsWith("[CROPDBG:applied]")) applied.set(f, e);
      }

      const out: Row[] = [];
      const seen = new Set<string>();
      document.querySelectorAll<HTMLImageElement>('[data-testid="justified-grid-item"] img').forEach((img) => {
        const tf = img.style.transform;
        const isFill = img.style.position === "absolute"; // new crop-fill path
        if ((!tf && !isFill) || seen.has(img.alt)) return;
        seen.add(img.alt);
        const item = img.closest('[data-testid="justified-grid-item"]') as HTMLElement | null;
        if (!item) return;
        const tR = item.getBoundingClientRect();
        const iR = img.getBoundingClientRect();
        const gT = Math.round(iR.top - tR.top), gB = Math.round(tR.bottom - iR.bottom);
        const gL = Math.round(iR.left - tR.left), gR = Math.round(tR.right - iR.right);
        const f = img.alt;
        const g = grid.get(f), cv = cover.get(f), ap = applied.get(f);
        const c = fileCrop.get(f);
        const branch = (cv?.tag ?? "").includes("MEASURED") ? "MEAS" : (cv?.tag ?? "").includes("FALLBACK") ? "FALLBK" : "?";
        out.push({
          file: f,
          live: `tile ${Math.round(tR.width)}x${Math.round(tR.height)} AR${(tR.width / tR.height).toFixed(2)} nat${(img.naturalWidth / Math.max(1, img.naturalHeight)).toFixed(2)} gap[T${gT} B${gB} L${gL} R${gR}]`,
          grid: g ? `grid eff=${num(g.effectiveAR)} clamp=${num(g.clampedTileAR)} (stored ${num(g.storedW)}x${num(g.storedH)})` : "grid: (no log)",
          cover: cv ? `cover ${branch} tile=${num(cv.tileW)}x${num(cv.tileH)} img=${num(cv.imgW)}x${num(cv.imgH)}` : "cover: (no log)",
          applied: isFill
            ? `FILL: w=${img.style.width} h=${img.style.height} left=${img.style.left} top=${img.style.top}`
            : `applied[${ap ? `meas=${String(ap.usingMeasured)}` : "?"}]: ${tf}`,
          ideal: isFill
            ? "ideal: (fill path — gap should be 0)"
            : (c ? `ideal: ${idealCover(c, tR.width, tR.height, img.naturalWidth, img.naturalHeight)}` : "ideal: (no crop)"),
          bad: gT > 1 || gB > 1 || gL > 1 || gR > 1,
        });
      });
      setRows(out);
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  if (!CROP_DEBUG || rows.length === 0) return null;

  return (
    <div style={{
      position: "fixed", left: 8, bottom: 8, zIndex: 99999, maxWidth: "98vw",
      background: "rgba(0,0,0,0.93)", color: "#0f0", font: "11px/1.4 ui-monospace, monospace",
      padding: "8px 10px", borderRadius: 6, whiteSpace: "pre-wrap", border: "1px solid #0a0",
      pointerEvents: "none",
    }}>
      <div style={{ color: "#fff", marginBottom: 4 }}>CROPDBG — edited tiles ({rows.length}) · ❌ = on-screen gap (bug)</div>
      {rows.map((r, i) => (
        <div key={i} style={{ color: r.bad ? "#f66" : "#6f6", marginBottom: 6 }}>
          {r.bad ? "❌" : "✅"} {r.file}
          {"\n"}    {r.live}
          {"\n"}    {r.grid}
          {"\n"}    {r.cover}
          {"\n"}    {r.applied}
          {"\n"}    {r.ideal}
        </div>
      ))}
    </div>
  );
}
