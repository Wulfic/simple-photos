package com.simplephotos.data.media

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Mirrors `web/src/gallery/renditionChoice.test.ts` so both clients agree on
 * what the picker offers and what it defaults to (#49). Where the two diverge
 * it is because Android can answer "is this link expensive?" definitively and
 * web cannot — see [isConstrained] versus web's three-signal guess.
 *
 * The live-library shapes below are the measured census from `todo.md`, not
 * invented examples: 71 of 742 videos are portrait, 14 of them exactly
 * `1080x1920`, and 4 are `2288x1088`. Both groups break a naive height-based
 * rung rule, which is why the ladder keys on the short edge.
 */
class RenditionChoiceTest {

    /** Landscape 16:9 unless a test says otherwise. */
    private fun rung(
        shortEdge: Int,
        isSource: Boolean = false,
        blobId: String? = "b-$shortEdge",
        width: Int = Math.round(shortEdge * 16f / 9f),
        height: Int = shortEdge,
    ) = Rendition(
        shortEdge = shortEdge,
        width = width,
        height = height,
        isSource = isSource,
        blobId = blobId,
        codec = "h264",
        sizeBytes = shortEdge * 1000L,
    )

    // ── offerableRenditions ─────────────────────────────────────────────────

    @Test
    fun offerable_isEmptyForTheNormalCaseOfOneQuality() {
        assertEquals(emptyList<Rendition>(), offerableRenditions(emptyList()))
        assertEquals(emptyList<Rendition>(), offerableRenditions(null))
    }

    @Test
    fun offerable_sortsHighestFirstRatherThanTrustingServerOrder() {
        val got = offerableRenditions(listOf(rung(1080), rung(2160, isSource = true)))
        assertEquals(listOf(2160, 1080), got.map { it.shortEdge })
    }

    @Test
    fun offerable_neverOffersTheSameQualityTwice() {
        val got = offerableRenditions(listOf(rung(1080), rung(1080), rung(2160)))
        assertEquals(listOf(2160, 1080), got.map { it.shortEdge })
    }

    @Test
    fun offerable_dropsRungsThisPlayerCannotStream() {
        // Null blobId is an unencrypted install. This player only speaks
        // spblob://, so such a rung would be a menu entry that does nothing.
        val got = offerableRenditions(listOf(rung(2160, isSource = true), rung(1080, blobId = null)))
        assertEquals(listOf(2160), got.map { it.shortEdge })
    }

    // ── shouldOfferPicker ───────────────────────────────────────────────────

    @Test
    fun picker_notDrawnWhenThereIsNothingToChooseBetween() {
        assertFalse(shouldOfferPicker(emptyList()))
        assertFalse(shouldOfferPicker(null))
    }

    @Test
    fun picker_notDrawnForALoneSourceRung() {
        // A one-entry menu implies a choice the user does not have.
        assertFalse(shouldOfferPicker(listOf(rung(2160, isSource = true))))
    }

    @Test
    fun picker_notDrawnWhenTheOnlyAlternativeIsUnstreamable() {
        // Two rungs on the wire, but one is plaintext-only — so after filtering
        // there is again nothing to choose between. Counting before filtering
        // would draw a gear icon opening a one-entry menu.
        assertFalse(
            shouldOfferPicker(listOf(rung(2160, isSource = true), rung(1080, blobId = null)))
        )
    }

    @Test
    fun picker_drawnOnceARealAlternativeExists() {
        assertTrue(shouldOfferPicker(listOf(rung(2160, isSource = true), rung(1080))))
    }

    // ── isConstrained ───────────────────────────────────────────────────────

    @Test
    fun constrained_meteredAloneNeverDowngrades() {
        // The issue is explicit: with the data saver OFF, always serve highest
        // regardless of network. A metered link must not downgrade a user who
        // never asked for it.
        assertFalse(isConstrained(dataSaverEnabled = false, metered = true))
    }

    @Test
    fun constrained_dataSaverAloneNeverDowngrades() {
        assertFalse(isConstrained(dataSaverEnabled = true, metered = false))
    }

    @Test
    fun constrained_requiresBoth() {
        assertTrue(isConstrained(dataSaverEnabled = true, metered = true))
        assertFalse(isConstrained(dataSaverEnabled = false, metered = false))
    }

    // ── chooseDefaultRendition ──────────────────────────────────────────────

    @Test
    fun default_isNullWhenThereIsNoLadder() {
        // Not an error — it means "play the photo's own blob as before #49".
        assertNull(chooseDefaultRendition(emptyList(), constrained = false))
        assertNull(chooseDefaultRendition(null, constrained = true))
    }

    @Test
    fun default_isHighestOnAnUnconstrainedLink() {
        val got = chooseDefaultRendition(listOf(rung(1080), rung(2160, isSource = true)), false)
        assertEquals(2160, got?.shortEdge)
    }

    @Test
    fun default_capsAtAbsolute1080NotOneRungDown() {
        // The 8K case from the live census. One rung down from 7680x4320 is 4K,
        // which is not what "lower on cellular" means to anybody — so the cap is
        // an absolute quality, not a relative step.
        val ladder = listOf(rung(4320, isSource = true), rung(2160), rung(1080))
        val got = chooseDefaultRendition(ladder, constrained = true)
        assertEquals(CONSTRAINED_MAX_SHORT_EDGE, got?.shortEdge)
    }

    @Test
    fun default_takesSmallestWhenEveryRungExceedsTheCap() {
        // A 4K source whose 1080 rung has not been generated yet. Refusing to
        // play would be worse: the alternative is a metered client fetching 4K.
        val ladder = listOf(rung(4320, isSource = true), rung(2160))
        val got = chooseDefaultRendition(ladder, constrained = true)
        assertEquals(2160, got?.shortEdge)
    }

    @Test
    fun default_neverPicksARungThePickerRefusesToShow() {
        // Defaulting to a filtered-out rung would leave the menu ticking nothing
        // and no way back to it.
        val ladder = listOf(rung(2160, isSource = true), rung(1080, blobId = null))
        val got = chooseDefaultRendition(ladder, constrained = true)
        assertEquals(2160, got?.shortEdge)
    }

    // ── the live library's awkward shapes ───────────────────────────────────

    @Test
    fun portrait1080x1920IsThe1080pTierNotA1920pOne() {
        // 14 live videos are exactly this. Keyed on height they would look like
        // a 1920p source needing a downscale; keyed on the short edge they are
        // already the 1080p tier and need no rung at all.
        val r = rung(1080, isSource = true, width = 1080, height = 1920)
        assertEquals("Original (1080p)", formatRenditionLabel(r))
        assertFalse(shouldOfferPicker(listOf(r)))
    }

    @Test
    fun portraitAndLandscapeOfTheSameTierSortTogether() {
        // A portrait 1080x1920 rung and a landscape 3840x2160 source: the
        // ordering must come from the short edge, not from either raw dimension
        // (1920 > 2160 is false, but 1920 > 1080 is true — pick the wrong field
        // and the portrait rung sorts above the 4K original).
        val ladder = listOf(
            rung(1080, width = 1080, height = 1920),
            rung(2160, isSource = true, width = 3840, height = 2160),
        )
        assertEquals(listOf(2160, 1080), offerableRenditions(ladder).map { it.shortEdge })
    }

    // ── labels ──────────────────────────────────────────────────────────────

    @Test
    fun label_showsTheResolutionEvenForTheOriginal() {
        // "Original" alone forces the user to guess whether it is bigger than
        // the 1080p entry below it.
        assertEquals("Original (2160p)", formatRenditionLabel(rung(2160, isSource = true)))
        assertEquals("1080p", formatRenditionLabel(rung(1080)))
    }

    // ── renditionsEqual ─────────────────────────────────────────────────────

    @Test
    fun equal_treatsNullAndEmptyAsTheSameState() {
        // Both mean "one quality" and they arrive from different places: a
        // pre-#49 server yields null, a #49 server sends an empty list for the
        // ~600 videos needing no rung. Treating them as different makes the
        // first pass after a server upgrade rewrite the whole library.
        assertTrue(renditionsEqual(null, emptyList()))
        assertTrue(renditionsEqual(emptyList(), null))
        assertTrue(renditionsEqual(null, null))
    }

    @Test
    fun equal_detectsARealLadderChange() {
        val before = listOf(rung(2160, isSource = true))
        val after = listOf(rung(2160, isSource = true), rung(1080))
        assertFalse(renditionsEqual(before, after))
    }

    @Test
    fun equal_detectsARungGainingItsBlob() {
        // The moment a rung becomes playable is an UPDATE, not an INSERT — the
        // trigger bug `036` had to fix. The client must notice it too.
        assertFalse(
            renditionsEqual(listOf(rung(1080, blobId = null)), listOf(rung(1080)))
        )
    }
}
