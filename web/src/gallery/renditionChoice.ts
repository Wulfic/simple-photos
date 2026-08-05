/**
 * Which quality of a video to play, and what to call it (#49).
 *
 * Pure by design. Every decision here is arithmetic over the `renditions[]` the
 * server hydrates onto each sync record plus a snapshot of the network state —
 * no DOM, no Dexie, no fetch. That is deliberate: the picker's *behaviour* (what
 * it offers, what it defaults to on a metered link) is exactly the part that
 * cannot be verified by opening a video and looking at it, and this repo has no
 * jsdom to render the menu into. See `renditionChoice.test.ts`.
 *
 * ## The wire contract, restated so it is not re-derived at each call site
 *
 * - Renditions arrive **highest first**, `is_source` marking the untouched
 *   original. This module re-sorts anyway; see {@link offerableRenditions}.
 * - `short_edge` is both the rung's identity and the `?rendition=` selector, and
 *   because the ladder keys on the *short* edge it is also literally the "p"
 *   number a user expects — a portrait `1080x1920` is `1080`, not `1920`.
 * - **An empty array is the normal case.** It means one quality exists, so no
 *   picker should be drawn at all.
 */

/** One playable quality, as it arrives on a sync record. */
export interface Rendition {
  /** Rung identity, the `?rendition=` selector, and the "p" number. */
  short_edge: number;
  width: number;
  height: number;
  /** The untouched original. Never assume this is also the highest — sort. */
  is_source: boolean;
  /** Encrypted installs: `GET /api/blobs/{blob_id}`. Null on plaintext installs. */
  blob_id: string | null;
  codec: string | null;
  size_bytes: number;
}

/**
 * Ceiling applied when the connection looks expensive.
 *
 * 1080 rather than "one rung down" because the issue asks for a *quality* cap,
 * not a relative step: on a 8K source one rung down is 4K, which is not what
 * "lower on cellular" means to anybody.
 */
export const CONSTRAINED_MAX_SHORT_EDGE = 1080;

/** What we can learn about the link, from whatever APIs the browser exposes. */
export interface NetworkHint {
  /** `navigator.connection.saveData` — an explicit user request. */
  saveData?: boolean;
  /** `navigator.connection.effectiveType`. */
  effectiveType?: string;
  /** `navigator.connection.type` — only Chromium-on-Android reports this. */
  type?: string;
}

/** Effective types we treat as too slow for a full-quality stream. */
const SLOW_EFFECTIVE_TYPES = new Set(["slow-2g", "2g", "3g"]);

/**
 * Whether to default to a reduced quality.
 *
 * Three independent signals, ORed rather than ranked, because they fail in
 * different directions and none is reliably present: `saveData` is an explicit
 * instruction and outranks everything; `type` is the only *honest* wifi/cellular
 * answer but exists on one engine; `effectiveType` is widely available but
 * measures throughput, so a fast cellular link reads as `4g`.
 *
 * Absent all three — Safari and Firefox report no `connection` at all — this is
 * false and the caller defaults to highest. That is the right way to be wrong:
 * the issue's complaint is that no choice exists, and a desktop browser on a
 * fixed line is the common case for the one engine family that tells us nothing.
 */
export function isConstrainedNetwork(net: NetworkHint | undefined): boolean {
  if (!net) return false;
  if (net.saveData) return true;
  if (net.type === "cellular") return true;
  return !!net.effectiveType && SLOW_EFFECTIVE_TYPES.has(net.effectiveType);
}

/**
 * Read the live network state, or `undefined` where the API does not exist.
 *
 * The one impure function in this module, kept here so the rest stays testable
 * and so there is a single place that knows the API is non-standard.
 */
export function readNetworkHint(): NetworkHint | undefined {
  const conn = (
    navigator as Navigator & {
      connection?: { saveData?: boolean; effectiveType?: string; type?: string };
    }
  ).connection;
  if (!conn) return undefined;
  return {
    saveData: conn.saveData,
    effectiveType: conn.effectiveType,
    type: conn.type,
  };
}

