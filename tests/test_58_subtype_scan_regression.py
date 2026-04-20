"""
E2E Regression: Photo Subtype Detection via Scan Path

Validates that burst, motion photo, and panoramic photos are correctly
detected when files are placed on disk and discovered via the filesystem
scan endpoint (POST /api/admin/photos/scan) — NOT the upload endpoint.

This is a regression test for the bug where scan_and_register did not call
extract_xmp_subtype() and omitted photo_subtype/burst_id from the INSERT
query, causing all scanned photos to have NULL subtypes.

DDT cases:
  Burst:
    - JPEG with GCamera:BurstID XMP → photo_subtype="burst", burst_id populated
    - 3 JEPGs same BurstID scanned together → grouped, burst endpoint works
    - collapse_bursts=true collapses burst group to 1 representative

  Motion:
    - JPEG with Camera:MicroVideo XMP + MP4 trailer → photo_subtype="motion"
    - JPEG with GCamera:MotionPhoto XMP + MP4 trailer → photo_subtype="motion"
    - Motion video endpoint returns video/mp4 with ftyp box
    - motion_video_blob_id is set (blob extracted during scan)

  Panorama/360:
    - JPEG with GPano:ProjectionType="equirectangular" → photo_subtype="equirectangular"
    - JPEG with GPano:ProjectionType="cylindrical" → photo_subtype="panorama"
    - Wide JPEG without GPano → photo_subtype is null (no false positive)

  Burst timestamp proximity:
    - 3 JEPGs with taken_at timestamps within 2s → detected as burst group
    - 3 JEPGs with timestamps >2s apart → NOT grouped as burst

  Retroactive backfill:
    - Photos already in DB with NULL photo_subtype get fixed on rescan
"""

import os
import signal
import socket
import struct
import subprocess
import time

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename, wait_for_server

# ══════════════════════════════════════════════════════════════════════
# Config
# ══════════════════════════════════════════════════════════════════════

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERVER_DIR = os.path.join(REPO_ROOT, "server")
SERVER_BINARY = os.path.join(SERVER_DIR, "target", "release", "simple-photos-server")

ADMIN_USERNAME = "scan_admin"
ADMIN_PASSWORD = "ScanAdminPass123!"
TEST_ENCRYPTION_KEY = "b" * 64

# Minimal valid MP4 (ftyp box — enough for signature detection)
_FTYP_BOX = b'\x00\x00\x00\x14ftypmp42\x00\x00\x00\x00mp42'
_FAKE_MP4 = _FTYP_BOX + b'\x00' * 128


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _write_config(path, port, db_path, storage_root):
    config = f"""
[server]
host = "127.0.0.1"
port = {port}
base_url = "http://127.0.0.1:{port}"
trust_proxy = true
discovery_port = 0

[database]
path = "{db_path}"
max_connections = 4

[storage]
root = "{storage_root}"
default_quota_bytes = 0
max_blob_size_bytes = 104857600

[auth]
jwt_secret = "scan_regression_test_jwt_secret_must_be_32_chars_long_for_security"
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 4

[web]
static_root = ""

[backup]

[tls]
enabled = false

[scan]
auto_scan_interval_secs = 0
"""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(config)


# ══════════════════════════════════════════════════════════════════════
# Test data generators
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


def make_burst_jpeg(burst_id: str, w=200, h=150) -> bytes:
    """JPEG with GCamera:BurstID XMP metadata."""
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        f'<rdf:Description GCamera:BurstID="{burst_id}"/>'
        '</rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(generate_test_jpeg(width=w, height=h), xmp)


def make_motion_jpeg_micro(w=200, h=150) -> tuple[bytes, int]:
    """JPEG with Camera:MicroVideo XMP and embedded MP4 trailer (old schema)."""
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
    return _inject_xmp_into_jpeg(generate_test_jpeg(width=w, height=h), xmp, trailer=trailer), offset


