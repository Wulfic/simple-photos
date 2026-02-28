import { useRef, useEffect } from "react";
import { api } from "../api/client";
import { encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import { blobTypeFromMime, mediaTypeFromMime } from "../db";
import { arrayBufferToBase64, generateMigrationThumbnail } from "../utils/media";

export function useMigrationWorker(
  migrationStatus: string,
  loadEncryptionSettings: () => Promise<void>,
) {
  const migrationRunningRef = useRef(false);

  useEffect(() => {
    if (migrationStatus !== "encrypting") return;
    if (migrationRunningRef.current) return;
    if (!hasCryptoKey()) return;

    migrationRunningRef.current = true;

    (async () => {
      try {
        console.log("[Migration] Starting encryption migration worker...");

        // Fetch ALL plain photos that need encrypting
        const allPhotos: Array<{
          id: string; filename: string; file_path: string; mime_type: string;
          media_type: string; size_bytes: number; width: number; height: number;
          duration_secs: number | null; taken_at: string | null;
          latitude: number | null; longitude: number | null;
          thumb_path: string | null; created_at: string;
        }> = [];
        let cursor: string | undefined;
        do {
          const res = await api.photos.list({ after: cursor, limit: 200 });
          allPhotos.push(...res.photos);
          cursor = res.next_cursor ?? undefined;
        } while (cursor);

        console.log(`[Migration] Found ${allPhotos.length} photos to encrypt`);

        const total = allPhotos.length;
        let completed = 0;
        let succeeded = 0;
        let failedCount = 0;
        let lastError = "";

        for (const photo of allPhotos) {
          // Retry each photo up to 3 times to handle transient network/auth issues
          let attempts = 0;
          let itemSuccess = false;

          while (attempts < 3 && !itemSuccess) {
            attempts++;
            try {
              const stepStart = Date.now();

              // Step 1: Download the raw file
              console.log(`[Migration] [${completed + 1}/${total}] "${photo.filename}" (${(photo.size_bytes / 1024).toFixed(0)} KB) — downloading (attempt ${attempts}/3)...`);
              const fileBuffer = await api.photos.downloadFile(photo.id);
              const fileData = new Uint8Array(fileBuffer);
              console.log(`[Migration]   Downloaded: ${fileData.length} bytes in ${Date.now() - stepStart}ms`);

              // Step 2: Generate thumbnail
              let thumbBlobId: string | undefined;
              try {
                const thumbStart = Date.now();
                const thumbData = await generateMigrationThumbnail(fileData, photo.mime_type, 256);
                if (thumbData) {
                  const thumbPayload = JSON.stringify({
                    v: 1,
                    photo_blob_id: "",
                    width: 256,
                    height: 256,
                    data: arrayBufferToBase64(thumbData),
                  });
                  const encThumb = await encrypt(new TextEncoder().encode(thumbPayload));
                  const thumbHash = await sha256Hex(new Uint8Array(encThumb));
                  const thumbBlobType = photo.media_type === "video" ? "video_thumbnail" : "thumbnail";
                  console.log(`[Migration]   Uploading encrypted thumbnail (${encThumb.byteLength} bytes, type=${thumbBlobType})...`);
                  const thumbUpload = await api.blobs.upload(encThumb, thumbBlobType, thumbHash);
                  thumbBlobId = thumbUpload.blob_id;
                  console.log(`[Migration]   Thumbnail uploaded: ${thumbBlobId} in ${Date.now() - thumbStart}ms`);
                } else {
                  console.log(`[Migration]   Thumbnail generation returned null (skipping)`);
                }
              } catch (thumbErr: any) {
                console.warn(`[Migration]   Thumbnail generation/upload failed (continuing without): ${thumbErr.message}`, thumbErr);
              }

              // Step 3: Build and encrypt photo payload
              const serverBlobType = blobTypeFromMime(photo.mime_type);
              const photoPayload = JSON.stringify({
                v: 1,
                filename: photo.filename,
                taken_at: photo.taken_at || photo.created_at,
                mime_type: photo.mime_type,
                media_type: (photo.media_type || mediaTypeFromMime(photo.mime_type)) as "photo" | "gif" | "video",
                width: photo.width,
                height: photo.height,
                duration: photo.duration_secs ?? undefined,
                latitude: photo.latitude ?? undefined,
                longitude: photo.longitude ?? undefined,
                album_ids: [],
                thumbnail_blob_id: thumbBlobId || "",
                data: arrayBufferToBase64(fileData),
              });

              const payloadBytes = new TextEncoder().encode(photoPayload);
              console.log(`[Migration]   Payload: ${(payloadBytes.length / 1024).toFixed(0)} KB (base64 inflated from ${(fileData.length / 1024).toFixed(0)} KB raw)`);

              const encStart = Date.now();
              const encPhoto = await encrypt(payloadBytes);
              console.log(`[Migration]   Encrypted: ${encPhoto.byteLength} bytes in ${Date.now() - encStart}ms`);

              const photoHash = await sha256Hex(new Uint8Array(encPhoto));

              // Step 4: Upload encrypted blob
              const uploadStart = Date.now();
              console.log(`[Migration]   Uploading encrypted photo (${encPhoto.byteLength} bytes, type=${serverBlobType}, hash=${photoHash.substring(0, 12)}...)...`);
              const uploadResult = await api.blobs.upload(encPhoto, serverBlobType, photoHash);
              console.log(`[Migration]   Upload complete in ${Date.now() - uploadStart}ms (total: ${Date.now() - stepStart}ms)`);

              // Step 5: Link the blob to the plain photo so it won't be re-migrated
              await api.photos.markEncrypted(photo.id, uploadResult.blob_id);
              console.log(`[Migration]   Linked photo ${photo.id} → blob ${uploadResult.blob_id}`);

              itemSuccess = true;
              succeeded++;
            } catch (itemErr: any) {
              console.error(`[Migration]   FAILED "${photo.filename}" attempt ${attempts}/3:`, itemErr.message, itemErr.stack);
              if (attempts >= 3) {
                failedCount++;
                lastError = `Failed on "${photo.filename}": ${itemErr.message}`;
                console.error(`[Migration]   GIVING UP on "${photo.filename}" after 3 attempts. Error: ${itemErr.message}`);
              }
              // Brief pause before retry to let transient issues settle
              if (attempts < 3) {
                const delay = 500 * attempts;
                console.log(`[Migration]   Retrying in ${delay}ms...`);
                await new Promise((r) => setTimeout(r, delay));
              }
            }
          }

          completed++;
          // Report progress (include error only when a photo exhausts all retries)
          if (!itemSuccess) {
            await api.encryption.reportProgress({
              completed_count: completed,
              error: lastError,
            });
          } else {
            await api.encryption.reportProgress({ completed_count: completed });
          }
        }

        console.log(`[Migration] Complete: ${succeeded} succeeded, ${failedCount} failed out of ${total}`);

        // Mark migration complete — report failures if any occurred
        if (failedCount > 0) {
          await api.encryption.reportProgress({
            completed_count: total,
            done: true,
            error: `Migration finished with ${failedCount}/${total} failures. Last error: ${lastError}`,
          });
        } else {
          await api.encryption.reportProgress({ completed_count: total, done: true });
        }
        await loadEncryptionSettings();
      } catch (err: any) {
        console.error("[Migration] Top-level migration error:", err.message, err.stack);
        await api.encryption.reportProgress({
          completed_count: 0,
          error: `Migration failed: ${err.message}`,
        }).catch(() => {});
      } finally {
        migrationRunningRef.current = false;
      }
    })();
  }, [migrationStatus, loadEncryptionSettings]);
}
