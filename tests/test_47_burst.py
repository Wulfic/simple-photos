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


class TestBurstSearchCollapse:
    """Search results must collapse burst stacks to a single representative
    (with a frame count), matching the gallery, smart-album, and secure grids
    — which all collapse bursts. Regression for "bursts not collapsed in
    search": the endpoint used to return every frame as its own result.
    """

    def test_search_collapses_burst_to_one(self, user_client):
        token = f"burstsrch{int(time.time())}"
        burst_id = f"burst-search-{int(time.time())}"
        ids = []
        for i in range(3):
            # Shared filename token so the search matches every frame; varying
            # width keeps the bytes distinct so upload-time hash dedup doesn't
            # drop frames before they're indexed.
            content = generate_burst_jpeg(burst_id, width=200 + i)
            pid = _upload(user_client, f"{token}_{i}.jpg", content)
            ids.append(pid)
        for pid in ids:
            _wait_for_photo(user_client, pid)

        results = user_client.search(token)["results"]
        matching = [r for r in results if r.get("burst_id") == burst_id]
        assert len(matching) == 1, (
            f"Burst not collapsed in search: expected 1 result, got {len(matching)}"
        )
        assert matching[0]["burst_count"] == 3, (
            f"Expected burst_count=3, got {matching[0].get('burst_count')}"
        )

    def test_search_normal_photo_has_null_burst(self, user_client):
        token = f"normsrch{int(time.time())}"
        pid = _upload(user_client, f"{token}.jpg", generate_normal_jpeg())
        _wait_for_photo(user_client, pid)

        results = user_client.search(token)["results"]
        match = next((r for r in results if r["id"] == pid), None)
        assert match is not None, "Normal photo missing from search results"
        assert match.get("burst_id") is None
        assert match.get("burst_count") is None


class TestBurstDetectionPrecedence:
    """XMP-derived burst_id must take precedence over the timestamp-based
    grouper.  Regression for todo P0-8: a photo that already carries an
    XMP `GCamera:BurstID` must not be reassigned to a different group when
    `POST /api/photos/detect-bursts` is invoked, even if its timestamp is
    within the burst-gap window of an unrelated photo from the same camera.
    """

    def test_xmp_burst_id_survives_timestamp_detector(self, user_client):
        # Upload three frames sharing an XMP BurstID.
        burst_id = f"xmp-precedence-{int(time.time())}"
        xmp_pids = []
        for i in range(3):
            content = generate_burst_jpeg(burst_id, width=200 + i)
            pid = _upload(user_client, unique_filename(), content)
            xmp_pids.append(pid)
        for pid in xmp_pids:
            _wait_for_photo(user_client, pid)

        # Trigger timestamp-based detection.  This must not touch the
        # XMP-grouped photos because they already have a burst_id.
        r = user_client.post("/api/photos/detect-bursts")
        assert r.status_code == 200, r.text[:200]

        # All three photos must still report the original XMP burst_id.
        data = user_client.list_photos(subtype="burst")
        for pid in xmp_pids:
            match = next((p for p in data["photos"] if p["id"] == pid), None)
            assert match is not None, f"Photo {pid} disappeared from burst list"
            assert match["burst_id"] == burst_id, (
                f"XMP burst_id was overwritten by timestamp detector: "
                f"expected {burst_id!r}, got {match['burst_id']!r}"
            )

    def test_timestamp_detector_skips_photos_with_subtype(self, user_client):
        # A photo carrying photo_subtype != 'burst' (e.g. motion / panorama)
        # must not be roped into a timestamp-derived burst group.  We can't
        # easily build a synthetic motion photo here without mp4 trailers,
        # so we cover the simpler invariant: a single XMP-burst photo plus
        # a normal photo must not get grouped together by the timestamp
        # detector even though they share a camera and were uploaded back-
        # to-back (timestamps within the 2-second burst window).
        burst_id = f"xmp-solo-{int(time.time())}"
        burst_pid = _upload(
            user_client, unique_filename(), generate_burst_jpeg(burst_id),
        )
        normal_pid = _upload(
            user_client, unique_filename(), generate_normal_jpeg(),
        )
        _wait_for_photo(user_client, burst_pid)
        _wait_for_photo(user_client, normal_pid)

        r = user_client.post("/api/photos/detect-bursts")
        assert r.status_code == 200

        data = user_client.list_photos()
        burst_photo = next(p for p in data["photos"] if p["id"] == burst_pid)
        normal_photo = next(p for p in data["photos"] if p["id"] == normal_pid)

        # The normal photo must NOT have inherited the XMP photo's burst_id.
        assert burst_photo["burst_id"] == burst_id
        assert normal_photo["burst_id"] != burst_id, (
            "Timestamp detector pulled an unrelated photo into an XMP burst group"
        )
