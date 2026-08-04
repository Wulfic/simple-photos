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
 * `otherSecureAlbums` is how many OTHER secure albums still hold this photo
 * (0 for the common single-album case). It decides which of two genuinely
 * different outcomes the user is agreeing to:
 *
 *  - 0  → the photo leaves the secure domain and becomes visible in the regular
 *         gallery again. This is the pre-Z1 behaviour and still the usual one.
 *  - >0 → the photo stays hidden and stays secured, because another secure album
 *         still contains it. Telling the user it "returns to your gallery" here
 *         would be a privacy-shaped lie: they would believe they had un-secured
 *         something they had not, or believe they had exposed something they
 *         had not.
 */
export function secureRemovalPrompt(
  count: number,
  albumName?: string,
  otherSecureAlbums = 0,
): RemovalPrompt {
  const where = albumName ? `“${albumName}”` : "this secure album";
  const subject = count === 1 ? "It" : "They";

  if (otherSecureAlbums > 0) {
    const others = `${otherSecureAlbums} other secure album${otherSecureAlbums === 1 ? "" : "s"}`;
    return {
      title: `Remove ${photoCount(count)} from ${where}?`,
      body:
        `${subject} will stay secured — ${count === 1 ? "it is" : "they are"} also in ` +
        `${others}, so ${count === 1 ? "it" : "they"} will NOT return to your regular gallery.`,
    };
  }

  return {
    title: `Remove ${photoCount(count)} from ${where}?`,
    body:
      `${subject} will be unsecured and become visible in your regular gallery again. ` +
      `Nothing is deleted.`,
  };
}
