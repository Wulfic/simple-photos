/**
 * Burst-stack selection expansion.
 *
 * Burst stacks are collapsed to a single representative tile in every grid
 * (gallery, smart albums, secure album picker), so a multi-select only ever
 * holds the representative frame's blobId. Adding that to an album — or moving
 * it into a secure album — would silently drop the rest of the burst. This
 * re-hydrates the full membership from the local photo cache for any selected
 * representative, leaving non-burst ids untouched.
 */
import { db } from "../db";

/**
 * Given a set of selected blob IDs (each typically a burst representative),
 * return a de-duplicated list that also includes every other frame belonging
 * to the same burst group(s). Non-burst ids pass through unchanged.
 */
export async function expandBurstSelection(blobIds: Iterable<string>): Promise<string[]> {
  const ids = [...new Set(blobIds)];
  if (ids.length === 0) return ids;

  // Which of the selected ids belong to a burst, and to which group(s)?
  const selectedRows = await db.photos.where("blobId").anyOf(ids).toArray();
  const burstIds = new Set<string>();
  for (const r of selectedRows) {
    if (r.burstId) burstIds.add(r.burstId);
  }
  if (burstIds.size === 0) return ids;

  // Pull every frame in those burst groups (burstId is indexed in db v9).
  const members = await db.photos.where("burstId").anyOf([...burstIds]).toArray();
  const out = new Set(ids);
  for (const m of members) out.add(m.blobId);
  return [...out];
}