/**
 * Normalise the server's list into what a picker may actually show.
 *
 * Sorts highest-first rather than trusting the server's `ORDER BY`, and drops
 * duplicate rungs keeping the first seen. Neither should ever happen — the
 * table's primary key is `(photo_id, short_edge)` and the query orders
 * descending — but "should never happen" is how a picker ends up offering the
 * same quality twice, and the cost of being sure is one sort of a 2-3 element
 * array.
 */
export function offerableRenditions(list: Rendition[] | undefined): Rendition[] {
  if (!list || list.length === 0) return [];
  const seen = new Set<number>();
  return [...list]
    .sort((a, b) => b.short_edge - a.short_edge)
    .filter((r) => {
      if (seen.has(r.short_edge)) return false;
      seen.add(r.short_edge);
      return true;
    });
}

/**
 * Whether to draw the gear icon at all.
 *
 * A one-entry picker is worse than no picker: it implies a choice the user does
 * not have, and it is the *normal* state for the overwhelming majority of the
 * library (only videos above the 1080p tier ever get a second rung).
 */
export function shouldOfferPicker(list: Rendition[] | undefined): boolean {
  return offerableRenditions(list).length >= 2;
}

/**
 * The rung to start playback on.
 *
 * Returns `undefined` for an empty list, which means "play the photo's own blob
 * exactly as the viewer did before #49" — not an error.
 */
export function chooseDefaultRendition(
  list: Rendition[] | undefined,
  net?: NetworkHint,
): Rendition | undefined {
  const offerable = offerableRenditions(list);
  if (offerable.length === 0) return undefined;
  if (!isConstrainedNetwork(net)) return offerable[0];

  // Highest rung within the cap. `find` over a descending list is that rung.
  const capped = offerable.find((r) => r.short_edge <= CONSTRAINED_MAX_SHORT_EDGE);
  // Nothing at or below the cap means every rung is huge (a 4K source whose
  // 1080 rung has not been produced yet). Take the smallest rather than
  // refusing to play — the alternative is a metered client fetching the 4K.
  return capped ?? offerable[offerable.length - 1];
}

/**
 * Whether the mirror's ladder differs from the one the server just sent.
 *
 * Exists to keep the sync reconcile honest about what actually changed.
 * **`undefined` and `[]` are the same state** — "this video has one quality" —
 * and they arrive from different places: a pre-#49 server (and every row cached
 * before this field existed) yields `undefined`, while a #49 server sends `[]`
 * for the ~600 videos that need no rung. Treating those as different makes the
 * first sync pass against an upgraded server rewrite the entire library, and
 * every pass after it rewrite every video — which is the exact O(library) write
 * amplification #38 spent a workstream removing.
 *
 * Order-sensitive on purpose: {@link offerableRenditions} sorts before display,
 * so a reordering is not a semantic change, but the server emits a stable order
 * and a cheap positional compare is enough to catch every real one.
 */
export function renditionsEqual(
  a: Rendition[] | undefined,
  b: Rendition[] | undefined,
): boolean {
  const left = a ?? [];
  const right = b ?? [];
  if (left.length !== right.length) return false;
  return left.every((x, i) => {
    const y = right[i];
    return (
      x.short_edge === y.short_edge &&
      x.width === y.width &&
      x.height === y.height &&
      x.is_source === y.is_source &&
      x.blob_id === y.blob_id &&
      x.codec === y.codec &&
      x.size_bytes === y.size_bytes
    );
  });
}

/**
 * Menu label for a rung.
 *
 * The resolution is shown even for the original: "Original" alone forces the
 * user to guess whether it is bigger than the 1080p entry below it.
 */
export function formatRenditionLabel(r: Rendition): string {
  return r.is_source ? `Original (${r.short_edge}p)` : `${r.short_edge}p`;
}
