"""
E2E: Motion Photo — subtype detection, video extraction, serve endpoint.

DDT cases:
- JPEG with MicroVideo XMP → subtype == "motion"
- JPEG with MotionPhoto XMP → subtype == "motion"
- Motion video endpoint returns video/mp4 content
- Regular JPEG → subtype is null
- Motion video Content-Length matches expected trailer size
"""

import io
import struct
import time

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


# ── Test data generators ─────────────────────────────────────────────────────

# Minimal valid MP4 (ftyp box only — enough for signature detection)
_FTYP_BOX = b'\x00\x00\x00\x14ftypmp42\x00\x00\x00\x00mp42'
# Pad to make it look like a real video
_FAKE_MP4 = _FTYP_BOX + b'\x00' * 128


def _inject_xmp_into_jpeg(jpeg_bytes: bytes, xmp_xml: str, trailer: bytes = b"") -> bytes:
    """Inject XMP metadata into a JPEG file after the SOI marker.

    Builds a valid APP1 segment containing the XMP namespace header
    followed by the XMP XML payload.  Optionally appends a trailer
    (e.g., an embedded MP4 for motion photos) after the JPEG EOI marker.
    """
    # JPEG structure: SOI (FFD8) ... segments ... EOI (FFD9)
    assert jpeg_bytes[:2] == b'\xff\xd8', "Not a valid JPEG"

    # XMP APP1 marker
    xmp_header = b'http://ns.adobe.com/xap/1.0/\x00'
    xmp_data = xmp_header + xmp_xml.encode('utf-8')
    # APP1 segment: FF E1 + 2-byte length (includes length field itself)
    segment_length = len(xmp_data) + 2
    app1 = b'\xff\xe1' + struct.pack('>H', segment_length) + xmp_data

    # Insert APP1 right after SOI marker
    result = jpeg_bytes[:2] + app1 + jpeg_bytes[2:]

    # Append trailer after the JPEG data
    if trailer:
        result = result + trailer

    return result


def generate_motion_photo_micro_video(width=200, height=150):
    """Generate a test JPEG with Camera:MicroVideo XMP and an embedded MP4 trailer."""
    jpeg = generate_test_jpeg(width=width, height=height)
    trailer = _FAKE_MP4
    offset = len(trailer)
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description'
        f' Camera:MicroVideo="1"'
        f' Camera:MicroVideoOffset="{offset}"'
        '/></rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(jpeg, xmp, trailer=trailer), offset


def generate_motion_photo_new_schema(width=200, height=150):
    """Generate a test JPEG with GCamera:MotionPhoto XMP (new schema)."""
    jpeg = generate_test_jpeg(width=width, height=height)
    trailer = _FAKE_MP4
    offset = len(trailer)
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description'
        f' GCamera:MotionPhoto="1"'
        f' GCamera:MotionVideoOffset="{offset}"'
        '/></rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(jpeg, xmp, trailer=trailer), offset


def generate_normal_jpeg(width=200, height=150):
    """Generate a regular JPEG with no XMP subtype markers."""
    return generate_test_jpeg(width=width, height=height)


# ── Helpers ──────────────────────────────────────────────────────────────────

def _upload(client: APIClient, filename: str, content: bytes, mime: str = "image/jpeg"):
    """Upload a photo and return the photo_id."""
    result = client.upload_photo(filename=filename, content=content, mime_type=mime)
    return result["photo_id"]


def _find_photo(client: APIClient, photo_id: str):
    """Find a photo in the listing by ID."""
    data = client.list_photos()
    for p in data["photos"]:
        if p["id"] == photo_id:
            return p
    return None


