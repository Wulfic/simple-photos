package com.simplephotos.data.remote

import com.google.gson.Gson
import com.simplephotos.data.media.offerableRenditions
import com.simplephotos.data.media.shouldOfferPicker
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.data.remote.dto.SecureGalleryItemsResponse
import com.simplephotos.data.remote.dto.toDomain
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The secure listing's video ladder (#49 remainder), pinned at the wire.
 *
 * This is a **parsing** test on purpose. The logic downstream of it —
 * `offerableRenditions` / `shouldOfferPicker` — is already covered by
 * `RenditionChoiceTest`, and the picker UI itself needs a device. What has no
 * other guard is the `@SerializedName` mapping, and its failure mode is the
 * quietest one in this codebase: Gson leaves an unrecognised field at its
 * default, so a renamed key turns the ladder into `null`, which collapses to
 * "no picker" — indistinguishable from a video that legitimately has no rungs.
 * `PhotoDto.renditions` records that this exact thing already happened once:
 * the field was silently ignored for the whole life of `8564636`.
 */
class SecureItemLadderTest {

    private val gson = Gson()

    /** Exactly what `list_gallery_items` emits for a secured 4K video. */
    private val securedVideoJson = """
        {"items":[{
          "id":"item-1",
          "blob_id":"clone-blob",
          "added_at":"2026-08-04T00:00:00Z",
          "gallery_id":"g1",
          "media_type":"video",
          "renditions":[
            {"short_edge":2160,"width":3840,"height":2160,"is_source":true,
             "blob_id":"orig-blob","codec":"hevc","size_bytes":900000000},
            {"short_edge":1080,"width":1920,"height":1080,"is_source":false,
             "blob_id":"rung-1080","codec":"h264","size_bytes":120000000}
          ]
        }]}
    """.trimIndent()

    @Test
    fun `a secured video's ladder survives the wire and reaches the picker`() {
        val items = gson.fromJson(securedVideoJson, SecureGalleryItemsResponse::class.java).items
        val ladder = items.single().renditions.toDomain()

        assertEquals(listOf(2160, 1080), ladder.map { it.shortEdge })
        assertTrue("the 2160 rung is the untouched original", ladder[0].isSource)
        assertEquals("rung-1080", ladder[1].blobId)
        assertEquals(120_000_000L, ladder[1].sizeBytes)

        // The point of carrying it at all: the picker must actually appear.
        assertTrue(
            "a secured 4K video with a 1080p rung must offer a choice",
            shouldOfferPicker(ladder)
        )
        assertEquals(2, offerableRenditions(ladder).size)
    }

    /**
     * The source rung's `blob_id` names the **hidden original's** blob, not this
     * item's clone. The viewer must therefore treat "Original" as "this item's
     * own payload" rather than fetching that id — pinned here because the two
     * ids being different is the whole reason that branch exists, and a fixture
     * where they happened to match would make the bug invisible.
     */
    @Test
    fun `the source rung does not name the secure item's own blob`() {
        val item = gson.fromJson(securedVideoJson, SecureGalleryItemsResponse::class.java)
            .items.single()
        val source = item.renditions.toDomain().single { it.isSource }

        assertEquals("clone-blob", item.blobId)
        assertFalse(
            "playing the source rung's blob would serve the hidden original, " +
                "not the secure clone",
            source.blobId == item.blobId
        )
    }

    /**
     * Both "this server predates the field" and "this video has no rungs" must
     * land on the same answer — no picker — because nothing downstream can act
     * on the difference. Same contract as `PhotoDto`.
     */
    @Test
    fun `an absent or empty ladder both draw no picker`() {
        val absent = gson.fromJson(
            """{"items":[{"id":"i","blob_id":"b","added_at":"t","media_type":"video"}]}""",
            SecureGalleryItemsResponse::class.java
        ).items.single()
        val empty = gson.fromJson(
            """{"items":[{"id":"i","blob_id":"b","added_at":"t","media_type":"video",
                "renditions":[]}]}""",
            SecureGalleryItemsResponse::class.java
        ).items.single()

        assertTrue(absent.renditions.toDomain().isEmpty())
        assertTrue(empty.renditions.toDomain().isEmpty())
        assertFalse(shouldOfferPicker(absent.renditions.toDomain()))
        assertFalse(shouldOfferPicker(empty.renditions.toDomain()))
    }

    /**
     * A secured **still** is the overwhelming majority of secure items and must
     * carry nothing — the server sends no rungs for it, and the client must not
     * invent an empty picker on a photo.
     */
    @Test
    fun `a secured photo carries no ladder`() {
        val item = SecureGalleryItem(
            id = "i", blobId = "b", addedAt = "t", mediaType = "photo"
        )
        assertTrue(item.renditions.toDomain().isEmpty())
        assertFalse(shouldOfferPicker(item.renditions.toDomain()))
    }
}
