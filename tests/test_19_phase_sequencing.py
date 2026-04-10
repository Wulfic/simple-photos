"""
Test 19: Phase Sequencing — verify that the conversion ingest engine waits
for Phase 1 encryption to finish before starting, and that the banner
status endpoints reflect the correct state at each stage.

Lifecycle under test:
  Phase 1: Scan discovers native files → server-side encryption runs
  Phase 2: Ingest engine discovers non-native files → converts them
  Phase 3: Newly converted files are encrypted

Verification strategy:
  - Server logs are parsed to confirm encryption completes BEFORE conversion
    starts (log-line ordering is authoritative, unlike API polling which is
    subject to non-atomic races).
  - Final-state checks verify all photos end up encrypted and conversions
    are registered correctly.
  - API-level checks confirm conversion-status endpoint reports correctly.
"""

import os
import re
import struct
import time

import pytest
from conftest import (
    ADMIN_PASSWORD,
    ADMIN_USERNAME,
    TEST_ENCRYPTION_KEY,
    ServerInstance,
    _find_free_port,
)
from helpers import (
    APIClient,
    _ffmpeg_available,
    generate_test_jpeg,
    unique_filename,
)


pytestmark = pytest.mark.skipif(
    not _ffmpeg_available(),
    reason="ffmpeg not installed — conversion/ingest tests require ffmpeg",
)


def _read_server_log(server) -> str:
    """Read the full server log file."""
    if hasattr(server, "log_path") and os.path.exists(server.log_path):
        with open(server.log_path) as f:
            return f.read()
    return ""


def _unique_tiff(seed: int) -> bytes:
    """Generate a TIFF with unique pixel data to avoid content-hash dedup."""
    width, height = 2, 2
    r = (seed * 37) % 256
    g = (seed * 73) % 256
    b = (seed * 113) % 256
    pixel_data = bytes([r, g, b] * (width * height))
    strip_offset = 8 + 2 + (12 * 10) + 4 + 6
    bps_offset = 8 + 2 + (12 * 10) + 4

    def tag(tag_id, typ, count, value):
        return struct.pack("<HHII", tag_id, typ, count, value)

    ifd = struct.pack("<H", 10)
    ifd += tag(0x0100, 3, 1, width)       # ImageWidth
    ifd += tag(0x0101, 3, 1, height)      # ImageLength
    ifd += tag(0x0102, 3, 3, bps_offset)  # BitsPerSample
    ifd += tag(0x0103, 3, 1, 1)           # Compression (None)
    ifd += tag(0x0106, 3, 1, 2)           # PhotometricInterpretation (RGB)
    ifd += tag(0x0111, 4, 1, strip_offset)  # StripOffsets
    ifd += tag(0x0115, 3, 1, 3)           # SamplesPerPixel
    ifd += tag(0x0116, 4, 1, height)      # RowsPerStrip
    ifd += tag(0x0117, 4, 1, len(pixel_data))  # StripByteCounts
    ifd += tag(0x011C, 3, 1, 1)           # PlanarConfiguration
    ifd += struct.pack("<I", 0)            # Next IFD offset (none)
    bps_data = struct.pack("<HHH", 8, 8, 8)

    header = b"II" + struct.pack("<HI", 42, 8)
    return header + ifd + bps_data + pixel_data


def _find_log_line_index(log_lines: list, pattern: str) -> int:
    """Return the index of the first line matching the regex, or -1."""
    for i, line in enumerate(log_lines):
        if re.search(pattern, line):
            return i
    return -1


