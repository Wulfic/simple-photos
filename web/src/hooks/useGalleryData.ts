import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, hasCryptoKey } from "../crypto/crypto";
import {
  db,
  type CachedPhoto,
  type MediaType,
  mediaTypeFromMime,
} from "../db";
import { base64ToArrayBuffer } from "../utils/media";
import { type PlainPhoto, fetchAllPages } from "../utils/gallery";
import { useLiveQuery } from "dexie-react-hooks";

// ── Encrypted payload shapes (shared with upload hook) ───────────────────────

export interface PhotoPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type: "photo" | "gif" | "video" | "audio";
  width: number;
  height: number;
  duration?: number;
  latitude?: number;
  longitude?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64
}

export interface ThumbnailPayload {
  v: number;
  photo_blob_id: string;
  width: number;
  height: number;
  data: string; // base64 JPEG
}

export type EncryptionMode = "plain" | "encrypted";

export interface GalleryDataResult {
  mode: EncryptionMode | null;
  loading: boolean;
  error: string;
  setError: (msg: string) => void;
  plainPhotos: PlainPhoto[];
  /** Encrypted-mode photos from IndexedDB (live query, auto-updates). */
  encryptedPhotos: CachedPhoto[] | undefined;
  secureBlobIds: Set<string>;
  migrationStatus: string;
  migrationTotal: number;
  migrationCompleted: number;
  setMigrationStatus: (s: string) => void;
  setMigrationTotal: (n: number) => void;
  setMigrationCompleted: (n: number) => void;
  loadPlainPhotos: () => Promise<void>;
  loadEncryptedPhotos: () => Promise<void>;
}

/**
 * Core data hook for the Gallery page.
 *
 * Handles mode detection (plain vs. encrypted), initial data loading,
 * and provides loaders for both plain and encrypted photo lists.
 */
