"""
E2E DDT: Panorama / 360° aspect-ratio fallback detection.

When XMP `GPano:ProjectionType` is missing or stripped, the server falls
back to image dimensions to assign `photo_subtype`:

* aspect ∈ [1.97, 2.03] **and** ``width >= 4000`` → ``equirectangular``
* aspect ``>= 2.0`` (horizontal) OR ``h/w >= 2.5`` (vertical) →
  ``panorama``
* long edge ``< 2048`` or aspect ``< 2.0`` (and not a vertical pano) →
  no subtype assigned

The thresholds were tightened from the previous (1.8 / 1024 / 1500)
values which over-matched ultra-wide landscape shots and 2:1 wallpapers
as panoramas, and under-tagged because of the 1024-px minimum.
"""

from __future__ import annotations

import time

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


# ── helpers ────────────────────────────────────────────────────────────────


def _upload(client: APIClient, content: bytes) -> str:
    result = client.upload_photo(
        filename=unique_filename(),
        content=content,
        mime_type="image/jpeg",
    )
    return result["photo_id"]


def _wait_for_photo(client: APIClient, photo_id: str, timeout: float = 10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        for p in client.list_photos()["photos"]:
            if p["id"] == photo_id:
                return p
        time.sleep(0.25)
    raise TimeoutError(f"Photo {photo_id} not visible after {timeout}s")


# ── DDT table ──────────────────────────────────────────────────────────────
# Each row: (width, height, expected_subtype)
#
# Boundary, min, max, and negative cases each appear as distinct rows per
# project rule (DDT must cover boundaries and invalid inputs).

ASPECT_FALLBACK_CASES = [
    # ── True 360° equirectangular: 2:1 aspect AND width >= 4000 ─────
    pytest.param(5760, 2880, "equirectangular", id="equirect_5760x2880_pixel_360"),
    pytest.param(7680, 3840, "equirectangular", id="equirect_7680x3840_high_res_360"),
    pytest.param(4000, 2000, "equirectangular", id="equirect_min_width_4000"),

    # ── Panorama (cylindrical / wide stitch) ────────────────────────
    pytest.param(7000, 1000, "panorama", id="panorama_7to1"),
    pytest.param(9000, 2000, "panorama", id="panorama_45to1_real_phone_pano"),
    pytest.param(3000, 1000, "panorama", id="panorama_3to1"),
    # 2:1 but BELOW 4000 width → panorama, not equirectangular.
    pytest.param(3000, 1500, "panorama", id="panorama_2to1_below_4000"),
    pytest.param(2200, 1100, "panorama", id="panorama_min_long_edge_2048"),
    # Vertical panorama (Samsung "vertical pano" — h/w ≥ 2.5).
    pytest.param(1080, 4000, "panorama", id="vertical_panorama_3_7to1"),

    # ── Negatives: must NOT receive a subtype ───────────────────────
    pytest.param(800, 400, None, id="too_small_below_2048_long_edge"),
    pytest.param(1500, 1000, None, id="aspect_1_5_not_panorama"),
    pytest.param(4000, 3000, None, id="standard_4_3_photo"),
    pytest.param(1920, 1080, None, id="hd_16_9_not_panorama"),
    # Old 1.8:1 phone landscape — used to false-positive as panorama,
    # now correctly left untagged.
    pytest.param(5760, 3200, None, id="ultra_wide_18to1_landscape_no_longer_pano"),
    # 9:16 portrait video — must not be tagged vertical-panorama.
    pytest.param(2160, 3840, None, id="portrait_video_9_16_not_pano"),
    # Long-edge below 2048 (old 1024 threshold) must now be skipped.
    pytest.param(1024, 500, None, id="long_edge_1024_below_new_floor"),
]


@pytest.mark.parametrize("width,height,expected_subtype", ASPECT_FALLBACK_CASES)
def test_aspect_ratio_fallback_assigns_subtype(
    user_client: APIClient, width: int, height: int, expected_subtype
):
    """Upload a JPEG with no XMP and confirm aspect-ratio fallback."""
    content = generate_test_jpeg(width=width, height=height)
    photo_id = _upload(user_client, content)
    photo = _wait_for_photo(user_client, photo_id)
    assert photo.get("photo_subtype") == expected_subtype, (
        f"width={width} height={height} aspect={width/height:.3f} "
        f"expected={expected_subtype!r} got={photo.get('photo_subtype')!r}"
    )


def test_aspect_fallback_filter_returns_panorama(user_client: APIClient):
    """The /api/photos?subtype=panorama filter must include aspect-only panoramas."""
    pano_id = _upload(user_client, generate_test_jpeg(width=7000, height=1000))
    normal_id = _upload(user_client, generate_test_jpeg(width=800, height=600))
    _wait_for_photo(user_client, pano_id)
    _wait_for_photo(user_client, normal_id)

    data = user_client.list_photos(subtype="panorama")
    ids = [p["id"] for p in data["photos"]]
    assert pano_id in ids
    assert normal_id not in ids


def test_aspect_fallback_filter_returns_equirectangular(user_client: APIClient):
    """The equirectangular filter must include aspect-only 360° detections."""
    eq_id = _upload(user_client, generate_test_jpeg(width=3000, height=1500))
    pano_id = _upload(user_client, generate_test_jpeg(width=7000, height=1000))
    _wait_for_photo(user_client, eq_id)
    _wait_for_photo(user_client, pano_id)

    data = user_client.list_photos(subtype="equirectangular")
    ids = [p["id"] for p in data["photos"]]
    assert eq_id in ids
    assert pano_id not in ids
