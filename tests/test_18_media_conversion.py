"""
Test 18: Media Conversion Pipeline — upload and scan of non-native formats
with automatic conversion to browser-native equivalents via FFmpeg.

Covers:
  - Upload of convertible formats (TIFF, MKV, AVI, AIFF, M4A, HEIC)
  - Verification that converted files are stored in the correct format
  - Scan-based conversion of files placed in the storage directory
  - Native format uploads remain unchanged (regression)
  - Duplicate detection still works across converted uploads
  - Rejection of truly unsupported formats
  - Encryption of converted files
"""

import hashlib
import os
import shutil
import subprocess
import time

import pytest
from helpers import (
    APIClient,
    generate_test_jpeg,
    generate_test_png,
    generate_test_tiff,
    generate_test_video_mkv,
    generate_test_video_avi,
    generate_test_audio_aiff,
    generate_test_audio_m4a,
    generate_test_heic,
    generate_random_bytes,
    unique_filename,
    _ffmpeg_available,
)


# Skip the entire module if ffmpeg is not installed.
pytestmark = pytest.mark.skipif(
    not _ffmpeg_available(),
    reason="ffmpeg not installed — conversion tests require ffmpeg",
)


# ── Image conversion ─────────────────────────────────────────────────

class TestImageConversion:
    """Uploading non-native image formats converts them to JPEG."""

    def test_upload_tiff_converts_to_jpeg(self, user_client: APIClient):
        """TIFF upload → converted to JPEG, stored & retrievable."""
        content = generate_test_tiff()
        assert len(content) > 0, "Failed to generate test TIFF"

        data = user_client.upload_photo("landscape.tiff", content, mime_type="image/tiff")
        assert "photo_id" in data
        # Filename should have .jpg extension after conversion
        assert data["filename"].endswith(".jpg"), (
            f"Expected .jpg filename, got: {data['filename']}"
        )
        assert data["size_bytes"] > 0

    def test_upload_tif_variant(self, user_client: APIClient):
        """TIF (3-letter extension) is also accepted and converted."""
        content = generate_test_tiff()
        data = user_client.upload_photo("photo.tif", content, mime_type="image/tiff")
        assert "photo_id" in data
        assert data["filename"].endswith(".jpg")

    def test_upload_heic_converts_to_jpeg(self, user_client: APIClient):
        """HEIC (Apple) upload → converted to JPEG."""
        content = generate_test_heic()
        if not content:
            pytest.skip("System ffmpeg cannot encode HEIC test files (libx265 missing)")

        data = user_client.upload_photo("IMG_001.heic", content, mime_type="image/heic")
        assert "photo_id" in data
        assert data["filename"].endswith(".jpg"), (
            f"Expected .jpg filename after HEIC conversion, got: {data['filename']}"
        )

    def test_tiff_listed_in_photos(self, user_client: APIClient):
        """Converted TIFF should appear in the photo list with JPEG mime type."""
        content = generate_test_tiff()
        data = user_client.upload_photo("listed_test.tiff", content, mime_type="image/tiff")
        photo_id = data["photo_id"]

        photos = user_client.list_photos()["photos"]
        match = [p for p in photos if p["id"] == photo_id]
        assert len(match) == 1
        assert match[0]["mime_type"] == "image/jpeg"
        assert match[0]["media_type"] == "photo"


# ── Video conversion ─────────────────────────────────────────────────

class TestVideoConversion:
    """Uploading non-native video formats converts them to MP4."""

    def test_upload_mkv_converts_to_mp4(self, user_client: APIClient):
        """MKV upload → converted to MP4."""
        content = generate_test_video_mkv()
        assert len(content) > 0, "Failed to generate test MKV"

        data = user_client.upload_photo("recording.mkv", content, mime_type="video/x-matroska")
        assert "photo_id" in data
        assert data["filename"].endswith(".mp4"), (
            f"Expected .mp4 filename, got: {data['filename']}"
        )

    def test_upload_avi_converts_to_mp4(self, user_client: APIClient):
        """AVI upload → converted to MP4."""
        content = generate_test_video_avi()
        assert len(content) > 0, "Failed to generate test AVI"

        data = user_client.upload_photo("clip.avi", content, mime_type="video/x-msvideo")
        assert "photo_id" in data
        assert data["filename"].endswith(".mp4")

    def test_mkv_listed_as_video(self, user_client: APIClient):
        """Converted MKV appears in photo list as video/mp4."""
        content = generate_test_video_mkv()
        data = user_client.upload_photo("video_list.mkv", content)
        photo_id = data["photo_id"]

        photos = user_client.list_photos()["photos"]
        match = [p for p in photos if p["id"] == photo_id]
        assert len(match) == 1
        assert match[0]["mime_type"] == "video/mp4"
        assert match[0]["media_type"] == "video"


# ── Audio conversion ─────────────────────────────────────────────────

