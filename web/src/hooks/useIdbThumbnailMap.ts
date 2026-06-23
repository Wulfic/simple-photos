/**
 * useIdbThumbnailMap — load representative thumbnails for a list of
 * clusters/albums from the IndexedDB photo cache, returning a
 * `{ [key]: objectUrl }` map. Object URLs are revoked automatically when the
 * inputs change or the component unmounts.
 *
 * Replaces the byte-identical "load thumbUrls map" effect that the Trips,
 * Memories, Pets and People smart-album list views each hand-rolled (and which
 * the W5 SmartClusterList will reuse).
 */
import { useEffect, useRef, useState } from "react";
import { db } from "../db";

export interface IdbThumbnailItem<K extends string | number> {
  /** Map key (cluster/album id). */
  key: K;
  /** Server photo id of the representative photo (or null to skip). */
  photoId: string | null | undefined;
}

export interface IdbThumbnailOptions {
  /** Fallback URL when no cached thumbnail exists (e.g. server thumb endpoint). */
  fallbackUrl?: (photoId: string) => string;
}

export function useIdbThumbnailMap<K extends string | number>(
  items: IdbThumbnailItem<K>[],
  options?: IdbThumbnailOptions,
): Record<K, string> {
  const [urls, setUrls] = useState<Record<K, string>>({} as Record<K, string>);

  // Refs keep the effect dependency list to a single stable signature string,
  // so an inline-built `items` array doesn't retrigger the load every render.
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const fallbackRef = useRef(options?.fallbackUrl);
  fallbackRef.current = options?.fallbackUrl;

  const signature = items.map((i) => `${i.key}:${i.photoId ?? ""}`).join("|");

  useEffect(() => {
    let cancelled = false;
    const created: string[] = [];
    (async () => {
      const next = {} as Record<K, string>;
      for (const item of itemsRef.current) {
        if (!item.photoId) continue;
        const photo =
          (await db.photos.where("serverPhotoId").equals(item.photoId).first()) ??
          (await db.photos.get(item.photoId));
        if (cancelled) return;
        if (photo?.thumbnailData) {
          const mime = photo.thumbnailMimeType || "image/jpeg";
          const url = URL.createObjectURL(new Blob([photo.thumbnailData], { type: mime }));
          created.push(url);
          next[item.key] = url;
        } else if (fallbackRef.current) {
          next[item.key] = fallbackRef.current(item.photoId);
        }
      }
      if (!cancelled) setUrls(next);
    })();
    return () => {
      cancelled = true;
      created.forEach(URL.revokeObjectURL);
    };
  }, [signature]);

  return urls;
}