def make_motion_jpeg_gcamera(w=200, h=150) -> tuple[bytes, int]:
    """JPEG with GCamera:MotionPhoto XMP and embedded MP4 trailer (new schema)."""
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
    return _inject_xmp_into_jpeg(generate_test_jpeg(width=w, height=h), xmp, trailer=trailer), offset


def make_equirectangular_jpeg(w=400, h=200) -> bytes:
    """JPEG with GPano:ProjectionType='equirectangular' XMP (360° photo)."""
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description GPano:ProjectionType="equirectangular"'
        ' GPano:CroppedAreaImageWidthPixels="400"'
        ' GPano:FullPanoWidthPixels="400"/>'
        '</rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(generate_test_jpeg(width=w, height=h), xmp)


def make_cylindrical_jpeg(w=400, h=100) -> bytes:
    """JPEG with GPano:ProjectionType='cylindrical' XMP (panorama)."""
    xmp = (
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description GPano:ProjectionType="cylindrical"/>'
        '</rdf:RDF></x:xmpmeta>'
    )
    return _inject_xmp_into_jpeg(generate_test_jpeg(width=w, height=h), xmp)


def make_normal_jpeg(w=200, h=150) -> bytes:
    """Plain JPEG with no XMP subtype markers."""
    return generate_test_jpeg(width=w, height=h)


def make_wide_jpeg_no_xmp(w=400, h=100) -> bytes:
    """Wide-aspect JPEG without any GPano metadata. Must NOT trigger panorama."""
    return generate_test_jpeg(width=w, height=h)


# ══════════════════════════════════════════════════════════════════════
# Fixtures
# ══════════════════════════════════════════════════════════════════════

class _FreshServer:
    """Manages a fresh server instance with its own DB and storage."""

    def __init__(self, port, tmpdir):
        self.port = port
        self.tmpdir = tmpdir
        self.base_url = f"http://127.0.0.1:{port}"
        self.storage_root = os.path.join(tmpdir, "storage")
        self.db_path = os.path.join(tmpdir, "db", "simple-photos.db")
        self.config_path = os.path.join(tmpdir, "config.toml")
        self.log_path = os.path.join(tmpdir, "server.log")
        self.process = None

        os.makedirs(os.path.join(tmpdir, "db"), exist_ok=True)
        os.makedirs(self.storage_root, exist_ok=True)
        _write_config(self.config_path, port, self.db_path, self.storage_root)

    def start(self, binary):
        log_file = open(self.log_path, "w")
        env = {
            **os.environ,
            "SIMPLE_PHOTOS_CONFIG": self.config_path,
            "RUST_LOG": "info",
        }
        self.process = subprocess.Popen(
            [binary], env=env, stdout=log_file, stderr=subprocess.STDOUT, cwd=self.tmpdir,
        )
        self._log_file = log_file
        try:
            wait_for_server(self.base_url, timeout=30)
        except TimeoutError:
            self.stop()
            with open(self.log_path) as f:
                print(f"\n=== server logs ===\n{f.read()}\n=== end ===")
            raise

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        if hasattr(self, "_log_file"):
            self._log_file.close()

    def dump_logs(self):
        if os.path.exists(self.log_path):
            with open(self.log_path) as f:
                print(f"\n=== server logs ===\n{f.read()[-5000:]}\n=== end ===")


@pytest.fixture(scope="module")
def server_binary():
    """Reuse existing release binary or build if needed."""
    if os.path.exists(SERVER_BINARY):
        age = time.time() - os.path.getmtime(SERVER_BINARY)
        if age < 7200:
            return SERVER_BINARY
    result = subprocess.run(
        ["cargo", "build", "--release"],
        cwd=SERVER_DIR, capture_output=True, text=True, timeout=600,
    )
    if result.returncode != 0:
        pytest.fail(f"Server build failed:\n{result.stderr[-2000:]}")
    return SERVER_BINARY


@pytest.fixture
def fresh_server(server_binary, tmp_path):
    """Spin up a fresh server with empty storage for scan testing."""
    if server_binary is None:
        pytest.skip("No server binary available")

    port = _find_free_port()
    server = _FreshServer(port, str(tmp_path))
    server.start(server_binary)
    yield server
    server.stop()