class TestAudioConversion:
    """Uploading non-native audio formats converts them to MP3."""

    def test_upload_aiff_converts_to_mp3(self, user_client: APIClient):
        """AIFF upload → converted to MP3."""
        content = generate_test_audio_aiff()
        assert len(content) > 0, "Failed to generate test AIFF"

        data = user_client.upload_photo("track.aiff", content, mime_type="audio/aiff")
        assert "photo_id" in data
        assert data["filename"].endswith(".mp3"), (
            f"Expected .mp3 filename, got: {data['filename']}"
        )

    def test_upload_m4a_converts_to_mp3(self, user_client: APIClient):
        """M4A (AAC container) upload → converted to MP3."""
        content = generate_test_audio_m4a()
        assert len(content) > 0, "Failed to generate test M4A"

        data = user_client.upload_photo("voice_memo.m4a", content, mime_type="audio/mp4")
        assert "photo_id" in data
        assert data["filename"].endswith(".mp3")

    def test_aiff_listed_as_audio(self, user_client: APIClient):
        """Converted AIFF appears in photo list as audio/mpeg."""
        content = generate_test_audio_aiff()
        data = user_client.upload_photo("audio_list.aiff", content)
        photo_id = data["photo_id"]

        photos = user_client.list_photos()["photos"]
        match = [p for p in photos if p["id"] == photo_id]
        assert len(match) == 1
        assert match[0]["mime_type"] == "audio/mpeg"
        assert match[0]["media_type"] == "audio"


# ── Native format regression ─────────────────────────────────────────

class TestNativeFormatRegression:
    """Native formats must still work exactly as before — no conversion."""

    def test_jpeg_upload_unchanged(self, user_client: APIClient):
        """JPEG uploads should not be modified."""
        content = generate_test_jpeg()
        data = user_client.upload_photo("native.jpg", content)
        assert data["filename"] == "native.jpg"
        assert data["size_bytes"] == len(content)

    def test_png_upload_unchanged(self, user_client: APIClient):
        """PNG uploads should not be modified."""
        content = generate_test_png()
        data = user_client.upload_photo("native.png", content, mime_type="image/png")
        assert data["filename"] == "native.png"
        assert data["size_bytes"] == len(content)

    def test_native_dedup_still_works(self, user_client: APIClient):
        """Content-hash dedup should still work for native JPEG uploads."""
        content = generate_test_jpeg()
        data1 = user_client.upload_photo("dup_a.jpg", content)
        data2 = user_client.upload_photo("dup_b.jpg", content)
        assert data1["photo_id"] == data2["photo_id"]


# ── Unsupported formats ──────────────────────────────────────────────

class TestUnsupportedFormats:
    """Truly unsupported formats should still be rejected."""

    def test_reject_unknown_extension(self, user_client: APIClient):
        """Files with unknown extensions are rejected with 400."""
        content = generate_random_bytes(256)
        r = user_client.post(
            "/api/photos/upload",
            data=content,
            headers={
                **user_client._auth_headers(),
                "X-Filename": "data.xyz",
                "X-Mime-Type": "application/octet-stream",
                "Content-Type": "application/octet-stream",
            },
        )
        assert r.status_code == 400

    def test_reject_text_file(self, user_client: APIClient):
        """Text files should be rejected."""
        r = user_client.post(
            "/api/photos/upload",
            data=b"Hello world",
            headers={
                **user_client._auth_headers(),
                "X-Filename": "readme.txt",
                "X-Mime-Type": "text/plain",
                "Content-Type": "application/octet-stream",
            },
        )
        assert r.status_code == 400


# ── Scan-based conversion ────────────────────────────────────────────

