/**
 * Migration Web Worker
 *
 * Runs the encryption migration loop in a dedicated worker thread so that
 * browser tab-throttling does NOT pause the download → encrypt → upload
 * pipeline. The main thread sends a "start" message with the auth token,
 * encryption key, and the list of plain photos. The worker processes each
 * one and posts progress back.
 *
 * Why a Worker?
 * Modern browsers aggressively throttle background tabs — setTimeout/fetch
 * on the main thread can stall or slow to once-per-minute. Workers are
 * NOT subject to the same throttling, so encryption continues at full
 * speed regardless of whether the user is looking at the tab.
 */

// ── Types ───────────────────────────────────────────────────────────────────

interface PlainPhoto {
  id: string;
  filename: string;
  file_path: string;
  mime_type: string;
  media_type: string;
  size_bytes: number;
  width: number;
  height: number;
  duration_secs: number | null;
  taken_at: string | null;
  latitude: number | null;
  longitude: number | null;
  thumb_path: string | null;
  created_at: string;
}

interface StartMessage {
  type: "start";
  accessToken: string;
  refreshToken: string;
  keyHex: string;
  photos: PlainPhoto[];
}

interface TokenRefreshMessage {
  type: "tokenRefresh";
  accessToken: string;
  refreshToken: string;
}

type InMessage = StartMessage | TokenRefreshMessage;

// Outbound messages
interface ProgressMessage {
  type: "progress";
  completed: number;
  total: number;
  succeeded: number;
  failed: number;
  currentFile: string;
}

interface DoneMessage {
  type: "done";
  succeeded: number;
  failed: number;
  total: number;
  lastError: string;
}

interface ErrorMessage {
  type: "error";
  message: string;
}

interface TokenRequestMessage {
  type: "needToken";
}

type OutMessage = ProgressMessage | DoneMessage | ErrorMessage | TokenRequestMessage;

// ── Globals ─────────────────────────────────────────────────────────────────

let currentAccessToken = "";
let currentRefreshToken = "";

// ── Crypto helpers (worker-safe, no sessionStorage) ─────────────────────────

const NONCE_LENGTH = 12;

function hexToArray(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    bytes[i / 2] = parseInt(hex.substring(i, i + 2), 16);
  }
  return bytes;
}

function arrayToHex(arr: Uint8Array): string {
  return Array.from(arr)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function arrayBufferToBase64(buffer: ArrayBuffer | Uint8Array): string {
  const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  const CHUNK = 32768;
  const parts: string[] = [];
  for (let i = 0; i < bytes.byteLength; i += CHUNK) {
    const slice = bytes.subarray(i, Math.min(i + CHUNK, bytes.byteLength));
    parts.push(String.fromCharCode(...slice));
  }
  return btoa(parts.join(""));
}

async function workerEncrypt(
  plaintext: Uint8Array,
  cryptoKey: CryptoKey
): Promise<ArrayBuffer> {
  const nonce = crypto.getRandomValues(new Uint8Array(NONCE_LENGTH));
  const ciphertext = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: nonce as BufferSource },
    cryptoKey,
    plaintext as BufferSource
  );
  const result = new Uint8Array(NONCE_LENGTH + ciphertext.byteLength);
  result.set(nonce, 0);
  result.set(new Uint8Array(ciphertext), NONCE_LENGTH);
  return result.buffer;
}

async function workerSha256Hex(data: Uint8Array): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", data as BufferSource);
  return arrayToHex(new Uint8Array(hash));
}

// ── API helpers ─────────────────────────────────────────────────────────────

const BASE = "/api";

async function apiFetch(
  path: string,
  init: RequestInit = {}
): Promise<Response> {
  const headers: Record<string, string> = {
    ...(init.headers as Record<string, string> || {}),
    "X-Requested-With": "SimplePhotos",
  };
  if (currentAccessToken) {
    headers["Authorization"] = `Bearer ${currentAccessToken}`;
  }
  if (init.body && typeof init.body === "string") {
    headers["Content-Type"] = "application/json";
  }

  let res = await fetch(`${BASE}${path}`, { ...init, headers });

  // Handle 401 — try refreshing the token
  if (res.status === 401 && currentRefreshToken) {
    const refreshed = await tryRefreshToken();
    if (refreshed) {
      headers["Authorization"] = `Bearer ${currentAccessToken}`;
      res = await fetch(`${BASE}${path}`, { ...init, headers });
    }
  }

  return res;
}

