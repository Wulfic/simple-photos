"""
E2E: Panorama / 360° Photo — subtype detection and filter API.

DDT cases:
- JPEG with GPano:ProjectionType="equirectangular" → subtype "equirectangular"
- JPEG with GPano:ProjectionType="cylindrical" → subtype "panorama"
- Regular wide-aspect JPEG without GPano → subtype null (no false positive)
- Subtype filter returns correct photos
"""

import struct
import time

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


# ── Test data generators ─────────────────────────────────────────────────────


def _inject_xmp_into_jpeg(jpeg_bytes: bytes, xmp_xml: str) -> bytes:
    """Inject XMP metadata into a JPEG file after the SOI marker."""
    assert jpeg_bytes[:2] == b'\xff\xd8', "Not a valid JPEG"
    xmp_header = b'http://ns.adobe.com/xap/1.0/\x00'
    xmp_data = xmp_header + xmp_xml.encode('utf-8')
    segment_length = len(xmp_data) + 2
    app1 = b'\xff\xe1' + struct.pack('>H', segment_length) + xmp_data
    return jpeg_bytes[:2] + app1 + jpeg_bytes[2:]


def generate_equirectangular_jpeg(width=400, height=200):
    """Generate a JPEG with GPano:ProjectionType='equirectangular' (360° photo)."""
    jpeg = generate_test_jpeg(width=width, height=height)
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description'
        ' GPano:ProjectionType="equirectangular"'
        ' GPano:CroppedAreaImageWidthPixels="400"'
        ' GPano:FullPanoWidthPixels="400"'
        '/></rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(jpeg, xmp)


def generate_cylindrical_pano_jpeg(width=400, height=100):
    """Generate a JPEG with GPano:ProjectionType='cylindrical' (panorama)."""
    jpeg = generate_test_jpeg(width=width, height=height)
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description'
        ' GPano:ProjectionType="cylindrical"'
        '/></rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(jpeg, xmp)


def generate_wide_jpeg_no_xmp(width=400, height=100):
    """Generate a wide aspect ratio JPEG without any GPano metadata."""
    return generate_test_jpeg(width=width, height=height)


# ── Helpers ──────────────────────────────────────────────────────────────────

def _upload(client: APIClient, filename: str, content: bytes):
    result = client.upload_photo(filename=filename, content=content, mime_type="image/jpeg")
    return result["photo_id"]


def _find_photo(client: APIClient, photo_id: str):
    data = client.list_photos()
    for p in data["photos"]:
        if p["id"] == photo_id:
            return p
    return None


def _wait_for_photo(client: APIClient, photo_id: str, timeout: float = 10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        found = _find_photo(client, photo_id)
        if found:
            return found
        time.sleep(0.3)
    raise TimeoutError(f"Photo {photo_id} not found in listing after {timeout}s")


# ── DDT: Panorama/360 detection ─────────────────────────────────────────────

PANO_DETECTION_CASES = [
    pytest.param(
        "equirect.jpg", generate_equirectangular_jpeg,
        "equirectangular",
        id="equirectangular_360",
    ),
    pytest.param(
        "cylindrical.jpg", generate_cylindrical_pano_jpeg,
        "panorama",
        id="cylindrical_panorama",
    ),
    pytest.param(
        "wide_no_xmp.jpg", generate_wide_jpeg_no_xmp,
        None,
        id="wide_jpeg_no_false_positive",
    ),
]


class TestPanoramaDetection:
    """Verify that panoramic and 360° photos are detected via XMP metadata."""

    @pytest.mark.parametrize("filename,gen_func,expected_subtype", PANO_DETECTION_CASES)
    def test_subtype_detected(self, user_client, filename, gen_func, expected_subtype):
        content = gen_func()
        photo_id = _upload(user_client, unique_filename(), content)
        photo = _wait_for_photo(user_client, photo_id)
        assert photo["photo_subtype"] == expected_subtype, \
            f"Expected subtype '{expected_subtype}', got '{photo.get('photo_subtype')}'"


class TestPanoramaFilter:
    """Verify the subtype filter works for panorama and equirectangular subtypes."""

    def test_filter_equirectangular(self, user_client):
        eq_id = _upload(user_client, unique_filename(), generate_equirectangular_jpeg())
        pano_id = _upload(user_client, unique_filename(), generate_cylindrical_pano_jpeg())
        normal_id = _upload(user_client, unique_filename(), generate_wide_jpeg_no_xmp())

        _wait_for_photo(user_client, eq_id)
        _wait_for_photo(user_client, pano_id)
        _wait_for_photo(user_client, normal_id)

        data = user_client.list_photos(subtype="equirectangular")
        ids = [p["id"] for p in data["photos"]]
        assert eq_id in ids
        assert pano_id not in ids
        assert normal_id not in ids

    def test_filter_panorama(self, user_client):
        eq_id = _upload(user_client, unique_filename(), generate_equirectangular_jpeg())
        pano_id = _upload(user_client, unique_filename(), generate_cylindrical_pano_jpeg())

        _wait_for_photo(user_client, eq_id)
        _wait_for_photo(user_client, pano_id)

        data = user_client.list_photos(subtype="panorama")
        ids = [p["id"] for p in data["photos"]]
        assert pano_id in ids
        assert eq_id not in ids

    def test_both_in_unfiltered(self, user_client):
        eq_id = _upload(user_client, unique_filename(), generate_equirectangular_jpeg())
        pano_id = _upload(user_client, unique_filename(), generate_cylindrical_pano_jpeg())

        _wait_for_photo(user_client, eq_id)
        _wait_for_photo(user_client, pano_id)

        data = user_client.list_photos()
        ids = [p["id"] for p in data["photos"]]
        assert eq_id in ids
        assert pano_id in ids
