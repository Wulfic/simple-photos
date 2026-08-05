/**
 * Secure smart albums — the built-in synthetic albums (Secure Gallery / Photos /
 * GIFs / Videos / Audio) derived from the aggregate secure-items feed. Membership
 * is a pure function of each item's `media_type`; nothing is stored.
 *
 * Mirrors the web `secureSmartAlbums.ts`. The `secure-smart-*` id namespace
 * deliberately never collides with the main gallery's `smart-*` ids.
 */
package com.simplephotos.data

import com.simplephotos.data.remote.dto.SecureGalleryItem

/** A computed, visible secure smart album (only produced when count > 0). */
data class SecureSmartAlbum(
    val id: String,
    val label: String,
    val count: Int,
    /** Newest matching item — its thumbnail becomes the album cover. */
    val coverItem: SecureGalleryItem,
)

object SecureSmartAlbums {
    const val ALL = "secure-smart-all"
    const val PHOTOS = "secure-smart-photos"
    const val GIFS = "secure-smart-gifs"
    const val VIDEOS = "secure-smart-videos"
    const val AUDIO = "secure-smart-audio"

    private data class Def(
        val id: String,
        val label: String,
        val matches: (SecureGalleryItem) -> Boolean,
    )

    // Ordered: Secure Gallery → Photos → GIFs → Videos → Audio. `PHOTOS` mirrors
    // the main gallery's `smart-photos` (includes GIFs) and adopts NULL
    // media_type (backup servers with no clone photos row) so nothing vanishes.
    private val DEFS = listOf(
        Def(ALL, "Secure Gallery") { true },
        Def(PHOTOS, "Photos") {
            it.mediaType == "photo" || it.mediaType == "gif" || it.mediaType == null
        },
        Def(GIFS, "GIFs") { it.mediaType == "gif" },
        Def(VIDEOS, "Videos") { it.mediaType == "video" },
        Def(AUDIO, "Audio") { it.mediaType == "audio" },
    )

    /** True when [id] names one of the built-in secure smart albums. */
    fun isSmart(id: String?): Boolean = id != null && DEFS.any { it.id == id }

    /** Display label for a smart album id, or null if not a smart id. */
    fun labelOf(id: String): String? = DEFS.firstOrNull { it.id == id }?.label

    /** Filter the aggregate feed down to one smart album's members. */
    fun filter(items: List<SecureGalleryItem>, id: String): List<SecureGalleryItem> {
        val def = DEFS.firstOrNull { it.id == id } ?: return emptyList()
        return items.filter(def.matches)
    }

    /**
     * Compute the visible smart albums from the aggregate feed. Only non-empty
     * types are returned. [items] is assumed to be in `added_at DESC` order (the
     * server contract), so the first match per def is the newest item = cover.
     * `count` is the raw membership count (bursts NOT collapsed).
     */
    fun compute(items: List<SecureGalleryItem>): List<SecureSmartAlbum> {
        val out = mutableListOf<SecureSmartAlbum>()
        for (def in DEFS) {
            var count = 0
            var cover: SecureGalleryItem? = null
            for (item in items) {
                if (!def.matches(item)) continue
                count++
                if (cover == null) cover = item // first match = newest
            }
            val c = cover
            if (count > 0 && c != null) {
                out.add(SecureSmartAlbum(def.id, def.label, count, c))
            }
        }
        return out
    }
}