async function tryRefreshToken(): Promise<boolean> {
  try {
    const res = await fetch(`${BASE}/auth/refresh`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Requested-With": "SimplePhotos",
      },
      body: JSON.stringify({ refresh_token: currentRefreshToken }),
    });
    if (!res.ok) return false;
    const data = await res.json();
    currentAccessToken = data.access_token;
    currentRefreshToken = data.refresh_token;
    // Notify main thread of new tokens
    self.postMessage({
      type: "tokenUpdate",
      accessToken: data.access_token,
      refreshToken: data.refresh_token,
    });
    return true;
  } catch {
    return false;
  }
}

async function downloadPhoto(photoId: string): Promise<ArrayBuffer> {
  const res = await apiFetch(`/photos/${photoId}/file`);
  if (!res.ok) throw new Error(`Download failed: ${res.status}`);
  return res.arrayBuffer();
}

async function uploadBlob(
  data: ArrayBuffer,
  blobType: string,
  clientHash: string,
  contentHash?: string
): Promise<{ blob_id: string }> {
  const headers: Record<string, string> = {
    "X-Blob-Type": blobType,
    "X-Blob-Size": data.byteLength.toString(),
  };
  if (clientHash) headers["X-Client-Hash"] = clientHash;
  if (contentHash) headers["X-Content-Hash"] = contentHash;

  const res = await apiFetch("/blobs", {
    method: "POST",
    headers,
    body: data,
  });
  if (!res.ok) {
    const errText = await res.text().catch(() => "");
    throw new Error(`Upload failed: ${res.status} ${errText}`);
  }
  return res.json();
}

async function markEncrypted(
  photoId: string,
  blobId: string,
  thumbBlobId?: string
): Promise<void> {
  const res = await apiFetch(`/photos/${photoId}/mark-encrypted`, {
    method: "POST",
    body: JSON.stringify({
      blob_id: blobId,
      thumb_blob_id: thumbBlobId || null,
    }),
  });
  if (!res.ok) throw new Error(`markEncrypted failed: ${res.status}`);
}

async function reportProgress(payload: {
  completed_count: number;
  done?: boolean;
  error?: string;
}): Promise<void> {
  await apiFetch("/encryption/progress", {
    method: "POST",
    body: JSON.stringify(payload),
  }).catch(() => {});
}

// ── Thumbnail generation (using OffscreenCanvas in Worker) ──────────────────

function blobTypeFromMime(mimeType: string): string {
  if (mimeType === "image/gif") return "gif";
  if (mimeType.startsWith("video/")) return "video";
  if (mimeType.startsWith("audio/")) return "audio";
  return "photo";
}

async function generateThumbnailInWorker(
  data: Uint8Array,
  mimeType: string,
  size: number
): Promise<ArrayBuffer | null> {
  // Skip video thumbnails in worker (no HTMLVideoElement); skip non-image types
  if (mimeType.startsWith("video/")) return null;

  try {
    const blob = new Blob([data.buffer as ArrayBuffer], { type: mimeType });
    const imageBitmap = await createImageBitmap(blob);
    const canvas = new OffscreenCanvas(size, size);
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;

    // Cover-fit (center crop)
    const scale = Math.max(size / imageBitmap.width, size / imageBitmap.height);
    const sw = size / scale;
    const sh = size / scale;
    const sx = (imageBitmap.width - sw) / 2;
    const sy = (imageBitmap.height - sh) / 2;
    ctx.drawImage(imageBitmap, sx, sy, sw, sh, 0, 0, size, size);
    imageBitmap.close();

    const outputBlob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.8 });
    return outputBlob.arrayBuffer();
  } catch {
    return null;
  }
}

// ── Main migration loop ─────────────────────────────────────────────────────

