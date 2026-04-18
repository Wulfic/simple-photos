"""
E2E: HDR Photo — subtype detection from Gainmap XMP.

DDT cases:
- JPEG with hdrgm:Version XMP → subtype "hdr"
- JPEG with HDRGainMap XMP → subtype "hdr"
- Regular JPEG → subtype null
- Served file preserves original data (Gainmap not stripped)
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


def generate_hdr_gainmap_jpeg(width=200, height=150):
    """Generate a JPEG with hdrgm:Version XMP (Ultra HDR Gainmap)."""
    jpeg = generate_test_jpeg(width=width, height=height)
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description'
        ' hdrgm:Version="1.0"'
        ' hdrgm:HDRCapacityMin="0"'
        ' hdrgm:HDRCapacityMax="2.3"'
        '/></rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(jpeg, xmp)


def generate_hdr_gainmap_alt_jpeg(width=200, height=150):
    """Generate a JPEG with HDRGainMap marker (alternative detection)."""
    jpeg = generate_test_jpeg(width=width, height=height)
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description HDRGainMap="true"'
        '/></rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(jpeg, xmp)


def generate_normal_jpeg(width=200, height=150):
    """Regular JPEG with no HDR markers."""
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


# ── DDT: HDR detection ──────────────────────────────────────────────────────

HDR_DETECTION_CASES = [
    pytest.param(
        "hdr_gainmap.jpg", generate_hdr_gainmap_jpeg,
        "hdr",
        id="hdr_gainmap_version",
    ),
    pytest.param(
        "hdr_alt.jpg", generate_hdr_gainmap_alt_jpeg,
        "hdr",
        id="hdr_gainmap_alt_marker",
    ),
    pytest.param(
        "normal.jpg", generate_normal_jpeg,
        None,
        id="normal_jpeg_no_hdr",
    ),
]


class TestHDRDetection:
    """Verify that HDR Gainmap photos are detected via XMP metadata."""

    @pytest.mark.parametrize("filename,gen_func,expected_subtype", HDR_DETECTION_CASES)
    def test_subtype_detected(self, user_client, filename, gen_func, expected_subtype):
        content = gen_func()
        photo_id = _upload(user_client, unique_filename(), content)
        photo = _wait_for_photo(user_client, photo_id)
        assert photo["photo_subtype"] == expected_subtype, \
            f"Expected subtype '{expected_subtype}', got '{photo.get('photo_subtype')}'"


class TestHDRFilter:
    """Verify the subtype filter works for HDR photos."""

    def test_filter_hdr(self, user_client):
        hdr_id = _upload(user_client, unique_filename(), generate_hdr_gainmap_jpeg())
        normal_id = _upload(user_client, unique_filename(), generate_normal_jpeg())

        _wait_for_photo(user_client, hdr_id)
        _wait_for_photo(user_client, normal_id)

        data = user_client.list_photos(subtype="hdr")
        ids = [p["id"] for p in data["photos"]]
        assert hdr_id in ids
        assert normal_id not in ids


class TestHDRFileIntegrity:
    """Verify that the served file preserves the Gainmap XMP data."""

    def test_served_file_contains_gainmap(self, user_client):
        content = generate_hdr_gainmap_jpeg()
        photo_id = _upload(user_client, unique_filename(), content)
        _wait_for_photo(user_client, photo_id)

        r = user_client.get_photo_file(photo_id)
        assert r.status_code == 200
        # The served file should contain the Gainmap XMP string
        assert b"hdrgm:Version" in r.content, \
            "Served file should preserve the HDR Gainmap XMP metadata"
