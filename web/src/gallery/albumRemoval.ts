/**
 * What a "remove from album" confirmation should actually say.
 *
 * Kept pure and separate from the components for two reasons. The repo has no
 * jsdom, so a rendered dialog cannot be asserted in vitest at all — the only
 * testable part of this feature is the wording, and the wording is the part
 * that can be *wrong*. And the wording is genuinely conditional now: since Z1 a
 * photo may live in several secure albums, so "it will return to your regular
 * gallery" — which the secure gallery said unconditionally — is false whenever
 * another secure album still holds it.
 *
 * A confirmation that misdescribes its own effect is worse than no confirmation:
 * it spends the user's attention and then misinforms them.
 */

/** Title + body for a confirmation prompt. */
export interface RemovalPrompt {
  title: string;
  body: string;
}

/**
 * What the UI should do about a secure removal.
 *
 *  - `confirm` — ask the question. The body describes the real outcome.
 *  - `blocked` — we do not *know* what removal would do, so we must not ask.
 *
 * The blocked arm exists because the honest alternatives are both bad. A prompt
 * that hedges ("if it is in no other secure album, it returns…") makes the user
 * adjudicate a fact only the server holds, and a prompt that guesses is the Z1
 * bug restated. Refusing is recoverable — the caller offers a refresh — and it
 * is the only arm that cannot mislead.
 */
export type SecureRemovalVerdict = RemovalPrompt & { kind: "confirm" | "blocked" };

/**
 * How many OTHER secure albums hold this photo, from the `galleries` array the
 * secure feeds publish — or `undefined` when that cannot be determined.
 *
 * **Empty means UNKNOWN, not zero**, and the distinction is the entire point.
 * The server documents the same contract on its side: a miss is unreachable by
 * construction, so an empty array can only mean the feed did not publish
 * memberships at all (an older server), never "this photo is in one album".
 * Reading 0 as "no other album" is exactly how the UI came to promise a photo
 * would return to the regular gallery when it would stay secured.
 *
 * A list that does not contain the owning album is also unknown rather than
 * off-by-one: the owner is the one membership that must be there, so its absence
 * means the array is not what we think it is. Counting it as an "other" would
 * over-report by one and flip a single-album removal into the wrong branch.
 */
export function otherSecureAlbumCount(
  memberships: ReadonlyArray<{ id: string }> | null | undefined,
  owningGalleryId: string,
): number | undefined {
  if (!memberships || memberships.length === 0) return undefined;
  if (!memberships.some((g) => g.id === owningGalleryId)) return undefined;
  return memberships.length - 1;
}

function photoCount(n: number): string {
  return `${n} photo${n === 1 ? "" : "s"}`;
}

/**
 * Removing photos from an ordinary (non-secure) album.
 *
 * The load-bearing sentence is the second one. "Remove" next to a trash icon
 * reads as *delete*, and this action does not delete anything — it un-files the
 * photo. Saying so is the entire reason the prompt exists; without it the icon
 * change alone would make the action look more destructive than it is.
 */
export function albumRemovalPrompt(count: number, albumName?: string): RemovalPrompt {
  const where = albumName ? `“${albumName}”` : "this album";
  return {
    title: `Remove ${photoCount(count)} from ${where}?`,
    body:
      `${count === 1 ? "It stays" : "They stay"} in your gallery and in any other ` +
      `albums — only the link to ${where} is removed. Nothing is deleted.`,
  };
}

/**
 * Removing items from a SECURE album — one item or a whole selection.
 *
 * `otherSecureAlbums` holds, **per item actually being removed**, how many OTHER
 * secure albums still hold it, as resolved by {@link otherSecureAlbumCount}. Each
 * entry decides one of three genuinely different outcomes:
 *
 *  - 0         → the photo leaves the secure domain and becomes visible in the
 *                regular gallery again. Pre-Z1 behaviour, still the usual one.
 *  - >0        → the photo stays hidden and stays secured, because another secure
 *                album still contains it. Telling the user it "returns to your
 *                gallery" here would be a privacy-shaped lie.
 *  - undefined → we cannot tell the two apart.
 *
 * **One unknown blocks the whole batch, and an empty list is unknown too**: a
 * caller that resolved nothing has not answered the question. A batch can mix
 * the first two outcomes, so the prompt says how many go each way.
 *
 * The list form (rather than one scalar for every item) is Android's
 * `AlbumRemoval.secureRemovalPrompt`, ported so both clients ask the same
 * question the same way. The scalar form could not describe a multi-select at
 * all — which is why web had no multi-select removal (TODO.md S3).
 *
 * The argument is required and has no default, deliberately: a default of "0
 * other albums" hands the most dangerous answer to every call site by omission.
 */
export function secureRemovalPrompt(
  otherSecureAlbums: ReadonlyArray<number | undefined>,
  albumName: string | undefined,
): SecureRemovalVerdict {
  const count = otherSecureAlbums.length;
  const where = albumName ? `“${albumName}”` : "this secure album";

  if (count === 0 || otherSecureAlbums.some((n) => n === undefined)) {
    const these = count === 1 ? "this photo" : "these photos";
    const them = count === 1 ? "it" : "them";
    return {
      kind: "blocked",
      title: `Can’t remove from ${where} yet`,
      body:
        `This server did not report which secure albums hold ${these}, so we can’t ` +
        `tell you whether removing ${them} here would make ${them} visible in your ` +
        `regular gallery again. Refresh and try again.`,
    };
  }

  // Every entry is known past the guard above; the type guard just says so.
  const counts = otherSecureAlbums.filter((n): n is number => n !== undefined);
  const staying = counts.filter((n) => n > 0).length;
  const returning = count - staying;
  const subject = count === 1 ? "It" : "They";
  const subjectLower = count === 1 ? "it" : "they";
  const title = `Remove ${photoCount(count)} from ${where}?`;

  if (staying === 0) {
    return {
      kind: "confirm",
      title,
      body:
        `${subject} will be unsecured and become visible in your regular gallery again. ` +
        `Nothing is deleted.`,
    };
  }

  if (returning === 0) {
    const because =
      count === 1
        ? `it is also in ${counts[0]} other secure album${counts[0] === 1 ? "" : "s"}`
        : "each is also in another secure album";
    return {
      kind: "confirm",
      title,
      body:
        `${subject} will stay secured — ${because}, so ${subjectLower} will NOT ` +
        `return to your regular gallery.`,
    };
  }

  return {
    kind: "confirm",
    title,
    body:
      `${returning} will return to your regular gallery. The other ${staying} will ` +
      `stay secured — another secure album still holds ${staying === 1 ? "it" : "them"}. ` +
      `Nothing is deleted.`,
  };
}

/**
 * The success message after a secure removal, from the SAME per-item counts the
 * prompt was built from — a toast derived separately is how "Photo returned to
 * your gallery" came to be shown for photos that stayed secured (Z1d). Only call
 * this for a batch the prompt confirmed, i.e. no `undefined` entries.
 */
export function secureRemovalToast(otherSecureAlbums: ReadonlyArray<number>): string {
  const count = otherSecureAlbums.length;
  const staying = otherSecureAlbums.filter((n) => n > 0).length;
  const returning = count - staying;
  if (staying === 0) {
    return count === 1 ? "Photo returned to your gallery." : `${count} photos returned to your gallery.`;
  }
  if (returning === 0) {
    return count === 1
      ? "Removed. The photo stays secured in your other secure albums."
      : `Removed ${count}. They stay secured in your other secure albums.`;
  }
  return (
    `Removed ${count}. ${returning} returned to your gallery; ` +
    `${staying} stay secured in other secure albums.`
  );
}