async function runMigration(
  cryptoKey: CryptoKey,
  photos: PlainPhoto[]
): Promise<void> {
  const total = photos.length;
  let completed = 0;
  let succeeded = 0;
  let failedCount = 0;
  let lastError = "";

  console.log(`[Worker Migration] Starting: ${total} photos to encrypt`);

  for (const photo of photos) {
    let attempts = 0;
    let itemSuccess = false;

    while (attempts < 3 && !itemSuccess) {
      attempts++;
      try {
        // Step 1: Download the raw file
        const fileBuffer = await downloadPhoto(photo.id);
        const fileData = new Uint8Array(fileBuffer);

        // Step 2: Generate thumbnail
        let thumbBlobId: string | undefined;
        try {
          const thumbData = await generateThumbnailInWorker(fileData, photo.mime_type, 256);
          if (thumbData) {
            const thumbPayload = JSON.stringify({
              v: 1,
              photo_blob_id: "",
              width: 256,
              height: 256,
              data: arrayBufferToBase64(thumbData),
            });
            const encThumb = await workerEncrypt(
              new TextEncoder().encode(thumbPayload),
              cryptoKey
            );
            const thumbHash = await workerSha256Hex(new Uint8Array(encThumb));
            const thumbBlobType =
              photo.media_type === "video" ? "video_thumbnail" : "thumbnail";
            const thumbUpload = await uploadBlob(
              encThumb,
              thumbBlobType,
              thumbHash
            );
            thumbBlobId = thumbUpload.blob_id;
          }
        } catch (thumbErr: any) {
          console.warn(
            `[Worker Migration] Thumbnail failed for "${photo.filename}":`,
            thumbErr.message
          );
        }

        // Step 3: Build and encrypt photo payload
        const serverBlobType = blobTypeFromMime(photo.mime_type);
        const photoPayload = JSON.stringify({
          v: 1,
          filename: photo.filename,
          taken_at: photo.taken_at || photo.created_at,
          mime_type: photo.mime_type,
          media_type: photo.media_type || (photo.mime_type === "image/gif" ? "gif" : photo.mime_type.startsWith("video/") ? "video" : photo.mime_type.startsWith("audio/") ? "audio" : "photo"),
          width: photo.width,
          height: photo.height,
          duration: photo.duration_secs ?? undefined,
          latitude: photo.latitude ?? undefined,
          longitude: photo.longitude ?? undefined,
          album_ids: [],
          thumbnail_blob_id: thumbBlobId || "",
          data: arrayBufferToBase64(fileData),
        });

        const encPhoto = await workerEncrypt(
          new TextEncoder().encode(photoPayload),
          cryptoKey
        );
        const photoHash = await workerSha256Hex(new Uint8Array(encPhoto));
        const contentHash = (
          await workerSha256Hex(new Uint8Array(fileData))
        ).substring(0, 12);

        // Step 4: Upload encrypted blob
        const uploadResult = await uploadBlob(
          encPhoto,
          serverBlobType,
          photoHash,
          contentHash
        );

        // Step 5: Link blob to the plain photo (including thumbnail blob)
        await markEncrypted(photo.id, uploadResult.blob_id, thumbBlobId);

        itemSuccess = true;
        succeeded++;
      } catch (itemErr: any) {
        console.error(
          `[Worker Migration] FAILED "${photo.filename}" attempt ${attempts}/3:`,
          itemErr.message
        );
        if (attempts >= 3) {
          failedCount++;
          lastError = `Failed on "${photo.filename}": ${itemErr.message}`;
        } else {
          // Brief pause before retry
          await new Promise((r) => setTimeout(r, 500 * attempts));
        }
      }
    }

    completed++;

    // Report progress to server
    await reportProgress({
      completed_count: completed,
      ...(!itemSuccess ? { error: lastError } : {}),
    });

    // Report progress to main thread
    const msg: ProgressMessage = {
      type: "progress",
      completed,
      total,
      succeeded,
      failed: failedCount,
      currentFile: photo.filename,
    };
    self.postMessage(msg);
  }

  console.log(
    `[Worker Migration] Complete: ${succeeded} succeeded, ${failedCount} failed / ${total}`
  );

  // Mark migration complete on server
  await reportProgress({
    completed_count: total,
    done: true,
    ...(failedCount > 0
      ? {
          error: `Migration finished with ${failedCount}/${total} failures. Last: ${lastError}`,
        }
      : {}),
  });

  const doneMsg: DoneMessage = {
    type: "done",
    succeeded,
    failed: failedCount,
    total,
    lastError,
  };
  self.postMessage(doneMsg);
}

// ── Message handler ─────────────────────────────────────────────────────────

self.onmessage = async (e: MessageEvent<InMessage>) => {
  const msg = e.data;

  if (msg.type === "tokenRefresh") {
    currentAccessToken = msg.accessToken;
    currentRefreshToken = msg.refreshToken;
    return;
  }

  if (msg.type === "start") {
    currentAccessToken = msg.accessToken;
    currentRefreshToken = msg.refreshToken;

    try {
      // Import the raw key into a CryptoKey for AES-GCM
      const keyBytes = hexToArray(msg.keyHex);
      const cryptoKey = await crypto.subtle.importKey(
        "raw",
        keyBytes.buffer as ArrayBuffer,
        { name: "AES-GCM" },
        false,
        ["encrypt", "decrypt"]
      );

      await runMigration(cryptoKey, msg.photos);
    } catch (err: any) {
      console.error("[Worker Migration] Fatal:", err);
      const errMsg: ErrorMessage = {
        type: "error",
        message: err.message || "Migration worker failed",
      };
      self.postMessage(errMsg);
    }
  }
};
