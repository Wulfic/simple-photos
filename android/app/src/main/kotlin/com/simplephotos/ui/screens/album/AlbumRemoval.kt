/**
 * What a "remove from album" confirmation should actually say.
 *
 * Kept pure and free of Compose so the part that can be *wrong* — the wording —
 * is JVM-unit-testable. A rendered Compose dialog needs a device
 * (`androidTest`), which is exactly why the false sentence this file exists to
 * kill survived unnoticed on the phone long after web had fixed it.
 *
 * Mirrors `web/src/gallery/albumRemoval.ts` so both clients describe the same
 * operation the same way. The wording is genuinely conditional now: since Z1 a
 * photo may live in several secure albums, so "it will return to your regular
 * gallery" — which [GalleryDetailView] said unconditionally — is false whenever
 * another secure album still holds it.
 *
 * A confirmation that misdescribes its own effect is worse than no confirmation:
 * it spends the user's attention and then misinforms them.
 */
package com.simplephotos.ui.screens.album

import com.simplephotos.data.remote.dto.SecureGalleryRef

/** Title + body for a confirmation prompt. */
data class RemovalPrompt(val title: String, val body: String)

/**
 * What the UI should do about a secure removal.
 *
 *  - [Confirm] — ask the question. The body describes the real outcome.
 *  - [Blocked] — we do not *know* what removal would do, so we must not ask.
 *
 * The blocked arm exists because the honest alternatives are both bad. A prompt
 * that hedges ("if it is in no other secure album, it returns…") makes the user
 * adjudicate a fact only the server holds, and a prompt that guesses is the Z1
 * bug restated. Refusing is recoverable — the caller offers a refresh — and it
 * is the only arm that cannot mislead.
 *
 * A sealed hierarchy rather than a `kind` string (web's shape, forced by
 * TypeScript): an exhaustive `when` makes a call site that renders only the
 * confirm arm fail to compile instead of silently dropping the refusal.
 */
sealed interface SecureRemovalVerdict {
    val prompt: RemovalPrompt

    data class Confirm(override val prompt: RemovalPrompt) : SecureRemovalVerdict

    data class Blocked(override val prompt: RemovalPrompt) : SecureRemovalVerdict
}

object AlbumRemoval {

    /**
     * How many OTHER secure albums hold this photo, from the `galleries` array
     * the secure feeds publish — or `null` when that cannot be determined.
     *
     * **Empty means UNKNOWN, not zero**, and the distinction is the entire
     * point. The server documents the same contract on its side: a miss is
     * unreachable by construction, so an empty array can only mean the feed did
     * not publish memberships at all (an older server, or — the realistic
     * Android failure — a renamed wire key that Gson leaves at its default).
     * Reading 0 as "no other album" is exactly how the UI came to promise a
     * photo would return to the regular gallery when it would stay secured.
     *
     * A list that does not contain the owning album is also unknown rather than
     * off-by-one: the owner is the one membership that must be there, so its
     * absence means the array is not what we think it is. Counting it as an
     * "other" would over-report by one and flip a last-membership removal into
     * the "stays secured" branch — wrong in the direction that surprises the
     * user.
     */
    fun otherSecureAlbumCount(
        memberships: List<SecureGalleryRef>?,
        owningGalleryId: String,
    ): Int? {
        if (memberships.isNullOrEmpty()) return null
        if (memberships.none { it.id == owningGalleryId }) return null
        return memberships.size - 1
    }

    /**
     * Removing photos from an ordinary (non-secure) album.
     *
     * The load-bearing sentence is the second one. "Remove" next to a trash icon
     * reads as *delete*, and this action does not delete anything — it un-files
     * the photo. Saying so is the entire reason the prompt exists; without it
     * the icon change alone would make the action look more destructive than it
     * is.
     */
    fun albumRemovalPrompt(count: Int, albumName: String?): RemovalPrompt {
        val where = where(albumName, "this album")
        return RemovalPrompt(
            title = "Remove ${photoCount(count)} from $where?",
            body = "${if (count == 1) "It stays" else "They stay"} in your gallery and in " +
                "any other albums — only the link to $where is removed. Nothing is deleted.",
        )
    }