def _wait_for_photo(client: APIClient, photo_id: str, timeout: float = 10.0):
    """Poll until a photo appears in listing."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        found = _find_photo(client, photo_id)
        if found:
            return found
        time.sleep(0.3)
    raise TimeoutError(f"Photo {photo_id} not found in listing after {timeout}s")


# ── DDT: Motion photo detection ─────────────────────────────────────────────

MOTION_CASES = [
    pytest.param(
        "motion_micro.jpg", generate_motion_photo_micro_video,
        "motion", True,
        id="micro_video_old_schema",
    ),
    pytest.param(
        "motion_gcamera.jpg", generate_motion_photo_new_schema,
        "motion", True,
        id="motion_photo_new_schema",
    ),
]

NORMAL_CASES = [
    pytest.param(
        "regular.jpg", generate_normal_jpeg,
        None, False,
        id="regular_jpeg_no_subtype",
    ),
]


class TestMotionPhotoDetection:
    """Verify that motion photos uploaded via the upload endpoint are
    correctly tagged with photo_subtype == 'motion'."""

    @pytest.mark.parametrize("filename,gen_func,expected_subtype,is_motion", MOTION_CASES)
    def test_motion_subtype_detected(self, user_client, filename, gen_func, expected_subtype, is_motion):
        content, _offset = gen_func()
        photo_id = _upload(user_client, unique_filename(), content)
        photo = _wait_for_photo(user_client, photo_id)
        assert photo["photo_subtype"] == expected_subtype, \
            f"Expected subtype '{expected_subtype}', got '{photo.get('photo_subtype')}'"

    @pytest.mark.parametrize("filename,gen_func,expected_subtype,is_motion", NORMAL_CASES)
    def test_normal_photo_no_subtype(self, user_client, filename, gen_func, expected_subtype, is_motion):
        content = gen_func()
        photo_id = _upload(user_client, unique_filename(), content)
        photo = _wait_for_photo(user_client, photo_id)
        assert photo["photo_subtype"] is None, \
            f"Expected null subtype, got '{photo.get('photo_subtype')}'"


class TestMotionVideoEndpoint:
    """Verify that GET /api/photos/{id}/motion-video returns the embedded MP4."""

    def test_motion_video_returns_mp4(self, user_client):
        content, offset = generate_motion_photo_micro_video()
        photo_id = _upload(user_client, unique_filename(), content)
        _wait_for_photo(user_client, photo_id)

        r = user_client.get(f"/api/photos/{photo_id}/motion-video")
        assert r.status_code == 200, f"Expected 200, got {r.status_code}: {r.text}"
        assert r.headers.get("Content-Type") == "video/mp4"

    def test_motion_video_content_length(self, user_client):
        content, offset = generate_motion_photo_micro_video()
        photo_id = _upload(user_client, unique_filename(), content)
        _wait_for_photo(user_client, photo_id)

        r = user_client.get(f"/api/photos/{photo_id}/motion-video")
        assert r.status_code == 200
        # The body size should match the trailer size
        assert len(r.content) == offset

    def test_motion_video_has_ftyp(self, user_client):
        content, offset = generate_motion_photo_micro_video()
        photo_id = _upload(user_client, unique_filename(), content)
        _wait_for_photo(user_client, photo_id)

        r = user_client.get(f"/api/photos/{photo_id}/motion-video")
        assert r.status_code == 200
        # MP4 ftyp box at bytes 4-8
        assert r.content[4:8] == b"ftyp", "Motion video should start with ftyp box"

    def test_non_motion_photo_rejected(self, user_client):
        content = generate_normal_jpeg()
        photo_id = _upload(user_client, unique_filename(), content)
        _wait_for_photo(user_client, photo_id)

        r = user_client.get(f"/api/photos/{photo_id}/motion-video")
        assert r.status_code == 400, f"Expected 400 for non-motion photo, got {r.status_code}"


class TestMotionPhotoFilter:
    """Verify the subtype filter on the photo listing endpoint."""

    def test_filter_motion_only(self, user_client):
        # Upload one motion + one normal
        motion_content, _ = generate_motion_photo_micro_video()
        motion_id = _upload(user_client, unique_filename(), motion_content)
        normal_id = _upload(user_client, unique_filename(), generate_normal_jpeg())

        _wait_for_photo(user_client, motion_id)
        _wait_for_photo(user_client, normal_id)

        # Filter by subtype=motion
        data = user_client.list_photos(subtype="motion")
        photo_ids = [p["id"] for p in data["photos"]]
        assert motion_id in photo_ids
        assert normal_id not in photo_ids

    def test_filter_no_subtype_returns_all(self, user_client):
        motion_content, _ = generate_motion_photo_micro_video()
        motion_id = _upload(user_client, unique_filename(), motion_content)
        normal_id = _upload(user_client, unique_filename(), generate_normal_jpeg())

        _wait_for_photo(user_client, motion_id)
        _wait_for_photo(user_client, normal_id)

        # No subtype filter — both should appear
        data = user_client.list_photos()
        photo_ids = [p["id"] for p in data["photos"]]
        assert motion_id in photo_ids
        assert normal_id in photo_ids
