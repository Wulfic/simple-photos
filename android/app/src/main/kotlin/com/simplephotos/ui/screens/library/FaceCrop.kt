package com.simplephotos.ui.screens.library

/**
 * Face-tile framing maths — the Kotlin twin of `web/src/utils/thumbnailCss.ts`
 * (`faceCropRect`). Kept Android-free so it is JVM-unit-testable: this is pure
 * arithmetic and must not need a device to verify, which is precisely what
 * #48 got wrong on both platforms at once.
 *
 * Both platforms previously scaled the thumbnail about a transform-origin set
 * to the face centre. Scaling about a point holds that point STILL — it does
 * not move it to the middle — so every face rendered wherever it already was,
 * up and to the left for the common case. Android was worse than web because
 * `ContentScale.Crop` centre-cropped the aspect-preserving thumbnail to a
 * square first, so the bbox (normalised against the whole photo) was being
 * applied in a coordinate space it did not belong to.
 *
 * The fix is to stop asking "where does the face land after a crop?" and
 * instead place the window ourselves.
 */

/** Face bounding box, normalised 0–1 against the whole photo. */
data class TileFaceBox(val x: Float, val y: Float, val w: Float, val h: Float)

/** Fraction of the tile the face should roughly occupy after zooming. */
const val FACE_TARGET_FRACTION = 0.6f

/** Upper bound on the magnification so a tiny far-away face doesn't pixel-explode. */
const val FACE_MAX_ZOOM = 3f

/**
 * The sub-rectangle of the source image to draw.
 *  - [zx]/[zy] — visible fraction of the image per axis (1 = all of it).
 *  - [px]/[py] — where to pin that window, on the CSS `background-position`
 *    scale (0 = flush left/top, 1 = flush right/bottom). Shared with web so the
 *    two platforms cannot disagree about what the numbers mean.
 */
data class FaceCropRect(val zx: Float, val zy: Float, val px: Float, val py: Float)

/**
 * Position value that centres image-point [c] when [z] of the axis is visible.
 *
 * With container C and image I, the offset is p·(C − I), so image-point c lands
 * at p·(C − I) + c·I. Setting that to the centre C/2 and substituting z = C/I
 * gives p = (c − z/2)/(1 − z).
 *
 * At z = 1 the whole axis shows, so there is no pan freedom and no solution —
 * every p draws identically. Return the midpoint instead of dividing by zero.
 */
internal fun facePosition(c: Float, z: Float): Float =
    if (z >= 1f) 0.5f else ((c - z / 2f) / (1f - z)).coerceIn(0f, 1f)

/**
 * Map a normalised face box to the image window that frames it.
 *
 * The box is normalised against the whole photo and server thumbnails are
 * aspect-preserving downscales, so the fractions carry over unchanged — which
 * is what lets this work without knowing the image's aspect ratio.
 *
 * The window is the box scaled about its own centre by a single factor `k`, so
 * a face that is square in *pixels* produces a square window. Drawing that into
 * a square tile is therefore distortion-free even though [FaceCropRect.zx] and
 * [FaceCropRect.zy] differ — they differ by exactly the photo's aspect ratio,
 * which is the thing that cancels.
 *
 * Returns null for a missing or degenerate box; the caller draws a plain crop.
 */
fun faceCropRect(
    box: TileFaceBox?,
    targetFraction: Float = FACE_TARGET_FRACTION,
    minVisibleFraction: Float = 1f / FACE_MAX_ZOOM,
): FaceCropRect? {
    if (box == null) return null
    if (!box.x.isFinite() || !box.y.isFinite() || !box.w.isFinite() || !box.h.isFinite()) return null
    if (box.w <= 0f || box.h <= 0f) return null

    // A bbox larger than the photo is server garbage; clamp before it poisons
    // the ratios below.
    val bw = minOf(1f, box.w)
    val bh = minOf(1f, box.h)
    val m = maxOf(bw, bh)

    // `k` expands the box about its centre; the face then occupies 1/k of the
    // tile on both axes, so the target fraction is just k = 1/f — capped at 1/m
    // (any larger and the window runs off the photo) and floored at
    // minVisible/m (any smaller and we upscale a thumbnail into mush).
    // k >= 1 falls out of this: both bounds and the target are >= 1.
    val k = minOf(
        1f / m,
        maxOf(minVisibleFraction / m, 1f / maxOf(targetFraction, 0.0001f)),
    )

    val zx = minOf(1f, bw * k)
    val zy = minOf(1f, bh * k)
    val cx = (box.x + bw / 2f).coerceIn(0f, 1f)
    val cy = (box.y + bh / 2f).coerceIn(0f, 1f)

    return FaceCropRect(zx = zx, zy = zy, px = facePosition(cx, zx), py = facePosition(cy, zy))
}