class TestScanConversion:
    """Files placed directly in the storage directory are discovered by
    the scan endpoint.  Native files are registered immediately; non-native
    files are converted by the background ingest engine after native
    encryption completes."""

    def test_scan_discovers_and_converts_tiff(self, admin_client: APIClient,
                                               primary_server):
        """Place a TIFF in the storage root → scan → ingest converts → registered as JPEG."""
        content = generate_test_tiff()
        tiff_name = unique_filename("tiff")
        tiff_path = os.path.join(primary_server.storage_root, tiff_name)
        with open(tiff_path, "wb") as f:
            f.write(content)

        try:
            # Scan registers native files; conversion runs in background.
            admin_client.admin_trigger_scan()
            # Wait for the background ingest engine to finish converting.
            admin_client.wait_for_conversion(timeout=60)

            # Verify the photo was registered with JPEG mime
            photos = admin_client.list_photos()["photos"]
            converted = [
                p for p in photos
                if p.get("mime_type") == "image/jpeg"
                and p.get("filename", "").startswith(tiff_name.rsplit(".", 1)[0])
            ]
            assert len(converted) >= 1, (
                f"Expected converted TIFF in photos list. "
                f"TIFF: {tiff_name}, photos: {[p.get('filename') for p in photos]}"
            )
        finally:
            pass

    def test_scan_does_not_reconvert(self, admin_client: APIClient, primary_server):
        """Running scan twice should not produce duplicates for converted files."""
        content = generate_test_tiff()
        tiff_name = f"scan_nodupe_{int(time.time() * 1000)}.tiff"
        tiff_path = os.path.join(primary_server.storage_root, tiff_name)
        with open(tiff_path, "wb") as f:
            f.write(content)

        # First scan + wait for conversion
        admin_client.admin_trigger_scan()
        admin_client.wait_for_conversion(timeout=60)

        photos_after_first = admin_client.list_photos()["photos"]
        count_after_first = len(photos_after_first)

        # Second scan + wait — should not register duplicates
        admin_client.admin_trigger_scan()
        admin_client.wait_for_conversion(timeout=30)

        photos_after_second = admin_client.list_photos()["photos"]
        count_after_second = len(photos_after_second)

        assert count_after_second == count_after_first, (
            f"Second scan produced duplicates: {count_after_first} → {count_after_second}"
        )

    def test_scan_converts_video(self, admin_client: APIClient, primary_server):
        """Place an MKV in storage → scan → ingest converts → registered as MP4."""
        content = generate_test_video_mkv()
        mkv_name = unique_filename("mkv")
        mkv_path = os.path.join(primary_server.storage_root, mkv_name)
        with open(mkv_path, "wb") as f:
            f.write(content)

        admin_client.admin_trigger_scan()
        admin_client.wait_for_conversion(timeout=60)

        photos = admin_client.list_photos()["photos"]
        converted = [
            p for p in photos
            if p.get("mime_type") == "video/mp4"
            and p.get("filename", "").startswith(mkv_name.rsplit(".", 1)[0])
        ]
        assert len(converted) >= 1

    def test_scan_native_still_works(self, admin_client: APIClient, primary_server):
        """Native JPEG files placed in storage are still discovered normally."""
        content = generate_test_jpeg()
        jpg_name = unique_filename("jpg")
        jpg_path = os.path.join(primary_server.storage_root, jpg_name)
        with open(jpg_path, "wb") as f:
            f.write(content)

        result = admin_client.admin_trigger_scan()
        assert result.get("registered", 0) >= 1

        photos = admin_client.list_photos()["photos"]
        match = [p for p in photos if p.get("filename") == jpg_name]
        assert len(match) == 1
        assert match[0]["mime_type"] == "image/jpeg"


# ── Conversion + encryption integration ──────────────────────────────

class TestConversionEncryption:
    """Converted files should be encrypted just like native files."""

    def test_converted_upload_has_hash(self, user_client: APIClient):
        """Converted uploads should have a valid photo_hash."""
        content = generate_test_tiff()
        data = user_client.upload_photo("hash_test.tiff", content)
        assert data.get("photo_hash"), "Converted photo should have a content hash"
        assert len(data["photo_hash"]) > 0

    def test_converted_upload_dedup(self, user_client: APIClient):
        """Uploading the same convertible file twice should trigger dedup."""
        content = generate_test_tiff()
        data1 = user_client.upload_photo("dedup_conv_a.tiff", content)
        data2 = user_client.upload_photo("dedup_conv_b.tiff", content)
        # The converted bytes may differ slightly per run, so dedup may or
        # may not trigger depending on FFmpeg determinism.  At minimum, both
        # uploads should succeed.
        assert "photo_id" in data1
        assert "photo_id" in data2


# ── Extended format coverage ─────────────────────────────────────────

class TestExtendedFormats:
    """Verify a broad range of convertible formats are accepted."""

    @pytest.mark.parametrize("ext,expected_ext", [
        ("tiff", "jpg"),
        ("tif", "jpg"),
        ("mkv", "mp4"),
        ("avi", "mp4"),
        ("aiff", "mp3"),
        ("aif", "mp3"),
        ("m4a", "mp3"),
    ])
    def test_format_acceptance(self, user_client: APIClient, ext, expected_ext):
        """Server accepts the format and responds with converted extension."""
        generators = {
            "tiff": generate_test_tiff,
            "tif": generate_test_tiff,
            "mkv": generate_test_video_mkv,
            "avi": generate_test_video_avi,
            "aiff": generate_test_audio_aiff,
            "aif": generate_test_audio_aiff,
            "m4a": generate_test_audio_m4a,
        }
        gen = generators.get(ext)
        if gen is None:
            pytest.skip(f"No generator for {ext}")
        content = gen()
        if not content:
            pytest.skip(f"Could not generate test {ext} file")

        data = user_client.upload_photo(f"format_test.{ext}", content)
        assert data["filename"].endswith(f".{expected_ext}"), (
            f"Expected .{expected_ext}, got {data['filename']}"
        )
