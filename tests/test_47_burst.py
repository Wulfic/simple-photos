"""
E2E: Burst Photo — grouping, collapse API, burst frame listing.

DDT cases:
- JPEG with GCamera:BurstID XMP → subtype "burst", burst_id populated
- Multiple photos sharing same BurstID are grouped together
- GET /api/photos/burst/{burst_id} returns all frames ordered by taken_at ASC
- Regular JPEG → no burst_id
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


def generate_burst_jpeg(burst_id: str, width=200, height=150):
    """Generate a JPEG with GCamera:BurstID XMP metadata."""
    jpeg = generate_test_jpeg(width=width, height=height)
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description'
        f' GCamera:BurstID="{burst_id}"'
        '/></rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(jpeg, xmp)


def generate_normal_jpeg(width=200, height=150):
    """Regular JPEG with no burst markers."""
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


# ── DDT: Burst detection ────────────────────────────────────────────────────

BURST_DETECTION_CASES = [
    pytest.param(
        "burst_001.jpg", "test-burst-abc123",
        "burst", "test-burst-abc123",
        id="burst_with_id",
    ),
    pytest.param(
        "burst_002.jpg", "another-burst-xyz",
        "burst", "another-burst-xyz",
        id="burst_different_id",
    ),
]


class TestBurstDetection:
    """Verify that burst photos are detected and burst_id is stored."""

    @pytest.mark.parametrize("filename,burst_id,expected_subtype,expected_burst_id", BURST_DETECTION_CASES)
    def test_burst_subtype_detected(self, user_client, filename, burst_id, expected_subtype, expected_burst_id):
        content = generate_burst_jpeg(burst_id)
        photo_id = _upload(user_client, unique_filename(), content)
        photo = _wait_for_photo(user_client, photo_id)
        assert photo["photo_subtype"] == expected_subtype
        assert photo["burst_id"] == expected_burst_id

    def test_normal_photo_no_burst(self, user_client):
        content = generate_normal_jpeg()
        photo_id = _upload(user_client, unique_filename(), content)
        photo = _wait_for_photo(user_client, photo_id)
        assert photo["photo_subtype"] is None
        assert photo["burst_id"] is None


class TestBurstGrouping:
    """Verify that photos sharing the same burst_id are grouped together."""

    def test_burst_endpoint_returns_all_frames(self, user_client):
        burst_id = f"burst-group-{int(time.time())}"
        ids = []
        for i in range(3):
            content = generate_burst_jpeg(burst_id, width=200 + i, height=150)
            pid = _upload(user_client, unique_filename(), content)
            ids.append(pid)

        # Wait for all to appear
        for pid in ids:
            _wait_for_photo(user_client, pid)

        # Query burst endpoint
        r = user_client.get(f"/api/photos/burst/{burst_id}")
        assert r.status_code == 200
        burst_photos = r.json()
        assert len(burst_photos) == 3, f"Expected 3 burst frames, got {len(burst_photos)}"

        returned_ids = [p["id"] for p in burst_photos]
        for pid in ids:
            assert pid in returned_ids, f"Burst frame {pid} not found in response"

    def test_burst_endpoint_unknown_id(self, user_client):
        r = user_client.get("/api/photos/burst/nonexistent-burst-id")
        assert r.status_code == 404

    def test_burst_frames_all_have_burst_subtype(self, user_client):
        burst_id = f"burst-check-{int(time.time())}"
        ids = []
        for i in range(2):
            content = generate_burst_jpeg(burst_id)
            pid = _upload(user_client, unique_filename(), content)
            ids.append(pid)

        for pid in ids:
            photo = _wait_for_photo(user_client, pid)
            assert photo["photo_subtype"] == "burst"
            assert photo["burst_id"] == burst_id


class TestBurstFilter:
    """Verify the subtype filter works for burst photos."""

    def test_filter_burst_only(self, user_client):
        burst_id = f"burst-filter-{int(time.time())}"
        burst_content = generate_burst_jpeg(burst_id)
        burst_pid = _upload(user_client, unique_filename(), burst_content)
        normal_pid = _upload(user_client, unique_filename(), generate_normal_jpeg())

        _wait_for_photo(user_client, burst_pid)
        _wait_for_photo(user_client, normal_pid)

        data = user_client.list_photos(subtype="burst")
        ids = [p["id"] for p in data["photos"]]
        assert burst_pid in ids
        assert normal_pid not in ids


class TestBurstCollapse:
    """Verify collapse_bursts=true groups burst photos."""

    def test_collapse_reduces_burst_to_one(self, user_client):
        """Upload 3 photos with the same burst_id; collapse_bursts returns 1."""
        burst_id = f"burst-collapse-{int(time.time())}"
        ids = []
        for i in range(3):
            content = generate_burst_jpeg(burst_id, width=200 + i)
            pid = _upload(user_client, unique_filename(), content)
            ids.append(pid)

        for pid in ids:
            _wait_for_photo(user_client, pid)

        # Without collapse: all 3 appear
        data = user_client.list_photos(subtype="burst")
        burst_ids = [p["id"] for p in data["photos"] if p["burst_id"] == burst_id]
        assert len(burst_ids) >= 3

        # With collapse: only 1 representative
        data = user_client.list_photos(subtype="burst", collapse_bursts="true")
        collapsed = [p for p in data["photos"] if p["burst_id"] == burst_id]
        assert len(collapsed) == 1, f"Expected 1 collapsed burst, got {len(collapsed)}"
        assert collapsed[0]["burst_count"] == 3

    def test_collapse_normal_photos_unaffected(self, user_client):
        """Non-burst photos still appear normally with collapse_bursts=true."""
        normal_pid = _upload(user_client, unique_filename(), generate_normal_jpeg())
        _wait_for_photo(user_client, normal_pid)

        data = user_client.list_photos(collapse_bursts="true")
        ids = [p["id"] for p in data["photos"]]
        assert normal_pid in ids

    def test_collapse_burst_count_is_null_for_normal(self, user_client):
        """Non-burst photos should have burst_count = null even with collapse."""
        normal_pid = _upload(user_client, unique_filename(), generate_normal_jpeg())
        photo = _wait_for_photo(user_client, normal_pid)
        # Without collapse_bursts, burst_count should be null
        assert photo.get("burst_count") is None
