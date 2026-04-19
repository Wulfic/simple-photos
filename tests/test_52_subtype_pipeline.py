"""
E2E: Photo Subtypes Pipeline — burst stacking, motion video, panorama/360.

Validates the full pipeline from upload → metadata extraction → sync endpoint →
API filters for burst, motion photo, and panorama/360° photo subtypes.

Key scenarios:
  1. Burst photos: upload with GCamera:BurstID XMP → stacking collapses to 1,
     burst endpoint returns all frames, encrypted-sync includes subtype fields
  2. Motion photos: upload with GCamera:MotionPhoto XMP → embedded video served,
     sync endpoint returns motion_video_blob_id
  3. Panorama: upload with GPano:ProjectionType XMP → subtype detected,
     sync endpoint returns photo_subtype
  4. Encrypted sync: all three subtype fields present in sync response
"""

import struct
import time

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


# ══════════════════════════════════════════════════════════════════════
# Test data generators (shared XMP injection utility)
# ══════════════════════════════════════════════════════════════════════

def _inject_xmp_into_jpeg(jpeg_bytes: bytes, xmp_xml: str, trailer: bytes = b"") -> bytes:
    """Inject XMP metadata into a JPEG after the SOI marker, with optional trailer."""
    assert jpeg_bytes[:2] == b'\xff\xd8', "Not a valid JPEG"
    xmp_header = b'http://ns.adobe.com/xap/1.0/\x00'
    xmp_data = xmp_header + xmp_xml.encode('utf-8')
    segment_length = len(xmp_data) + 2
    app1 = b'\xff\xe1' + struct.pack('>H', segment_length) + xmp_data
    result = jpeg_bytes[:2] + app1 + jpeg_bytes[2:]
    if trailer:
        result = result + trailer
    return result


_FTYP_BOX = b'\x00\x00\x00\x14ftypmp42\x00\x00\x00\x00mp42'
_FAKE_MP4 = _FTYP_BOX + b'\x00' * 128


def gen_burst(burst_id: str, w=200, h=150) -> bytes:
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        f'<rdf:Description GCamera:BurstID="{burst_id}"/>'
        '</rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(generate_test_jpeg(width=w, height=h), xmp)


def gen_motion() -> tuple[bytes, int]:
    trailer = _FAKE_MP4
    offset = len(trailer)
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description GCamera:MotionPhoto="1"'
        f' GCamera:MotionVideoOffset="{offset}"/>'
        '</rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(generate_test_jpeg(200, 150), xmp, trailer), offset


def gen_pano(projection: str = "cylindrical") -> bytes:
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        f'<rdf:Description GPano:ProjectionType="{projection}"/>'
        '</rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(generate_test_jpeg(400, 100), xmp)


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(c: APIClient, content: bytes) -> str:
    return c.upload_photo(unique_filename(), content=content)["photo_id"]


