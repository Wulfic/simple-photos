/**
 * Photo-blob envelope decoding — format-aware over both blob layouts.
 *
 * The server stores a photo's media in one of two encrypted containers:
 *
 *  - **v1 (monolithic):** `AES-GCM( {"v":1, ...metadata..., "data": base64(bytes)} )`
 *    — one AES-GCM message; the whole file is base64 inside the JSON.
 *  - **v2 (chunked):** a `SPCHNKB2` container of an encrypted metadata frame
 *    followed by length-prefixed AES-GCM chunk frames (see server
 *    `blobs/chunked.rs`). Used for large videos so neither the server nor the
 *    client has to hold a multi-gigabyte base64 string in memory at once.
 *
 * [`decryptPhotoBlob`] detects the format from the downloaded bytes (the v2
 * magic prefix) — robust even when the `X-Blob-Format` response header is
 * stripped by a proxy or hidden by CORS — and returns the parsed metadata plus
 * the raw media bytes either way.
 */
import { decrypt } from "./crypto";
import { base64ToUint8Array } from "../utils/encoding";
import type { MediaPayload } from "../types/media";

/** Envelope metadata. Identical fields across v1/v2; v2 omits the inline `data`. */
export type BlobEnvelopeMeta = Omit<MediaPayload, "data"> & { data?: string };

export interface DecryptedPhotoBlob {
  /** Parsed envelope metadata (filename, mime_type, media_type, …). */
  payload: BlobEnvelopeMeta;
  /** Raw media bytes — an exact-sized array that owns its buffer. */
  bytes: Uint8Array;
}

/** v2 magic prefix — must match `MAGIC` in server `blobs/chunked.rs`. */
const CHUNK_MAGIC = [0x53, 0x50, 0x43, 0x48, 0x4e, 0x4b, 0x42, 0x32]; // "SPCHNKB2"

/** `true` if `buf` begins with the v2 chunked-blob magic prefix. */
export function isChunkedBlob(buf: Uint8Array): boolean {
  if (buf.byteLength < CHUNK_MAGIC.length) return false;
  for (let i = 0; i < CHUNK_MAGIC.length; i++) {
    if (buf[i] !== CHUNK_MAGIC[i]) return false;
  }
  return true;
}

/**
 * Decrypt a downloaded photo blob (either format) into its metadata + a single
 * contiguous media `Uint8Array`. Prefer [`decryptPhotoBlobToBlob`] for playback:
 * it builds the `Blob` from per-chunk parts and avoids one giant contiguous
 * allocation for multi-gigabyte videos.
 */
export async function decryptPhotoBlob(encrypted: ArrayBuffer): Promise<DecryptedPhotoBlob> {
  const buf = new Uint8Array(encrypted);

  if (isChunkedBlob(buf)) {
    const { payload, parts } = await decryptChunkedToParts(encrypted, buf);
    let total = 0;
    for (const p of parts) total += p.byteLength;
    const bytes = new Uint8Array(total);
    let off = 0;
    for (const p of parts) {
      bytes.set(p, off);
      off += p.byteLength;
    }
    return { payload, bytes };
  }

  // v1 monolithic envelope.
  const decrypted = await decrypt(encrypted);
  const payload = JSON.parse(new TextDecoder().decode(decrypted)) as BlobEnvelopeMeta;
  const bytes = payload.data ? base64ToUint8Array(payload.data) : new Uint8Array(0);
  return { payload, bytes };
}

/**
 * Decrypt a downloaded photo blob (either format) directly into a `Blob`.
 *
 * For v2 the chunk frames are decrypted and handed to the `Blob` constructor as
 * separate parts, so the media is never held as one giant contiguous array on
 * the JS heap — the browser stores the parts in the Blob's backing store. This
 * is the path playback should use for large videos.
 */
export async function decryptPhotoBlobToBlob(
  encrypted: ArrayBuffer,
  fallbackMime = "application/octet-stream",
): Promise<{ payload: BlobEnvelopeMeta; blob: Blob }> {
  const buf = new Uint8Array(encrypted);

  if (isChunkedBlob(buf)) {
    const { payload, parts } = await decryptChunkedToParts(encrypted, buf);
    const blob = new Blob(parts as BlobPart[], { type: payload.mime_type || fallbackMime });
    return { payload, blob };
  }

  // v1 monolithic envelope.
  const decrypted = await decrypt(encrypted);
  const payload = JSON.parse(new TextDecoder().decode(decrypted)) as BlobEnvelopeMeta;
  const bytes = payload.data ? base64ToUint8Array(payload.data) : new Uint8Array(0);
  const blob = new Blob([bytes as BlobPart], { type: payload.mime_type || fallbackMime });
  return { payload, blob };
}

/**
 * Decrypt only the envelope metadata, without reconstructing the media bytes.
 *
 * For v2 this decrypts just the leading metadata frame — so reading a video's
 * filename/dimensions doesn't decrypt gigabytes of chunk frames. For v1 the
 * whole message must be decrypted (metadata and data share one AES-GCM blob),
 * so prefer [`decryptPhotoBlob`] there if the bytes are needed anyway.
 */
export async function decryptBlobMetadata<T = BlobEnvelopeMeta>(
  encrypted: ArrayBuffer,
): Promise<T> {
  const buf = new Uint8Array(encrypted);

  if (isChunkedBlob(buf)) {
    const dv = new DataView(encrypted);
    let cur = CHUNK_MAGIC.length;
    if (cur + 4 > buf.byteLength) throw new Error("truncated chunked blob (length prefix)");
    const metaLen = dv.getUint32(cur, false);
    cur += 4;
    if (cur + metaLen > buf.byteLength) throw new Error("truncated chunked blob (metadata)");
    const metaFrame = encrypted.slice(cur, cur + metaLen);
    const metaPlain = await decrypt(metaFrame);
    return JSON.parse(new TextDecoder().decode(metaPlain)) as T;
  }

  const decrypted = await decrypt(encrypted);
  return JSON.parse(new TextDecoder().decode(decrypted)) as T;
}

/** Decrypt a v2 chunked blob into its metadata + the per-chunk plaintext parts. */
async function decryptChunkedToParts(
  encrypted: ArrayBuffer,
  buf: Uint8Array,
): Promise<{ payload: BlobEnvelopeMeta; parts: Uint8Array[] }> {
  const dv = new DataView(encrypted);
  let cur = CHUNK_MAGIC.length;

  // Length prefixes are big-endian u32 (server writes `to_be_bytes()`).
  const readLen = (): number => {
    if (cur + 4 > buf.byteLength) throw new Error("truncated chunked blob (length prefix)");
    const n = dv.getUint32(cur, false);
    cur += 4;
    return n;
  };
  const slice = (len: number): ArrayBuffer => {
    if (cur + len > buf.byteLength) throw new Error("truncated chunked blob (frame body)");
    const out = encrypted.slice(cur, cur + len);
    cur += len;
    return out;
  };

  // Leading metadata frame.
  const metaFrame = slice(readLen());
  const metaPlain = await decrypt(metaFrame);
  const payload = JSON.parse(new TextDecoder().decode(metaPlain)) as BlobEnvelopeMeta;

  // Remaining chunk frames.
  const parts: Uint8Array[] = [];
  while (cur < buf.byteLength) {
    const frame = slice(readLen());
    parts.push(await decrypt(frame));
  }

  return { payload, parts };
}
