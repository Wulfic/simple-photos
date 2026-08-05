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
 * Removing an item from a SECURE album.
 *
 * `otherSecureAlbums` is how many OTHER secure albums still hold this photo, as
 * resolved by {@link otherSecureAlbumCount}. It decides which of three genuinely
 * different outcomes the user is agreeing to:
 *
 *  - 0         → the photo leaves the secure domain and becomes visible in the
 *                regular gallery again. Pre-Z1 behaviour, still the usual one.
 *  - >0        → the photo stays hidden and stays secured, because another secure
 *                album still contains it. Telling the user it "returns to your
 *                gallery" here would be a privacy-shaped lie: they would believe
 *                they had un-secured something they had not, or believe they had
 *                exposed something they had not.
 *  - undefined → we cannot tell the two apart, so we refuse rather than guess.
 *
 * **The parameter is required and has no default**, deliberately. It used to
 * default to 0, which meant every call site that had not thought about
 * membership silently got the "returns to your gallery" promise — the most
 * dangerous of the three answers, handed out by omission. An argument you must
 * pass is the only version of this function that cannot be misused by accident.
 */
export function secureRemovalPrompt(
  count: number,
  albumName: string | undefined,
  otherSecureAlbums: number | undefined,
): SecureRemovalVerdict {
  const where = albumName ? `“${albumName}”` : "this secure album";
  const subject = count === 1 ? "It" : "They";

  if (otherSecureAlbums === undefined) {
    return {
      kind: "blocked",
      title: `Can’t remove from ${where} yet`,
      body:
        `This server did not report which secure albums hold ` +
        `${count === 1 ? "this photo" : "these photos"}, so we can’t tell you whether ` +
        `removing ${count === 1 ? "it" : "them"} here would make ` +
        `${count === 1 ? "it" : "them"} visible in your regular gallery again. ` +
        `Refresh and try again.`,
    };
  }

  if (otherSecureAlbums > 0) {
    const others = `${otherSecureAlbums} other secure album${otherSecureAlbums === 1 ? "" : "s"}`;
    return {
      kind: "confirm",
      title: `Remove ${photoCount(count)} from ${where}?`,
      body:
        `${subject} will stay secured — ${count === 1 ? "it is" : "they are"} also in ` +
        `${others}, so ${count === 1 ? "it" : "they"} will NOT return to your regular gallery.`,
    };
  }

  return {
    kind: "confirm",
    title: `Remove ${photoCount(count)} from ${where}?`,
    body:
      `${subject} will be unsecured and become visible in your regular gallery again. ` +
      `Nothing is deleted.`,
  };
}