    /**
     * Removing items from a SECURE album.
     *
     * [otherSecureAlbums] carries **one entry per item being removed**, each as
     * resolved by [otherSecureAlbumCount]: how many OTHER secure albums still
     * hold that item, or `null` for "cannot tell". A list rather than web's
     * scalar because this screen removes a whole multi-select at once, and the
     * answer is genuinely per item — one derivation covering both the single
     * tile and the batch, instead of a scalar rule plus a batch rule that would
     * drift the first time either was edited.
     *
     * The three outcomes it distinguishes:
     *
     *  - every item at 0 → the photos leave the secure domain and become visible
     *    in the regular gallery again. Pre-Z1 behaviour, still the usual one.
     *  - any item above 0 → those stay hidden and stay secured, because another
     *    secure album still contains them. Telling the user they "return to your
     *    gallery" here would be a privacy-shaped lie: they would believe they
     *    had un-secured something they had not.
     *  - any item unknown → we cannot tell the two apart, so we refuse rather
     *    than guess. An empty list is unknown too: a caller that resolved
     *    nothing has not answered the question, and the safe reading of "no
     *    information" is never "no other album".
     *
     * **The parameter is required and has no default**, deliberately — the same
     * decision web recorded. A default would hand the most dangerous of the
     * three answers to every call site that had not thought about membership,
     * by omission. An argument you must pass is the only version of this
     * function that cannot be misused by accident.
     */
    fun secureRemovalPrompt(
        otherSecureAlbums: List<Int?>,
        albumName: String?,
    ): SecureRemovalVerdict {
        val count = otherSecureAlbums.size
        val where = where(albumName, "this secure album")

        if (otherSecureAlbums.isEmpty() || otherSecureAlbums.any { it == null }) {
            val these = if (count == 1) "this photo" else "these photos"
            val them = if (count == 1) "it" else "them"
            return SecureRemovalVerdict.Blocked(
                RemovalPrompt(
                    title = "Can't remove from $where yet",
                    body = "This server did not report which secure albums hold $these, " +
                        "so we can't tell you whether removing $them here would make " +
                        "$them visible in your regular gallery again. Refresh and try again.",
                )
            )
        }

        val counts = otherSecureAlbums.filterNotNull()
        val staying = counts.count { it > 0 }
        val returning = counts.size - staying
        val subject = if (count == 1) "It" else "They"
        val subjectLower = if (count == 1) "it" else "they"
        val title = "Remove ${photoCount(count)} from $where?"

        // Nothing stays secured — the pre-Z1 outcome, and still the usual one.
        if (staying == 0) {
            return SecureRemovalVerdict.Confirm(
                RemovalPrompt(
                    title = title,
                    body = "$subject will be unsecured and become visible in your regular " +
                        "gallery again. Nothing is deleted.",
                )
            )
        }

        // Everything stays secured. For a single item we know exactly how many
        // other albums hold it; for a batch the counts differ per item, so the
        // claim is deliberately weaker rather than an average nobody asked for.
        if (returning == 0) {
            val because = if (count == 1) {
                val others = counts.first()
                "$subjectLower is also in $others other secure album${if (others == 1) "" else "s"}"
            } else {
                "each is also in another secure album"
            }
            return SecureRemovalVerdict.Confirm(
                RemovalPrompt(
                    title = title,
                    body = "$subject will stay secured — $because, so $subjectLower will NOT " +
                        "return to your regular gallery.",
                )
            )
        }

        // Mixed: the one case a single per-batch sentence cannot state honestly,
        // so it states both halves with their counts.
        return SecureRemovalVerdict.Confirm(
            RemovalPrompt(
                title = title,
                body = "$returning will return to your regular gallery. The other $staying " +
                    "will stay secured — another secure album still holds " +
                    "${if (staying == 1) "it" else "them"}. Nothing is deleted.",
            )
        )
    }

    private fun photoCount(n: Int): String = "$n photo${if (n == 1) "" else "s"}"

    private fun where(albumName: String?, fallback: String): String =
        if (albumName.isNullOrBlank()) fallback else "“$albumName”"
}
