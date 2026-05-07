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
import { useEffect, useState } from "react";

export default function CastReceiver() {
  const [photoUrl, setPhotoUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

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
          const msg = JSON.parse(evt.data as string) as { type: string; url?: string };
          if (msg.type === "LOAD_MEDIA" && msg.url) {
            setPhotoUrl(msg.url);
          }
        } catch {
          // malformed message — ignore
        }
      };
      conn.onclose = () => {
        // Session ended — show waiting state
        setPhotoUrl(null);
      };
      conn.onterminate = () => {
        setPhotoUrl(null);
      };
    }

    pres.receiver.connectionList.then((list) => {
      list.connections.forEach(attachHandlers);
      list.onconnectionavailable = (evt) => attachHandlers(evt.connection);
    }).catch((e) => {
      console.error("[cast-receiver] connectionList error", e);
    });
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

  // ── Waiting for content ─────────────────────────────────────────────────
  if (!photoUrl) {
    return (
      <div className="min-h-screen bg-black flex flex-col items-center justify-center gap-6">
        <svg viewBox="0 0 24 24" fill="white" className="w-16 h-16 opacity-30" aria-hidden="true">
          <path d="M21 3H3c-1.1 0-2 .9-2 2v3h2V5h18v14h-7v2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zM1 18v3h3c0-1.66-1.34-3-3-3zm0-4v2c2.76 0 5 2.24 5 5h2c0-3.87-3.13-7-7-7zm0-4v2c4.97 0 9 4.03 9 9h2c0-6.08-4.93-11-11-11z" />
        </svg>
        <p className="text-white/40 text-sm tracking-wide uppercase">Simple Photos</p>
      </div>
    );
  }

  // ── Photo display ───────────────────────────────────────────────────────
  return (
    <div className="min-h-screen bg-black flex items-center justify-center">
      <img
        src={photoUrl}
        alt=""
        className="max-w-full max-h-screen object-contain"
        style={{ width: "100vw", height: "100vh" }}
      />
    </div>
  );
}
