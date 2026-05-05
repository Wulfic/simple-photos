"""
E2E DDT: Panorama / equirectangular thumbnail preview resolution.

Standard photos still produce a 512px long-edge thumbnail. **Panoramas**
(aspect ≥ 2 in either direction) are now scaled by short-edge to ~384 px
so a 7000×1000 stitch is delivered as ~2688×384 instead of 512×73.

These tests upload synthetic JPEGs at known dimensions and assert the
returned thumbnail's pixel size matches the expected target rule.
"""

from __future__ import annotations

import io
import time

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


def _wait_thumb(client: APIClient, photo_id: str, timeout: float = 15.0) -> bytes:
    deadline = time.time() + timeout
    while time.time() < deadline:
        r = client.get_photo_thumb(photo_id)
        if r.status_code == 200 and r.content:
            return r.content
        time.sleep(0.3)
    raise TimeoutError(f"Thumbnail for {photo_id} not ready")


def _thumb_dims(content: bytes):
    from PIL import Image
    img = Image.open(io.BytesIO(content))
    img.load()
    return img.size


# Each row: (src_w, src_h, expected_short_edge, kind)
# Standard photo:    long edge <= 512
# Wide panorama:     short edge <= 384, long edge > 512 (proves no 512 cap)
# Tall panorama:     same in transposed direction.

PREVIEW_RES_CASES = [
    pytest.param(7000, 1000, 384, "panorama", id="wide_7to1_short_edge_384"),
    pytest.param(3000, 1500, 384, "equirectangular", id="equirect_2to1_short_edge_384"),
    pytest.param(2660, 1000, 384, "panorama", id="panorama_2_66to1"),
    pytest.param(1024, 500, 384, "panorama", id="panorama_min_width_short_edge_384"),
    pytest.param(4000, 3000, 512, "standard", id="standard_4_3_long_edge_512"),
    pytest.param(1920, 1080, 512, "standard", id="standard_16_9_long_edge_512"),
    pytest.param(800, 600, 512, "standard", id="small_4_3_long_edge_512"),
]


@pytest.mark.parametrize("src_w,src_h,expected,kind", PREVIEW_RES_CASES)
def test_panorama_preview_resolution(
    user_client: APIClient, src_w: int, src_h: int, expected: int, kind: str
):
    content = generate_test_jpeg(width=src_w, height=src_h)
    photo = user_client.upload_photo(
        filename=unique_filename(), content=content, mime_type="image/jpeg"
    )
    thumb = _wait_thumb(user_client, photo["photo_id"])
    tw, th = _thumb_dims(thumb)
    src_aspect = src_w / src_h
    if kind in ("panorama", "equirectangular"):
        # Short edge ≤ ~384 (allow ±2 px for filter rounding).
        short = min(tw, th)
        assert abs(short - expected) <= 2, (
            f"{kind} {src_w}×{src_h}: expected short edge ≈ {expected}, "
            f"got {tw}×{th}"
        )
        # Aspect ratio preserved (within 1%).
        thumb_aspect = tw / th
        assert abs(thumb_aspect - src_aspect) / src_aspect < 0.02, (
            f"Aspect ratio drift: src {src_aspect:.3f} vs thumb {thumb_aspect:.3f}"
        )
    else:
        long = max(tw, th)
        assert abs(long - expected) <= 2, (
            f"standard {src_w}×{src_h}: expected long edge ≈ {expected}, "
            f"got {tw}×{th}"
        )