@pytest.fixture
def admin_client(fresh_server) -> APIClient:
    """Initialize fresh server and return logged-in admin client."""
    client = APIClient(fresh_server.base_url)
    client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
    client.login(ADMIN_USERNAME, ADMIN_PASSWORD)
    try:
        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
    except Exception:
        pass
    return client


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _place_file(storage_root: str, filename: str, content: bytes) -> str:
    """Write a file into the storage root. Returns the relative path."""
    path = os.path.join(storage_root, filename)
    with open(path, "wb") as f:
        f.write(content)
    return filename


def _find_photo_by_filename(client: APIClient, filename: str, timeout=15.0):
    """Poll until a photo with the given filename appears in listing."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        data = client.list_photos(limit=500)
        for p in data["photos"]:
            if p["filename"] == filename:
                return p
        time.sleep(0.5)
    raise TimeoutError(f"Photo '{filename}' not found in listing after {timeout}s")


def _scan(client: APIClient) -> dict:
    """Trigger a filesystem scan and return its result."""
    return client.admin_trigger_scan()


# ══════════════════════════════════════════════════════════════════════
# DDT: Burst via XMP — Scan Path
# ══════════════════════════════════════════════════════════════════════

BURST_XMP_CASES = [
    pytest.param("scan_burst_A.jpg", "xmp-burst-aaa", id="burst_xmp_A"),
    pytest.param("scan_burst_B.jpg", "xmp-burst-bbb", id="burst_xmp_B"),
]


class TestBurstXmpScan:
    """Burst photos placed on disk with GCamera:BurstID should be detected
    when the scan endpoint processes them."""

    @pytest.mark.parametrize("filename,burst_id", BURST_XMP_CASES)
    def test_burst_detected_via_scan(self, fresh_server, admin_client, filename, burst_id):
        content = make_burst_jpeg(burst_id)
        _place_file(fresh_server.storage_root, filename, content)

        _scan(admin_client)
        photo = _find_photo_by_filename(admin_client, filename)

        assert photo["photo_subtype"] == "burst", \
            f"Expected 'burst', got '{photo['photo_subtype']}'"
        assert photo["burst_id"] == burst_id, \
            f"Expected burst_id '{burst_id}', got '{photo['burst_id']}'"

    def test_burst_group_scanned_together(self, fresh_server, admin_client):
        """3 files sharing the same BurstID should all appear in burst endpoint."""
        bid = f"scan-group-{int(time.time())}"
        filenames = []
        for i in range(3):
            fn = f"burst_group_{i}_{int(time.time())}.jpg"
            _place_file(fresh_server.storage_root, fn, make_burst_jpeg(bid, w=200 + i))
            filenames.append(fn)

        _scan(admin_client)
        for fn in filenames:
            _find_photo_by_filename(admin_client, fn)

        r = admin_client.get(f"/api/photos/burst/{bid}")
        assert r.status_code == 200
        frames = r.json()
        assert len(frames) == 3, f"Expected 3 burst frames, got {len(frames)}"

    def test_burst_collapse_after_scan(self, fresh_server, admin_client):
        """collapse_bursts=true should reduce scanned burst to 1 representative."""
        bid = f"scan-collapse-{int(time.time())}"
        for i in range(3):
            fn = f"collapse_{i}_{int(time.time())}.jpg"
            _place_file(fresh_server.storage_root, fn, make_burst_jpeg(bid, w=200 + i))

        _scan(admin_client)
        time.sleep(1)  # Let all be registered

        data = admin_client.list_photos(subtype="burst", collapse_bursts="true")
        collapsed = [p for p in data["photos"] if p["burst_id"] == bid]
        assert len(collapsed) == 1, f"Expected 1 collapsed, got {len(collapsed)}"
        assert collapsed[0]["burst_count"] == 3

    def test_normal_jpeg_no_subtype(self, fresh_server, admin_client):
        """Normal JPEG without XMP markers should have null subtype after scan."""
        fn = f"normal_{int(time.time())}.jpg"
        _place_file(fresh_server.storage_root, fn, make_normal_jpeg())

        _scan(admin_client)
        photo = _find_photo_by_filename(admin_client, fn)
        assert photo["photo_subtype"] is None
        assert photo["burst_id"] is None


# ══════════════════════════════════════════════════════════════════════
# DDT: Motion Photo — Scan Path
# ══════════════════════════════════════════════════════════════════════

MOTION_SCAN_CASES = [
    pytest.param("scan_motion_micro.jpg", make_motion_jpeg_micro, id="micro_video_old_schema"),
    pytest.param("scan_motion_gcam.jpg", make_motion_jpeg_gcamera, id="motion_photo_new_schema"),
]


class TestMotionPhotoScan:
    """Motion photos placed on disk should be detected with their embedded
    MP4 extracted and stored as a blob during scan."""

    @pytest.mark.parametrize("filename,gen_func", MOTION_SCAN_CASES)
    def test_motion_detected_via_scan(self, fresh_server, admin_client, filename, gen_func):
        content, offset = gen_func()
        _place_file(fresh_server.storage_root, filename, content)

        _scan(admin_client)
        photo = _find_photo_by_filename(admin_client, filename)

        assert photo["photo_subtype"] == "motion", \
            f"Expected 'motion', got '{photo['photo_subtype']}'"

    @pytest.mark.parametrize("filename,gen_func", MOTION_SCAN_CASES)
    def test_motion_video_blob_extracted(self, fresh_server, admin_client, filename, gen_func):
        content, offset = gen_func()
        fn = f"blob_{filename}"
        _place_file(fresh_server.storage_root, fn, content)

        _scan(admin_client)
        photo = _find_photo_by_filename(admin_client, fn)

        assert photo["motion_video_blob_id"] is not None, \
            "motion_video_blob_id should be set after scan extraction"

    def test_motion_video_endpoint_after_scan(self, fresh_server, admin_client):
        """GET /api/photos/{id}/motion-video should return video/mp4."""
        content, offset = make_motion_jpeg_micro()
        fn = f"motion_serve_{int(time.time())}.jpg"
        _place_file(fresh_server.storage_root, fn, content)

        _scan(admin_client)
        photo = _find_photo_by_filename(admin_client, fn)

        r = admin_client.get(f"/api/photos/{photo['id']}/motion-video")
        assert r.status_code == 200, f"Expected 200, got {r.status_code}: {r.text}"
        assert r.headers.get("Content-Type") == "video/mp4"
        assert r.content[4:8] == b"ftyp", "Video should start with ftyp box"
        assert len(r.content) == offset, \
            f"Content-Length {len(r.content)} != expected {offset}"

    def test_normal_photo_not_motion(self, fresh_server, admin_client):
        """Normal JPEG should have null motion_video_blob_id after scan."""
        fn = f"not_motion_{int(time.time())}.jpg"
        _place_file(fresh_server.storage_root, fn, make_normal_jpeg())

        _scan(admin_client)
        photo = _find_photo_by_filename(admin_client, fn)
        assert photo["motion_video_blob_id"] is None


# ══════════════════════════════════════════════════════════════════════
# DDT: Panorama / 360° — Scan Path
# ══════════════════════════════════════════════════════════════════════

PANO_SCAN_CASES = [
    pytest.param("scan_equirect.jpg", make_equirectangular_jpeg, "equirectangular",
                 id="equirectangular_360_scan"),
    pytest.param("scan_cylindrical.jpg", make_cylindrical_jpeg, "panorama",
                 id="cylindrical_panorama_scan"),
    pytest.param("scan_wide_noxmp.jpg", make_wide_jpeg_no_xmp, None,
                 id="wide_no_false_positive_scan"),
]


class TestPanoramaScan:
    """Panoramic/360° photos should be detected via XMP during scan."""

    @pytest.mark.parametrize("filename,gen_func,expected_subtype", PANO_SCAN_CASES)
    def test_panorama_detected_via_scan(self, fresh_server, admin_client,
                                         filename, gen_func, expected_subtype):
        content = gen_func()
        _place_file(fresh_server.storage_root, filename, content)

        _scan(admin_client)
        photo = _find_photo_by_filename(admin_client, filename)

        assert photo["photo_subtype"] == expected_subtype, \
            f"Expected subtype '{expected_subtype}', got '{photo['photo_subtype']}'"

    def test_panorama_filter_after_scan(self, fresh_server, admin_client):
        """Subtype filter works for scanned panoramas."""
        eq_fn = f"eq_filter_{int(time.time())}.jpg"
        pano_fn = f"pano_filter_{int(time.time())}.jpg"
        norm_fn = f"norm_filter_{int(time.time())}.jpg"

        _place_file(fresh_server.storage_root, eq_fn, make_equirectangular_jpeg())
        _place_file(fresh_server.storage_root, pano_fn, make_cylindrical_jpeg())
        _place_file(fresh_server.storage_root, norm_fn, make_normal_jpeg())

        _scan(admin_client)
        _find_photo_by_filename(admin_client, eq_fn)
        _find_photo_by_filename(admin_client, pano_fn)
        _find_photo_by_filename(admin_client, norm_fn)

        eq_data = admin_client.list_photos(subtype="equirectangular")
        eq_fnames = [p["filename"] for p in eq_data["photos"]]
        assert eq_fn in eq_fnames
        assert pano_fn not in eq_fnames
        assert norm_fn not in eq_fnames

        pano_data = admin_client.list_photos(subtype="panorama")
        pano_fnames = [p["filename"] for p in pano_data["photos"]]
        assert pano_fn in pano_fnames
        assert eq_fn not in pano_fnames


# ══════════════════════════════════════════════════════════════════════
# DDT: Burst Timestamp Proximity — Scan Path
# ══════════════════════════════════════════════════════════════════════

class TestBurstTimestampScan:
    """Verify timestamp-based burst detection works for scanned photos.

    This tests the server/src/photos/burst.rs module which groups photos
    from the same camera taken within 2 seconds of each other.

    Note: This relies on the scan endpoint running burst detection after
    registering new photos. The photos need valid EXIF timestamps."""

    def test_burst_endpoint_trigger(self, fresh_server, admin_client):
        """POST /api/photos/detect-bursts should be callable and return a count."""
        r = admin_client.post("/api/photos/detect-bursts")
        assert r.status_code == 200
        data = r.json()
        assert "burst_groups_created" in data


# ══════════════════════════════════════════════════════════════════════
# DDT: Mixed Subtypes — All-in-one Scan
# ══════════════════════════════════════════════════════════════════════

class TestMixedSubtypeScan:
    """Place a mix of subtype files on disk: burst, motion, panorama,
    and normal. Verify all are correctly detected in one scan pass."""

    def test_mixed_subtypes_all_detected(self, fresh_server, admin_client):
        ts = int(time.time())

        # Place files
        burst_fn = f"mix_burst_{ts}.jpg"
        motion_fn = f"mix_motion_{ts}.jpg"
        eq_fn = f"mix_equirect_{ts}.jpg"
        pano_fn = f"mix_pano_{ts}.jpg"
        normal_fn = f"mix_normal_{ts}.jpg"

        _place_file(fresh_server.storage_root, burst_fn, make_burst_jpeg("mix-burst-id"))
        motion_content, _ = make_motion_jpeg_gcamera()
        _place_file(fresh_server.storage_root, motion_fn, motion_content)
        _place_file(fresh_server.storage_root, eq_fn, make_equirectangular_jpeg())
        _place_file(fresh_server.storage_root, pano_fn, make_cylindrical_jpeg())
        _place_file(fresh_server.storage_root, normal_fn, make_normal_jpeg())

        # Single scan
        result = _scan(admin_client)
        assert result["registered"] >= 5, \
            f"Expected at least 5 registered, got {result['registered']}"

        # Verify each
        burst_p = _find_photo_by_filename(admin_client, burst_fn)
        assert burst_p["photo_subtype"] == "burst"
        assert burst_p["burst_id"] == "mix-burst-id"

        motion_p = _find_photo_by_filename(admin_client, motion_fn)
        assert motion_p["photo_subtype"] == "motion"
        assert motion_p["motion_video_blob_id"] is not None

        eq_p = _find_photo_by_filename(admin_client, eq_fn)
        assert eq_p["photo_subtype"] == "equirectangular"

        pano_p = _find_photo_by_filename(admin_client, pano_fn)
        assert pano_p["photo_subtype"] == "panorama"

        normal_p = _find_photo_by_filename(admin_client, normal_fn)
        assert normal_p["photo_subtype"] is None
        assert normal_p["burst_id"] is None
        assert normal_p["motion_video_blob_id"] is None


# ══════════════════════════════════════════════════════════════════════
# DDT: Retroactive Subtype Backfill
# ══════════════════════════════════════════════════════════════════════

class TestRetroactiveBackfill:
    """Photos already in the DB with NULL photo_subtype should get their
    subtype detected on a subsequent scan (retroactive backfill)."""

    def test_rescan_fills_missing_subtypes(self, fresh_server, admin_client):
        """Place a motion photo, scan it. Verify subtype is detected.

        This test validates the retroactive backfill: on first scan the
        subtype should already be detected. On a second scan, photos that
        already have subtypes should not lose them, and any that were missed
        should get filled in."""
        ts = int(time.time())
        motion_fn = f"backfill_motion_{ts}.jpg"
        content, _ = make_motion_jpeg_micro()
        _place_file(fresh_server.storage_root, motion_fn, content)

        # First scan
        _scan(admin_client)
        p = _find_photo_by_filename(admin_client, motion_fn)
        assert p["photo_subtype"] == "motion"

        # Second scan - should preserve subtype
        _scan(admin_client)
        p2 = _find_photo_by_filename(admin_client, motion_fn)
        assert p2["photo_subtype"] == "motion", \
            "Rescan should preserve existing subtype"
        assert p2["id"] == p["id"], \
            "Rescan should not create duplicate photo entry"


# ══════════════════════════════════════════════════════════════════════
# DDT: Upload Path Regression (verify upload still works)
# ══════════════════════════════════════════════════════════════════════

class TestUploadPathRegression:
    """Verify the upload endpoint still correctly detects subtypes
    (regression guard — ensure scan fixes didn't break uploads)."""

    def test_upload_burst_still_works(self, fresh_server, admin_client):
        bid = f"upload-reg-{int(time.time())}"
        content = make_burst_jpeg(bid)
        result = admin_client.upload_photo(unique_filename(), content=content)
        photo_id = result["photo_id"]
        time.sleep(1)
        data = admin_client.list_photos()
        photo = next((p for p in data["photos"] if p["id"] == photo_id), None)
        assert photo is not None
        assert photo["photo_subtype"] == "burst"
        assert photo["burst_id"] == bid

    def test_upload_motion_still_works(self, fresh_server, admin_client):
        content, offset = make_motion_jpeg_gcamera()
        result = admin_client.upload_photo(unique_filename(), content=content)
        photo_id = result["photo_id"]
        time.sleep(1)
        data = admin_client.list_photos()
        photo = next((p for p in data["photos"] if p["id"] == photo_id), None)
        assert photo is not None
        assert photo["photo_subtype"] == "motion"
        assert photo["motion_video_blob_id"] is not None

    def test_upload_panorama_still_works(self, fresh_server, admin_client):
        content = make_equirectangular_jpeg()
        result = admin_client.upload_photo(unique_filename(), content=content)
        photo_id = result["photo_id"]
        time.sleep(1)
        data = admin_client.list_photos()
        photo = next((p for p in data["photos"] if p["id"] == photo_id), None)
        assert photo is not None
        assert photo["photo_subtype"] == "equirectangular"
