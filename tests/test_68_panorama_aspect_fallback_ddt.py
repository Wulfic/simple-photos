"""
E2E DDT: Panorama / 360° aspect-ratio fallback detection.

When XMP `GPano:ProjectionType` is missing or stripped, the server falls
back to image dimensions to assign `photo_subtype`:

* ``aspect ≈ 2.0`` (1.95–2.05) and ``width >= 1500`` → ``equirectangular``
* ``aspect >= 2.0`` otherwise → ``panorama``
* ``width < 1024`` or ``aspect < 2.0`` → no subtype assigned

These cases mirror real-world samples in
``~/Desktop/Sample_files/image/`` (true 360° JPEGs and stitched
panoramas without XMP markers).
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
    # ── True 360° equirectangular: 2:1 aspect, width >= 1500 ────────
    pytest.param(3000, 1500, "equirectangular", id="equirect_3000x1500"),
    pytest.param(4096, 2048, "equirectangular", id="equirect_4096x2048"),
    pytest.param(1500, 750, "equirectangular", id="equirect_min_width_1500"),
    pytest.param(2050, 1000, "equirectangular", id="equirect_aspect_2_05"),

    # ── Panorama (cylindrical / wide stitch) ────────────────────────
    pytest.param(7000, 1000, "panorama", id="panorama_7to1"),
    pytest.param(2660, 1000, "panorama", id="panorama_2_66to1"),
    pytest.param(3000, 1000, "panorama", id="panorama_3to1"),
    # 2:1 but BELOW 1500 width → still panorama, not equirectangular.
    pytest.param(1200, 600, "panorama", id="panorama_2to1_below_1500"),
    pytest.param(1024, 500, "panorama", id="panorama_min_width_1024"),

    # ── Negatives: must NOT receive a subtype ───────────────────────
    pytest.param(800, 400, None, id="too_narrow_below_1024"),
    pytest.param(1500, 1000, None, id="aspect_1_5_not_panorama"),
    pytest.param(4000, 3000, None, id="standard_4_3_photo"),
    pytest.param(1920, 1080, None, id="hd_16_9_not_panorama"),
    pytest.param(1024, 600, None, id="aspect_1_7_not_panorama"),
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
