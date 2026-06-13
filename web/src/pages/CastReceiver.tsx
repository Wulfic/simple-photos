/**
 * Chromecast receiver page — served at /cast-view.
 *
 * When the Chromecast loads this page it acts as the presentation receiver.
 * The sender (AppHeader / CastDialog running in the user's browser) connects
 * via the Presentation API and sends JSON messages of the shape:
 *
 *   { type: "LOAD_MEDIA", url: string }
 *
 * The page displays the photo full-screen on the TV.
 *
 * This route MUST remain publicly accessible (no auth guard) because the
 * Chromecast fetches it independently.
 */
import { useEffect, useRef, useState } from "react";

interface CastMedia {
  url: string;
  kind: "photo" | "video";
}

export default function CastReceiver() {
  const [media, setMedia] = useState<CastMedia | null>(null);
  const [error, setError] = useState<string | null>(null);
  // True once the server stops answering health pings — used to stop showing
  // stale media when the server is shut down or restarted (issue #2c).
  const [serverDown, setServerDown] = useState(false);
  // The on-screen <video>, so playback-control messages can drive it.
  const videoRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    const pres = navigator.presentation as (typeof navigator.presentation & {
      receiver?: {
        connectionList: Promise<{
          connections: PresentationConnection[];
          onconnectionavailable: ((e: { connection: PresentationConnection }) => void) | null;
        }>;
      };
    }) | null;

    if (!pres?.receiver) {
      // Not in receiver context — this page was opened directly in a browser.
      setError("This page is the Chromecast receiver. Open it on a Chromecast device by casting from the Simple Photos app.");
      return;
    }

    function attachHandlers(conn: PresentationConnection) {
      conn.onmessage = (evt: MessageEvent) => {
        try {
          const msg = JSON.parse(evt.data as string) as {
            type: string;
            url?: string;
            contentType?: string;
            mediaKind?: "photo" | "video";
            action?: "play" | "pause" | "seek";
            position?: number;
          };
          if (msg.type === "LOAD_MEDIA" && msg.url) {
            // Prefer the explicit mediaKind. Fall back to sniffing the
            // contentType so older senders (which only sent { type, url })
            // continue to work for photos.
            const kind: "photo" | "video" =
              msg.mediaKind === "video" ||
              (msg.contentType?.startsWith("video/") ?? false)
                ? "video"
                : "photo";
            setMedia({ url: msg.url, kind });
          } else if (msg.type === "VIDEO_CONTROL") {
            // Mirror the controller's play / pause / scrub onto our <video>
            // so the casted device follows along (issue #2a).
            const v = videoRef.current;
            if (!v) return;
            const pos = typeof msg.position === "number" ? msg.position : undefined;
            if (msg.action === "seek") {
              if (pos !== undefined) v.currentTime = pos;
            } else if (msg.action === "play") {
              // Re-sync position before resuming if we've drifted.
              if (pos !== undefined && Math.abs(v.currentTime - pos) > 0.5) {
                v.currentTime = pos;
              }
              void v.play().catch(() => { /* autoplay/race — ignore */ });
            } else if (msg.action === "pause") {
              if (pos !== undefined) v.currentTime = pos;
              v.pause();
            }
          }
        } catch {
          // malformed message — ignore
        }
      };
      conn.onclose = () => {
        // Session ended — show waiting state
        setMedia(null);
      };
      conn.onterminate = () => {
        setMedia(null);
      };
    }

    pres.receiver.connectionList.then((list) => {
      list.connections.forEach(attachHandlers);
      list.onconnectionavailable = (evt) => attachHandlers(evt.connection);
    }).catch((e) => {
      console.error("[cast-receiver] connectionList error", e);
    });
  }, []);

  // ── Server liveness — stop casting when the server goes away (issue #2c) ──
  // The Presentation connection is a direct browser↔device link that survives
  // a server shutdown, so the receiver would otherwise keep displaying the
  // last frame forever. Poll /health; after a few consecutive failures, clear
  // the media and show an offline state. Recovers automatically on restart.
  useEffect(() => {
    let fails = 0;
    let cancelled = false;
    const check = async () => {
      try {
        const res = await fetch(`/health?t=${Date.now()}`, { cache: "no-store" });
        if (!res.ok) throw new Error(`status ${res.status}`);
        fails = 0;
        if (!cancelled) setServerDown(false);
      } catch {
        fails += 1;
        if (fails >= 3 && !cancelled) {
          setServerDown(true);
          setMedia(null);
        }
      }
    };
    void check();
    const iv = window.setInterval(() => void check(), 5000);
    return () => {
      cancelled = true;
      window.clearInterval(iv);
    };
  }, []);

  // ── Direct-browser error state ──────────────────────────────────────────
  if (error) {
    return (
      <div className="min-h-screen bg-black flex items-center justify-center p-8 text-center">
        <div>
          <svg viewBox="0 0 24 24" fill="white" className="w-12 h-12 mx-auto mb-4 opacity-50" aria-hidden="true">
            <path d="M21 3H3c-1.1 0-2 .9-2 2v3h2V5h18v14h-7v2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zM1 18v3h3c0-1.66-1.34-3-3-3zm0-4v2c2.76 0 5 2.24 5 5h2c0-3.87-3.13-7-7-7zm0-4v2c4.97 0 9 4.03 9 9h2c0-6.08-4.93-11-11-11z" />
          </svg>
          <p className="text-white/60 text-sm max-w-xs">{error}</p>
        </div>
      </div>
    );
  }

  // ── Server offline (issue #2c) ───────────────────────────────────────────
  if (serverDown) {
    return (
      <div className="min-h-screen bg-black flex flex-col items-center justify-center gap-6">
        <svg viewBox="0 0 24 24" fill="white" className="w-16 h-16 opacity-30" aria-hidden="true">
          <path d="M21 3H3c-1.1 0-2 .9-2 2v3h2V5h18v14h-7v2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zM1 18v3h3c0-1.66-1.34-3-3-3zm0-4v2c2.76 0 5 2.24 5 5h2c0-3.87-3.13-7-7-7zm0-4v2c4.97 0 9 4.03 9 9h2c0-6.08-4.93-11-11-11z" />
        </svg>
        <p className="text-white/40 text-sm tracking-wide uppercase">Simple Photos — disconnected</p>
      </div>
    );
  }

  // ── Waiting for content ─────────────────────────────────────────────────
  if (!media) {
    return (
      <div className="min-h-screen bg-black flex flex-col items-center justify-center gap-6">
        <svg viewBox="0 0 24 24" fill="white" className="w-16 h-16 opacity-30" aria-hidden="true">
          <path d="M21 3H3c-1.1 0-2 .9-2 2v3h2V5h18v14h-7v2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zM1 18v3h3c0-1.66-1.34-3-3-3zm0-4v2c2.76 0 5 2.24 5 5h2c0-3.87-3.13-7-7-7zm0-4v2c4.97 0 9 4.03 9 9h2c0-6.08-4.93-11-11-11z" />
        </svg>
        <p className="text-white/40 text-sm tracking-wide uppercase">Simple Photos</p>
      </div>
    );
  }

  // ── Media display ───────────────────────────────────────────────────────
  return (
    <div className="min-h-screen bg-black flex items-center justify-center">
      {media.kind === "video" ? (
        <video
          // `key` forces a fresh element when the URL changes, otherwise the
          // previous video keeps playing.
          key={media.url}
          ref={videoRef}
          src={media.url}
          autoPlay
          controls={false}
          playsInline
          className="max-w-full max-h-screen"
          style={{ width: "100vw", height: "100vh", objectFit: "contain" }}
        />
      ) : (
        <img
          src={media.url}
          alt=""
          className="max-w-full max-h-screen object-contain"
          style={{ width: "100vw", height: "100vh" }}
        />
      )}
    </div>
  );
}