class TestPhaseSequencing:
    """Conversion ingest waits for encryption before starting."""

    @pytest.fixture
    def fresh_server(self, server_binary, session_tmpdir):
        """Spin up a completely fresh server with no data."""
        if server_binary is None:
            pytest.skip("External servers: can't spin up fresh instance")

        port = _find_free_port()
        tmpdir = os.path.join(session_tmpdir, f"phase_seq_{int(time.time())}")
        server = ServerInstance("phase-seq", port, tmpdir)
        server.start(server_binary)
        yield server
        server.stop()

    # ── Test 1: Log-order sequencing ─────────────────────────────────

    def test_encryption_before_conversion_log_order(self, fresh_server):
        """Verify via server logs that encryption completes before
        conversion starts.

        The server logs are authoritative for sequencing because they're
        written by the same async task that runs encryption → conversion.
        """
        client = APIClient(fresh_server.base_url)
        client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        client.login(ADMIN_USERNAME, ADMIN_PASSWORD)

        # Place native + non-native files
        for i in range(5):
            name = unique_filename("jpg")
            path = os.path.join(fresh_server.storage_root, name)
            with open(path, "wb") as f:
                f.write(generate_test_jpeg(width=4 + i, height=4 + i))
        for i in range(3):
            name = unique_filename("tiff")
            path = os.path.join(fresh_server.storage_root, name)
            with open(path, "wb") as f:
                f.write(_unique_tiff(100 + i))

        # Store encryption key — this also triggers a scan + encryption
        # internally.  We rely on that single code path.
        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)

        # Trigger scan to start the conversion ingest engine for TIFFs.
        client.admin_trigger_scan()

        # Wait for the full pipeline to complete
        client.wait_for_conversion(timeout=90)
        # Extra settle time for encryption of converted files
        time.sleep(3)

        # ── Parse server logs for sequencing ──────────────────────────
        log = _read_server_log(fresh_server)
        log_lines = log.splitlines()

        # Encryption migration should log completion
        encrypt_done_idx = _find_log_line_index(
            log_lines,
            r"(All photos encrypted|migration complete|SERVER_MIG.*finished)",
        )
        # Ingest engine should log its start
        ingest_start_idx = _find_log_line_index(
            log_lines, r"\[INGEST\]"
        )

        assert encrypt_done_idx >= 0, (
            "Server log does not contain encryption completion message. "
            f"Log tail:\n{''.join(log_lines[-30:])}"
        )
        assert ingest_start_idx >= 0, (
            "Server log does not contain ingest engine messages. "
            f"Log tail:\n{''.join(log_lines[-30:])}"
        )

        # THE KEY ASSERTION: encryption completion must appear BEFORE
        # any ingest engine activity in the log.
        assert encrypt_done_idx < ingest_start_idx, (
            f"SEQUENCING VIOLATION: Ingest started (line {ingest_start_idx}) "
            f"before encryption finished (line {encrypt_done_idx}).\n"
            f"Encrypt line: {log_lines[encrypt_done_idx].strip()}\n"
            f"Ingest line:  {log_lines[ingest_start_idx].strip()}"
        )

    # ── Test 2: Final-state encrypted check ──────────────────────────

    def test_all_photos_encrypted_after_pipeline(self, fresh_server):
        """After scan → encrypt → convert → encrypt, every photo has an
        encrypted_blob_id (no stragglers)."""
        client = APIClient(fresh_server.base_url)
        client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        client.login(ADMIN_USERNAME, ADMIN_PASSWORD)

        # Place files
        for i in range(3):
            path = os.path.join(
                fresh_server.storage_root, unique_filename("jpg")
            )
            with open(path, "wb") as f:
                f.write(generate_test_jpeg(width=8 + i, height=8 + i))
        for i in range(2):
            path = os.path.join(
                fresh_server.storage_root, unique_filename("tiff")
            )
            with open(path, "wb") as f:
                f.write(_unique_tiff(200 + i))

        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
        client.admin_trigger_scan()

        # Wait for the full pipeline: encrypt → convert → encrypt-converted.
        # We poll for the expected photo count rather than relying solely on
        # wait_for_conversion, because conversion may briefly appear inactive
        # between phases.
        deadline = time.time() + 90
        while time.time() < deadline:
            sync = client.encrypted_sync()
            photos = sync.get("photos", [])
            unencrypted = [
                p for p in photos if p.get("encrypted_blob_id") is None
            ]
            # At least 4 (3 native + ≥1 converted), all encrypted
            if len(photos) >= 4 and len(unencrypted) == 0:
                break
            time.sleep(1)

        final_sync = client.encrypted_sync()
        final_photos = final_sync.get("photos", [])
        unencrypted_final = [
            p for p in final_photos if p.get("encrypted_blob_id") is None
        ]

        assert len(final_photos) >= 4, (
            f"Expected at least 4 photos (3 native + converted), "
            f"got {len(final_photos)}"
        )
        assert len(unencrypted_final) == 0, (
            f"{len(unencrypted_final)} photos still unencrypted after full "
            f"pipeline: "
            f"{[(p['filename'], p.get('encrypted_blob_id')) for p in unencrypted_final]}"
        )

    # ── Test 3: Conversion-status endpoint ───────────────────────────

    def test_conversion_status_reflects_activity(self, fresh_server):
        """conversion-status endpoint reports total/done counters after
        convertible files are processed."""
        client = APIClient(fresh_server.base_url)
        client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        client.login(ADMIN_USERNAME, ADMIN_PASSWORD)

        # Place ONLY non-native files (skip encryption complexity)
        for i in range(3):
            name = unique_filename("tiff")
            path = os.path.join(fresh_server.storage_root, name)
            with open(path, "wb") as f:
                f.write(_unique_tiff(300 + i))

        client.admin_trigger_scan()

        # Wait for conversion
        client.wait_for_conversion(timeout=60)

        # After conversion, endpoint should report done == total
        conv = client.admin_conversion_status()
        assert conv["total"] >= 1, (
            f"Expected conversion total >= 1, got {conv['total']}"
        )
        assert conv["done"] >= conv["total"], (
            f"Conversion not complete: done={conv['done']}, "
            f"total={conv['total']}"
        )

        # Photos should be registered
        photos = client.list_photos()["photos"]
        jpeg_from_tiff = [
            p for p in photos if p.get("mime_type") == "image/jpeg"
        ]
        assert len(jpeg_from_tiff) >= 1, (
            f"Expected at least 1 converted JPEG, "
            f"found {len(jpeg_from_tiff)}"
        )

    # ── Test 4: No conversion without convertible files ──────────────

    def test_conversion_not_active_without_convertible_files(
        self, fresh_server
    ):
        """When only native files exist, conversion should never become
        active."""
        client = APIClient(fresh_server.base_url)
        client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        client.login(ADMIN_USERNAME, ADMIN_PASSWORD)

        # Place only native JPEGs (no TIFFs to convert)
        for i in range(3):
            path = os.path.join(
                fresh_server.storage_root, unique_filename("jpg")
            )
            with open(path, "wb") as f:
                f.write(generate_test_jpeg(width=16 + i, height=16 + i))

        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
        client.admin_trigger_scan()

        # Poll for a while — conversion should never become active
        conversion_ever_active = False
        deadline = time.time() + 20
        while time.time() < deadline:
            try:
                conv = client.admin_conversion_status()
                if conv.get("active", False):
                    conversion_ever_active = True
                    break
                if conv.get("total", 0) > 0:
                    conversion_ever_active = True
                    break
            except Exception:
                pass
            time.sleep(0.5)

        # Wait for encryption to finish
        deadline = time.time() + 60
        while time.time() < deadline:
            sync = client.encrypted_sync()
            photos = sync.get("photos", [])
            unenc = [
                p for p in photos if p.get("encrypted_blob_id") is None
            ]
            if len(photos) > 0 and len(unenc) == 0:
                break
            time.sleep(1)

        assert not conversion_ever_active, (
            "Conversion became active even though there were no convertible "
            "files"
        )

    # ── Test 5: Encryption banner data during Phase 1 ────────────────

    def test_encryption_banner_data_during_phase1(self, fresh_server):
        """The encrypted-sync endpoint should report photos with
        encrypted_blob_id=NULL during Phase 1 (what the EncryptionBanner
        uses to determine pending work)."""
        client = APIClient(fresh_server.base_url)
        client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        client.login(ADMIN_USERNAME, ADMIN_PASSWORD)

        # Place several native files
        for i in range(5):
            path = os.path.join(
                fresh_server.storage_root, unique_filename("jpg")
            )
            with open(path, "wb") as f:
                f.write(generate_test_jpeg(width=32 + i, height=32 + i))

        # Trigger scan WITHOUT encryption key — photos should appear
        # as unencrypted indefinitely.
        client.admin_trigger_scan()
        time.sleep(1)

        sync = client.encrypted_sync()
        photos = sync.get("photos", [])
        unenc = [p for p in photos if p.get("encrypted_blob_id") is None]

        assert len(photos) >= 5, (
            f"Expected at least 5 photos from scan, got {len(photos)}"
        )
        assert len(unenc) == len(photos), (
            f"Without encryption key, all photos should have "
            f"encrypted_blob_id=NULL. "
            f"Found {len(unenc)} unencrypted out of {len(photos)}"
        )

        # Now store key — encryption should kick in
        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
        client.admin_trigger_scan()

        # Verify they eventually get encrypted
        deadline = time.time() + 60
        while time.time() < deadline:
            sync = client.encrypted_sync()
            photos = sync.get("photos", [])
            unenc = [
                p for p in photos if p.get("encrypted_blob_id") is None
            ]
            if len(photos) >= 5 and len(unenc) == 0:
                break
            time.sleep(1)

        final_sync = client.encrypted_sync()
        final_unenc = [
            p for p in final_sync["photos"]
            if p.get("encrypted_blob_id") is None
        ]
        assert len(final_unenc) == 0, (
            f"Encryption did not complete: {len(final_unenc)} photos "
            f"still pending"
        )