export function useGalleryData(): GalleryDataResult {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [mode, setMode] = useState<EncryptionMode | null>(null);
  const [plainPhotos, setPlainPhotos] = useState<PlainPhoto[]>([]);
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());
  const [migrationStatus, setMigrationStatus] = useState("idle");
  const [migrationTotal, setMigrationTotal] = useState(0);
  const [migrationCompleted, setMigrationCompleted] = useState(0);
  const navigate = useNavigate();

  // Encrypted-mode: cached photos from IndexedDB (live query)
  const encryptedPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  // ── Initialization ──────────────────────────────────────────────────────

  useEffect(() => {
    async function init() {
      try {
        const settings = await api.encryption.getSettings();
        const detected = settings.encryption_mode as EncryptionMode;
        setMode(detected);

        // Track migration state so the gallery can run the migration worker
        setMigrationStatus(settings.migration_status);
        setMigrationTotal(settings.migration_total);
        setMigrationCompleted(settings.migration_completed);

        // Fetch blob IDs that are in secure galleries (to hide from main gallery)
        try {
          const secureRes = await api.secureGalleries.secureBlobIds();
          setSecureBlobIds(new Set(secureRes.blob_ids));
        } catch {
          // Secure galleries may not be available — ignore
        }

        // Fire auto-scan in the background — don't block photo loading.
        // When it completes, reload photos so newly scanned files appear.
        const reloadAfterScan = detected === "encrypted" ? loadEncryptedPhotos : loadPlainPhotos;
        api.backup.triggerAutoScan()
          .then(() => reloadAfterScan())
          .catch(() => {
            // Non-critical — if the user isn't admin or endpoint fails, just ignore
          });

        if (detected === "encrypted") {
          if (!hasCryptoKey()) {
            navigate("/setup");
            return;
          }
          await loadEncryptedPhotos();
        } else {
          await loadPlainPhotos();
        }
      } catch {
        // Fallback: if encryption settings endpoint doesn't exist yet, assume encrypted (legacy)
        setMode("encrypted");
        if (!hasCryptoKey()) {
          navigate("/setup");
          return;
        }
        await loadEncryptedPhotos();
      }
    }
    init();
  }, []);

  // ── Load plain-mode photos from server ─────────────────────────────────

  async function loadPlainPhotos() {
    setLoading(true);
    try {
      const allPhotos: PlainPhoto[] = [];
      let cursor: string | undefined;
      do {
        const res = await api.photos.list({ after: cursor, limit: 200 });
        allPhotos.push(...res.photos);
        cursor = res.next_cursor ?? undefined;
      } while (cursor);

      setPlainPhotos(allPhotos.sort((a, b) => {
        // Sort by taken_at descending, fallback to created_at
        // Secondary sort by filename ASC for deterministic ordering (matches server + app)
        const aDate = a.taken_at || a.created_at;
        const bDate = b.taken_at || b.created_at;
        const dateCmp = bDate.localeCompare(aDate);
        if (dateCmp !== 0) return dateCmp;
        return (a.filename || "").localeCompare(b.filename || "");
      }));
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load photos");
    } finally {
      setLoading(false);
    }
  }

  // ── Load encrypted-mode photos from blobs + IndexedDB ──────────────────

  async function loadEncryptedPhotos() {
    setLoading(true);
    try {
      // Phase 1: Fetch metadata via encrypted-sync endpoint
      // Uses the same server-side data source as the Android app, ensuring
      // consistent sort order (COALESCE(taken_at, created_at) DESC).
      type SyncRecord = Awaited<ReturnType<typeof api.photos.encryptedSync>>["photos"][number];
      const allSyncPhotos: SyncRecord[] = [];
      let cursor: string | undefined;
      do {
        const res = await api.photos.encryptedSync({ after: cursor, limit: 500 });
        allSyncPhotos.push(...res.photos);
        cursor = res.next_cursor ?? undefined;
      } while (cursor);

      // Build set of server blob IDs for stale-entry cleanup
      const serverBlobIds = new Set(
        allSyncPhotos.map((p) => p.encrypted_blob_id).filter(Boolean) as string[]
      );

      // Phase 2: Also fetch blobs list for directly-uploaded encrypted
      // photos (not yet in photos table — no encrypted_blob_id link).
      const allBlobMedia = [
        ...(await fetchAllPages("photo")),
        ...(await fetchAllPages("gif")),
        ...(await fetchAllPages("video")),
      ];
      for (const blob of allBlobMedia) {
        serverBlobIds.add(blob.id);
      }

      // Remove stale entries from IndexedDB that no longer exist on server
      const cachedPhotos = await db.photos.toArray();
      const staleIds = cachedPhotos
        .map((p) => p.blobId)
        .filter((id) => !serverBlobIds.has(id));
      if (staleIds.length > 0) {
        await db.photos.bulkDelete(staleIds);
      }

      // Phase 3: Populate IndexedDB from sync records (migrated photos).
      // Only download small thumbnail blobs (~30 KB) — not full photos.
      for (const photo of allSyncPhotos) {
        const blobId = photo.encrypted_blob_id;
        if (!blobId) continue;

        const existing = await db.photos.get(blobId);
        if (existing) continue;

        // Parse takenAt: prefer taken_at, fall back to created_at (matches Android)
        let takenAt: number;
        try {
          takenAt = photo.taken_at
            ? new Date(photo.taken_at).getTime()
            : new Date(photo.created_at).getTime();
        } catch {
          takenAt = new Date(photo.created_at).getTime();
        }

        // Download and decrypt thumbnail blob if available
        let thumbnailData: ArrayBuffer | undefined;
        const thumbBlobId = photo.encrypted_thumb_blob_id;
        if (thumbBlobId) {
          try {
            const thumbEnc = await api.blobs.download(thumbBlobId);
            const thumbDec = await decrypt(thumbEnc);
            const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
            thumbnailData = base64ToArrayBuffer(thumbPayload.data);
          } catch {
            // Thumbnail fetch failed — show placeholder
          }
        }

        await db.photos.put({
          blobId,
          thumbnailBlobId: thumbBlobId ?? undefined,
          filename: photo.filename,
          takenAt,
          mimeType: photo.mime_type,
          mediaType: (photo.media_type as MediaType) ?? mediaTypeFromMime(photo.mime_type),
          width: photo.width,
          height: photo.height,
          duration: photo.duration_secs ?? undefined,
          albumIds: [],
          thumbnailData,
          contentHash: photo.photo_hash ?? undefined,
        });
      }

      // Phase 4: Handle directly-uploaded encrypted blobs not in photos table.
      // These require full blob decryption to extract metadata.
      const syncedBlobIds = new Set(
        allSyncPhotos.map((p) => p.encrypted_blob_id).filter(Boolean)
      );
      const unsyncedBlobs = allBlobMedia.filter((b) => !syncedBlobIds.has(b.id));

      for (const blob of unsyncedBlobs) {
        const existing = await db.photos.get(blob.id);
        if (existing) continue;

        try {
          const encrypted = await api.blobs.download(blob.id);
          const decrypted = await decrypt(encrypted);
          const payload: PhotoPayload = JSON.parse(new TextDecoder().decode(decrypted));

          let thumbnailData: ArrayBuffer | undefined;
          if (payload.thumbnail_blob_id) {
            try {
              const thumbEnc = await api.blobs.download(payload.thumbnail_blob_id);
              const thumbDec = await decrypt(thumbEnc);
              const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
              thumbnailData = base64ToArrayBuffer(thumbPayload.data);
            } catch {
              // Thumbnail fetch failed — show placeholder
            }
          }

          await db.photos.put({
            blobId: blob.id,
            thumbnailBlobId: payload.thumbnail_blob_id,
            filename: payload.filename,
            takenAt: new Date(payload.taken_at).getTime(),
            mimeType: payload.mime_type,
            mediaType: payload.media_type ?? mediaTypeFromMime(payload.mime_type),
            width: payload.width,
            height: payload.height,
            duration: payload.duration,
            latitude: payload.latitude,
            longitude: payload.longitude,
            albumIds: payload.album_ids ?? [],
            thumbnailData,
            contentHash: blob.content_hash ?? undefined,
          });
        } catch {
          // Skip items we can't decrypt (wrong key or corrupt blob)
        }
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load");
    } finally {
      setLoading(false);
    }
  }

  return {
    mode,
    loading,
    error,
    setError,
    plainPhotos,
    encryptedPhotos,
    secureBlobIds,
    migrationStatus,
    migrationTotal,
    migrationCompleted,
    setMigrationStatus,
    setMigrationTotal,
    setMigrationCompleted,
    loadPlainPhotos,
    loadEncryptedPhotos,
  };
}