def _wait_photo(c: APIClient, pid: str, timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        for p in c.list_photos()["photos"]:
            if p["id"] == pid:
                return p
        time.sleep(0.3)
    raise TimeoutError(f"Photo {pid} not found after {timeout}s")


# ══════════════════════════════════════════════════════════════════════
# Tests
# ══════════════════════════════════════════════════════════════════════

class TestBurstPipeline:
    """Burst photos: stacking, collapse, frame browsing, sync fields."""

    def test_burst_sync_includes_subtype_fields(self, user_client: APIClient):
        """Encrypted-sync returns photo_subtype, burst_id for burst photos."""
        bid = f"synctest-{int(time.time())}"
        pid = _upload(user_client, gen_burst(bid))
        _wait_photo(user_client, pid)

        sync = user_client.encrypted_sync()
        match = [p for p in sync["photos"] if p["id"] == pid]
        assert len(match) == 1
        assert match[0]["photo_subtype"] == "burst"
        assert match[0]["burst_id"] == bid

    def test_burst_collapse_returns_single_representative(self, user_client: APIClient):
        """collapse_bursts=true groups N burst frames into 1 with burst_count."""
        bid = f"collapse-{int(time.time())}"
        ids = [_upload(user_client, gen_burst(bid, w=200 + i)) for i in range(4)]
        for pid in ids:
            _wait_photo(user_client, pid)

        data = user_client.list_photos(subtype="burst", collapse_bursts="true")
        collapsed = [p for p in data["photos"] if p["burst_id"] == bid]
        assert len(collapsed) == 1, f"Expected 1 collapsed, got {len(collapsed)}"
        assert collapsed[0]["burst_count"] == 4

    def test_burst_endpoint_returns_all_frames_ordered(self, user_client: APIClient):
        """GET /api/photos/burst/{id} returns all frames in taken_at order."""
        bid = f"order-{int(time.time())}"
        ids = [_upload(user_client, gen_burst(bid, w=200 + i)) for i in range(3)]
        for pid in ids:
            _wait_photo(user_client, pid)

        r = user_client.get(f"/api/photos/burst/{bid}")
        assert r.status_code == 200
        frames = r.json()
        assert len(frames) == 3
        returned_ids = {f["id"] for f in frames}
        for pid in ids:
            assert pid in returned_ids

    def test_burst_unknown_id_404(self, user_client: APIClient):
        r = user_client.get("/api/photos/burst/nonexistent-burst-xyz")
        assert r.status_code == 404


class TestMotionPhotoPipeline:
    """Motion photos: detection, video extraction, sync fields."""

    def test_motion_sync_includes_subtype_and_video_blob(self, user_client: APIClient):
        """Encrypted-sync returns photo_subtype='motion' and motion_video_blob_id."""
        content, offset = gen_motion()
        pid = _upload(user_client, content)
        _wait_photo(user_client, pid)

        sync = user_client.encrypted_sync()
        match = [p for p in sync["photos"] if p["id"] == pid]
        assert len(match) == 1
        assert match[0]["photo_subtype"] == "motion"
        # motion_video_blob_id should be set (non-null)
        assert match[0]["motion_video_blob_id"] is not None

    def test_motion_video_endpoint_serves_mp4(self, user_client: APIClient):
        """GET /api/photos/{id}/motion-video returns video/mp4 with ftyp box."""
        content, offset = gen_motion()
        pid = _upload(user_client, content)
        _wait_photo(user_client, pid)

        r = user_client.get(f"/api/photos/{pid}/motion-video")
        assert r.status_code == 200
        assert r.headers.get("Content-Type") == "video/mp4"
        assert r.content[4:8] == b"ftyp"
        assert len(r.content) == offset

    def test_motion_video_rejected_for_normal_photo(self, user_client: APIClient):
        """Non-motion photo should return 400 on motion-video endpoint."""
        pid = _upload(user_client, generate_test_jpeg())
        _wait_photo(user_client, pid)
        r = user_client.get(f"/api/photos/{pid}/motion-video")
        assert r.status_code == 400


class TestPanoramaPipeline:
    """Panorama/360: detection, sync fields, filter."""

    def test_panorama_sync_subtype(self, user_client: APIClient):
        """Cylindrical panorama appears in sync with subtype='panorama'."""
        pid = _upload(user_client, gen_pano("cylindrical"))
        _wait_photo(user_client, pid)

        sync = user_client.encrypted_sync()
        match = [p for p in sync["photos"] if p["id"] == pid]
        assert len(match) == 1
        assert match[0]["photo_subtype"] == "panorama"

    def test_equirectangular_sync_subtype(self, user_client: APIClient):
        """Equirectangular 360° photo in sync with subtype='equirectangular'."""
        pid = _upload(user_client, gen_pano("equirectangular"))
        _wait_photo(user_client, pid)

        sync = user_client.encrypted_sync()
        match = [p for p in sync["photos"] if p["id"] == pid]
        assert len(match) == 1
        assert match[0]["photo_subtype"] == "equirectangular"

    def test_panorama_filter(self, user_client: APIClient):
        """Subtype filter correctly separates panorama from equirectangular."""
        pano_id = _upload(user_client, gen_pano("cylindrical"))
        eq_id = _upload(user_client, gen_pano("equirectangular"))
        normal_id = _upload(user_client, generate_test_jpeg())
        _wait_photo(user_client, pano_id)
        _wait_photo(user_client, eq_id)
        _wait_photo(user_client, normal_id)

        pano_data = user_client.list_photos(subtype="panorama")
        pano_ids = [p["id"] for p in pano_data["photos"]]
        assert pano_id in pano_ids
        assert eq_id not in pano_ids

        eq_data = user_client.list_photos(subtype="equirectangular")
        eq_ids = [p["id"] for p in eq_data["photos"]]
        assert eq_id in eq_ids
        assert pano_id not in eq_ids


class TestSyncSubtypeFields:
    """Encrypted-sync returns null subtype fields for normal photos."""

    def test_normal_photo_null_subtypes(self, user_client: APIClient):
        pid = _upload(user_client, generate_test_jpeg())
        _wait_photo(user_client, pid)

        sync = user_client.encrypted_sync()
        match = [p for p in sync["photos"] if p["id"] == pid]
        assert len(match) == 1
        assert match[0]["photo_subtype"] is None
        assert match[0]["burst_id"] is None
        assert match[0]["motion_video_blob_id"] is None

    def test_sync_cursor_pagination_includes_subtypes(self, user_client: APIClient):
        """Subtype fields are present even in paginated results."""
        bid = f"paginate-{int(time.time())}"
        pid = _upload(user_client, gen_burst(bid))
        _wait_photo(user_client, pid)

        # Use a very small limit to force pagination
        sync = user_client.encrypted_sync(limit=1)
        # At least one page should have our burst photo or a next cursor
        all_photos = sync["photos"]
        cursor = sync.get("next_cursor")
        while cursor:
            page = user_client.encrypted_sync(after=cursor, limit=1)
            all_photos.extend(page["photos"])
            cursor = page.get("next_cursor")

        match = [p for p in all_photos if p["id"] == pid]
        assert len(match) == 1
        assert match[0]["photo_subtype"] == "burst"
        assert match[0]["burst_id"] == bid
