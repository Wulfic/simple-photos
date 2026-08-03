package com.simplephotos.ui.screens.library

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Mirrors `web/src/utils/thumbnailCss.faceCrop.test.ts` so both clients frame a
 * face identically (#48).
 *
 * These assert the *property* — the face centre ends up in the middle of the
 * tile — rather than the formula. The web suite this replaces asserted the
 * formula's own output under a test named "centres the crop on the face
 * centre", so the assertion and the bug agreed with each other and the defect
 * shipped with a green suite.
 */
class FaceCropTest {

    private val eps = 1e-4f

    /**
     * Invert the produced rect: where in the tile (0–1) does image-point [c]
     * land? Tile is 1 unit wide; the image is 1/z wide and offset by
     * p·(1 − 1/z), which is exactly what the composable applies as a scale plus
     * translation.
     */
    private fun landsAt(z: Float, p: Float, c: Float): Float {
        val size = 1f / z
        val offset = p * (1f - size)
        return offset + c * size
    }

    private fun centreX(b: TileFaceBox) = b.x + b.w / 2f
    private fun centreY(b: TileFaceBox) = b.y + b.h / 2f

    @Test
    fun `null and degenerate boxes produce no rect`() {
        assertNull(faceCropRect(null))
        assertNull(faceCropRect(TileFaceBox(0f, 0f, 0f, 0.2f)))
        assertNull(faceCropRect(TileFaceBox(0f, 0f, 0.2f, -1f)))
        assertNull(faceCropRect(TileFaceBox(Float.NaN, 0f, 0.2f, 0.2f)))
    }

    @Test
    fun `face centre lands in the middle of the tile`() {
        val boxes = listOf(
            TileFaceBox(0.4f, 0.4f, 0.2f, 0.2f),   // dead centre
            TileFaceBox(0.25f, 0.2f, 0.1f, 0.1f),  // #48's "up and to the left"
            TileFaceBox(0.6f, 0.65f, 0.15f, 0.15f),
            TileFaceBox(0.3f, 0.3f, 0.12f, 0.18f), // non-square box
        )
        for (b in boxes) {
            val r = faceCropRect(b)!!
            assertEquals("x for $b", 0.5f, landsAt(r.zx, r.px, centreX(b)), 1e-3f)
            assertEquals("y for $b", 0.5f, landsAt(r.zy, r.py, centreY(b)), 1e-3f)
        }
    }

    @Test
    fun `box is expanded uniformly so a pixel-square face stays square`() {
        // w and h differ only because the photo is 4:3. The window must keep
        // that ratio or the face renders stretched in a square tile.
        val b = TileFaceBox(0.3f, 0.3f, 0.15f, 0.2f)
        val r = faceCropRect(b)!!
        assertEquals(b.w / b.h, r.zx / r.zy, eps)
    }

    @Test
    fun `face occupies the target fraction of the tile`() {
        val b = TileFaceBox(0.4f, 0.4f, 0.2f, 0.2f)
        val r = faceCropRect(b)!!
        assertEquals(FACE_TARGET_FRACTION, b.w / r.zx, eps)
        assertEquals(FACE_TARGET_FRACTION, b.h / r.zy, eps)
    }

    @Test
    fun `a tiny face is not magnified past the zoom cap`() {
        // Reaching the target would need 30x; the floor caps it instead.
        val r = faceCropRect(TileFaceBox(0.45f, 0.45f, 0.02f, 0.02f))!!
        assertEquals(1f / FACE_MAX_ZOOM, maxOf(r.zx, r.zy), eps)
    }

    @Test
    fun `the window never runs off the photo`() {
        val r = faceCropRect(TileFaceBox(0.05f, 0.05f, 0.9f, 0.6f))!!
        assertTrue("zx=${r.zx}", r.zx <= 1f + eps)
        assertTrue("zy=${r.zy}", r.zy <= 1f + eps)
        // The long axis is the binding constraint and should be fully used.
        assertEquals(1f, r.zx, eps)
    }

    @Test
    fun `a corner face still covers the tile completely`() {
        // It cannot be centred without panning off the photo, so the position
        // clamps — but no tile may end up showing background.
        val boxes = listOf(
            TileFaceBox(0f, 0f, 0.1f, 0.1f),
            TileFaceBox(0.9f, 0.9f, 0.1f, 0.1f),
            TileFaceBox(0.85f, 0.02f, 0.13f, 0.13f),
        )
        for (b in boxes) {
            val r = faceCropRect(b)!!
            assertTrue("left edge for $b", landsAt(r.zx, r.px, 0f) <= eps)
            assertTrue("right edge for $b", landsAt(r.zx, r.px, 1f) >= 1f - eps)
            assertTrue("top edge for $b", landsAt(r.zy, r.py, 0f) <= eps)
            assertTrue("bottom edge for $b", landsAt(r.zy, r.py, 1f) >= 1f - eps)
        }
    }

    @Test
    fun `at target fraction 1 the window is the box itself`() {
        // The parameters web's face chip uses: fill the container with the face.
        val b = TileFaceBox(0.2f, 0.1f, 0.3f, 0.4f)
        val r = faceCropRect(b, targetFraction = 1f, minVisibleFraction = 0f)!!
        assertNotNull(r)
        assertEquals(b.w, r.zx, eps)
        assertEquals(b.h, r.zy, eps)
        assertEquals(b.x / (1f - b.w), r.px, eps)
        assertEquals(b.y / (1f - b.h), r.py, eps)
    }

    @Test
    fun `a box larger than the photo is clamped rather than inverting the window`() {
        val r = faceCropRect(TileFaceBox(0f, 0f, 1.5f, 1.5f))!!
        assertEquals(1f, r.zx, eps)
        assertEquals(1f, r.zy, eps)
    }

    // ── tileFaceBoxOf — the DTO guard shared by the People and Pets tiles ──

    @Test
    fun `a cluster with no extent yields no box`() {
        // Both People and Pets legitimately receive all-null rep_bbox_*: a
        // secured-out cluster, or a pet processed before migration 039. The
        // caller must get null and draw a plain crop, not a degenerate window.
        assertNull(tileFaceBoxOf(null, null, null, null))
        assertNull(tileFaceBoxOf(0.1, 0.2, null, 0.4))
        assertNull(tileFaceBoxOf(0.1, 0.2, 0.3, null))
    }

    @Test
    fun `a missing origin is a real zero, not a missing box`() {
        // An animal flush against the top-left corner has x = y = 0, which JSON
        // may or may not carry. Treating that as "no box" would drop framing on
        // exactly the photos where it matters most.
        val b = tileFaceBoxOf(null, null, 0.3, 0.4)!!
        assertEquals(0f, b.x, eps)
        assertEquals(0f, b.y, eps)
        assertEquals(0.3f, b.w, eps)
        assertEquals(0.4f, b.h, eps)
    }

    @Test
    fun `a complete box passes through unchanged`() {
        val b = tileFaceBoxOf(0.1, 0.2, 0.3, 0.4)!!
        assertEquals(TileFaceBox(0.1f, 0.2f, 0.3f, 0.4f), b)
    }
}
