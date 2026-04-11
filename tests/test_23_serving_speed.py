"""
E2E Performance Regression Tests — Serving Speed Benchmarks

Measures end-to-end latency and throughput for photo/video/thumbnail/blob
serving. Designed to establish baselines and detect regressions.

Metrics captured per operation:
  - Time-to-first-byte (TTFB): time from request sent to first byte received
  - Total transfer time: time to download the complete response
  - Throughput: MB/s for the transfer
  - HTTP status & cache headers

Run:
    python3 -m pytest tests/test_23_serving_speed.py -v -s

The test prints a summary table at the end with all metrics.
"""

import hashlib
import io
import json
import os
import statistics
import struct
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from typing import List, Optional

import pytest
import requests

sys.path.insert(0, os.path.dirname(__file__))
from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_jpeg,
    random_password,
    random_username,
)


# ── Constants ────────────────────────────────────────────────────────

# Number of iterations per benchmark for statistical stability
ITERATIONS = 5
# File sizes to benchmark (bytes)
SMALL_PHOTO_SIZE = 50 * 1024          # 50 KB — typical compressed thumbnail
MEDIUM_PHOTO_SIZE = 2 * 1024 * 1024   # 2 MB — typical phone JPEG
LARGE_PHOTO_SIZE = 10 * 1024 * 1024   # 10 MB — high-res JPEG / RAW
VIDEO_SIZE = 20 * 1024 * 1024         # 20 MB — short video clip
BLOB_SMALL_SIZE = 100 * 1024          # 100 KB
BLOB_LARGE_SIZE = 10 * 1024 * 1024    # 10 MB


# ── Data classes ─────────────────────────────────────────────────────

@dataclass
class BenchmarkResult:
    """Single benchmark measurement."""
    name: str
    file_size_bytes: int
    ttfb_ms: float           # Time to first byte (ms)
    total_time_ms: float     # Total download time (ms)
    throughput_mbps: float   # MB/s
    status_code: int
    cache_hit: bool = False  # True if 304 Not Modified


@dataclass
class BenchmarkSuite:
    """Collection of benchmark results with summary statistics."""
    results: List[BenchmarkResult] = field(default_factory=list)

    def add(self, result: BenchmarkResult):
        self.results.append(result)

    def summary(self) -> str:
        """Format a human-readable summary table."""
        if not self.results:
            return "No results."

        # Group by name
        groups = {}
        for r in self.results:
            groups.setdefault(r.name, []).append(r)

        lines = []
        lines.append("")
        lines.append("=" * 110)
        lines.append(f"{'Benchmark':<40} {'Size':>8} {'TTFB(ms)':>10} {'Total(ms)':>10} "
                      f"{'Throughput':>12} {'Status':>8} {'Iters':>6}")
        lines.append("-" * 110)

        for name, group in groups.items():
            ttfbs = [r.ttfb_ms for r in group if not r.cache_hit]
            totals = [r.total_time_ms for r in group if not r.cache_hit]
            thrputs = [r.throughput_mbps for r in group if not r.cache_hit]
            size = group[0].file_size_bytes
            status = group[0].status_code

            if ttfbs:
                avg_ttfb = statistics.mean(ttfbs)
                avg_total = statistics.mean(totals)
                avg_thrput = statistics.mean(thrputs)
                size_str = _format_size(size)
                thrput_str = f"{avg_thrput:.1f} MB/s"
            else:
                avg_ttfb = 0
                avg_total = 0
                thrput_str = "N/A (304)"
                size_str = _format_size(size)

            lines.append(
                f"{name:<40} {size_str:>8} {avg_ttfb:>10.1f} {avg_total:>10.1f} "
                f"{thrput_str:>12} {status:>8} {len(group):>6}"
            )

            # Show min/max/stdev if enough samples
            if len(ttfbs) >= 3:
                std_ttfb = statistics.stdev(ttfbs)
                std_total = statistics.stdev(totals)
                lines.append(
                    f"  {'└ stdev':<38} {'':>8} {std_ttfb:>10.1f} {std_total:>10.1f}"
                )

        lines.append("=" * 110)
        return "\n".join(lines)

    def to_json(self) -> str:
        """Export results as JSON for tracking over time."""
        return json.dumps(
            [
                {
                    "name": r.name,
                    "file_size_bytes": r.file_size_bytes,
                    "ttfb_ms": round(r.ttfb_ms, 2),
                    "total_time_ms": round(r.total_time_ms, 2),
                    "throughput_mbps": round(r.throughput_mbps, 2),
                    "status_code": r.status_code,
                    "cache_hit": r.cache_hit,
                }
                for r in self.results
            ],
            indent=2,
        )


def _format_size(size_bytes: int) -> str:
    if size_bytes >= 1024 * 1024:
        return f"{size_bytes / (1024 * 1024):.0f} MB"
    elif size_bytes >= 1024:
        return f"{size_bytes / 1024:.0f} KB"
    return f"{size_bytes} B"


# ── Measurement helpers ──────────────────────────────────────────────

def _measure_download(session: requests.Session, url: str, headers: dict,
                      expected_size: int, name: str) -> BenchmarkResult:
    """Measure TTFB and total transfer time for a GET request using streaming."""
    start = time.perf_counter()
    resp = session.get(url, headers=headers, stream=True, timeout=60)
    ttfb = (time.perf_counter() - start) * 1000  # ms

    if resp.status_code == 304:
        return BenchmarkResult(
            name=name,
            file_size_bytes=expected_size,
            ttfb_ms=ttfb,
            total_time_ms=ttfb,
            throughput_mbps=0,
            status_code=304,
            cache_hit=True,
        )

    # Stream the full body
    total_bytes = 0
    for chunk in resp.iter_content(chunk_size=64 * 1024):
        total_bytes += len(chunk)

    total_time = (time.perf_counter() - start) * 1000  # ms
    transfer_secs = total_time / 1000.0

    if transfer_secs > 0 and total_bytes > 0:
        throughput = (total_bytes / (1024 * 1024)) / transfer_secs
    else:
        throughput = 0

    return BenchmarkResult(
        name=name,
        file_size_bytes=total_bytes,
        ttfb_ms=ttfb,
        total_time_ms=total_time,
        throughput_mbps=throughput,
        status_code=resp.status_code,
    )


def _measure_range_download(session: requests.Session, url: str, headers: dict,
                            total_size: int, range_start: int, range_end: int,
                            name: str) -> BenchmarkResult:
    """Measure a Range request (simulating video seek)."""
    range_headers = {**headers, "Range": f"bytes={range_start}-{range_end}"}
    expected = range_end - range_start + 1

    start = time.perf_counter()
    resp = session.get(url, headers=range_headers, stream=True, timeout=60)
    ttfb = (time.perf_counter() - start) * 1000

    total_bytes = 0
    for chunk in resp.iter_content(chunk_size=64 * 1024):
        total_bytes += len(chunk)

    total_time = (time.perf_counter() - start) * 1000
    transfer_secs = total_time / 1000.0

    if transfer_secs > 0 and total_bytes > 0:
        throughput = (total_bytes / (1024 * 1024)) / transfer_secs
    else:
        throughput = 0

    return BenchmarkResult(
        name=name,
        file_size_bytes=total_bytes,
        ttfb_ms=ttfb,
        total_time_ms=total_time,
        throughput_mbps=throughput,
        status_code=resp.status_code,
    )


def _measure_upload(session: requests.Session, url: str, headers: dict,
                    data: bytes, name: str) -> BenchmarkResult:
    """Measure upload speed."""
    start = time.perf_counter()
    resp = session.post(url, data=data, headers=headers, timeout=120)
    total_time = (time.perf_counter() - start) * 1000  # ms
    ttfb = total_time  # For uploads, TTFB ~= total time (server responds after processing)

    transfer_secs = total_time / 1000.0
    if transfer_secs > 0:
        throughput = (len(data) / (1024 * 1024)) / transfer_secs
    else:
        throughput = 0

    return BenchmarkResult(
        name=name,
        file_size_bytes=len(data),
        ttfb_ms=ttfb,
        total_time_ms=total_time,
        throughput_mbps=throughput,
        status_code=resp.status_code,
    )


# ── Test data generators for large files ─────────────────────────────

def _generate_large_jpeg(size_bytes: int) -> bytes:
    """Generate a JPEG that's approximately the target size.

    Creates a large-dimension image and adjusts quality to hit the target.
    """
    from PIL import Image
    import random

    # Estimate dimensions to get close to target size
    # At ~85 quality, JPEG is roughly 1-3 bytes/pixel for noisy content
    pixels_needed = max(size_bytes // 2, 1024)
    side = max(int(pixels_needed ** 0.5), 64)
    side = min(side, 8192)  # Cap at 8K

    # Create noisy image (compresses less → larger file)
    img = Image.new("RGB", (side, side))
    pixel_data = os.urandom(side * side * 3)
    img = Image.frombytes("RGB", (side, side), pixel_data)

    # Binary search for quality that gets us close to target size
    lo, hi = 10, 100
    best_buf = None
    for _ in range(8):
        q = (lo + hi) // 2
        buf = io.BytesIO()
        img.save(buf, format="JPEG", quality=q)
        current_size = buf.tell()
        best_buf = buf

        if current_size < size_bytes * 0.9:
            lo = q + 1
        elif current_size > size_bytes * 1.1:
            hi = q - 1
        else:
            break

    result = best_buf.getvalue()
    # If still too small, pad with JPEG comment markers
    if len(result) < size_bytes:
        padding_needed = size_bytes - len(result)
        # Insert JPEG comment (0xFFFE) before final marker
        comment_data = os.urandom(min(padding_needed, 65533))
        comment = struct.pack(">HH", 0xFFFE, len(comment_data) + 2) + comment_data
        # Insert before EOI (last 2 bytes = FFD9)
        result = result[:-2] + comment + result[-2:]

    return result


def _generate_fake_video(size_bytes: int) -> bytes:
    """Generate random bytes that simulate a video file for blob benchmarks.

    We use raw bytes for blob uploads since blobs are opaque encrypted data
    anyway. For /api/photos/upload we need actual media files.
    """
    return os.urandom(size_bytes)


# ── Fixtures ─────────────────────────────────────────────────────────

@pytest.fixture(scope="module")
def perf_client(primary_server, primary_admin) -> APIClient:
    """Dedicated user for performance tests."""
    username = random_username("perfuser_")
    primary_admin.admin_create_user(username, "PerfUserPass123!", role="user")
    client = APIClient(primary_server.base_url)
    client.login(username, "PerfUserPass123!")
    return client


@pytest.fixture(scope="module")
def benchmark_suite() -> BenchmarkSuite:
    """Shared benchmark results collector for the module."""
    return BenchmarkSuite()


@pytest.fixture(scope="module")
def uploaded_photos(perf_client: APIClient) -> dict:
    """Upload test photos of various sizes and return their IDs.

    Returns dict: {label: {photo_id, size_bytes, content_hash}}
    """
    photos = {}

    # Small photo (50 KB)
    small_content = _generate_large_jpeg(SMALL_PHOTO_SIZE)
    result = perf_client.upload_photo(filename="perf_small.jpg", content=small_content)
    photos["small"] = {
        "photo_id": result["photo_id"],
        "size_bytes": len(small_content),
        "content": small_content,
    }

    # Medium photo (2 MB)
    medium_content = _generate_large_jpeg(MEDIUM_PHOTO_SIZE)
    result = perf_client.upload_photo(filename="perf_medium.jpg", content=medium_content)
    photos["medium"] = {
        "photo_id": result["photo_id"],
        "size_bytes": len(medium_content),
        "content": medium_content,
    }

    # Large photo (10 MB)
    large_content = _generate_large_jpeg(LARGE_PHOTO_SIZE)
    result = perf_client.upload_photo(filename="perf_large.jpg", content=large_content)
    photos["large"] = {
        "photo_id": result["photo_id"],
        "size_bytes": len(large_content),
        "content": large_content,
    }

    # Wait for thumbnails to be generated
    time.sleep(2)

    return photos


@pytest.fixture(scope="module")
def uploaded_blobs(perf_client: APIClient) -> dict:
    """Upload test blobs of various sizes and return their IDs.

    Returns dict: {label: {blob_id, size_bytes}}
    """
    blobs = {}

    # Small blob (100 KB)
    small_data = generate_random_bytes(BLOB_SMALL_SIZE)
    result = perf_client.upload_blob("photo", small_data)
    blobs["small"] = {"blob_id": result["blob_id"], "size_bytes": len(small_data)}

    # Large blob (10 MB)
    large_data = generate_random_bytes(BLOB_LARGE_SIZE)
    result = perf_client.upload_blob("photo", large_data)
    blobs["large"] = {"blob_id": result["blob_id"], "size_bytes": len(large_data)}

    # Video-sized blob (20 MB)
    video_data = generate_random_bytes(VIDEO_SIZE)
    result = perf_client.upload_blob("video", video_data)
    blobs["video"] = {"blob_id": result["blob_id"], "size_bytes": len(video_data)}

    return blobs


# ── Photo Serving Tests ──────────────────────────────────────────────

class TestPhotoServing:
    """Benchmark photo file serving via /api/photos/{id}/file."""

    def test_serve_small_photo(self, perf_client, uploaded_photos, benchmark_suite):
        """50 KB photo — TTFB and throughput."""
        photo = uploaded_photos["small"]
        url = f"{perf_client.base_url}/api/photos/{photo['photo_id']}/file"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                photo["size_bytes"], "photo/file/small (50KB)",
            )
            assert result.status_code == 200
            benchmark_suite.add(result)

    def test_serve_medium_photo(self, perf_client, uploaded_photos, benchmark_suite):
        """2 MB photo — typical smartphone JPEG."""
        photo = uploaded_photos["medium"]
        url = f"{perf_client.base_url}/api/photos/{photo['photo_id']}/file"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                photo["size_bytes"], "photo/file/medium (2MB)",
            )
            assert result.status_code == 200
            benchmark_suite.add(result)

    def test_serve_large_photo(self, perf_client, uploaded_photos, benchmark_suite):
        """10 MB photo — high-resolution JPEG."""
        photo = uploaded_photos["large"]
        url = f"{perf_client.base_url}/api/photos/{photo['photo_id']}/file"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                photo["size_bytes"], "photo/file/large (10MB)",
            )
            assert result.status_code == 200
            benchmark_suite.add(result)

    def test_serve_photo_etag_304(self, perf_client, uploaded_photos, benchmark_suite):
        """ETag cache validation — should return 304 instantly."""
        photo = uploaded_photos["medium"]
        url = f"{perf_client.base_url}/api/photos/{photo['photo_id']}/file"

        # First request to get ETag
        resp = perf_client.get(f"/api/photos/{photo['photo_id']}/file")
        etag = resp.headers.get("ETag")
        assert etag, "Server should return ETag header"

        headers = {**perf_client._auth_headers(), "If-None-Match": etag}

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                photo["size_bytes"], "photo/file/etag-304 (2MB)",
            )
            assert result.status_code == 304
            benchmark_suite.add(result)


# ── Thumbnail Serving Tests ──────────────────────────────────────────

class TestThumbnailServing:
    """Benchmark thumbnail serving via /api/photos/{id}/thumb."""

    def test_serve_thumbnail(self, perf_client, uploaded_photos, benchmark_suite):
        """Pre-generated thumbnail — should be very fast."""
        photo = uploaded_photos["medium"]
        url = f"{perf_client.base_url}/api/photos/{photo['photo_id']}/thumb"
        headers = perf_client._auth_headers()

        # First request to verify it's not pending
        resp = perf_client.get(f"/api/photos/{photo['photo_id']}/thumb")
        if resp.status_code == 202:
            # Wait for thumbnail generation
            for _ in range(30):
                time.sleep(1)
                resp = perf_client.get(f"/api/photos/{photo['photo_id']}/thumb")
                if resp.status_code == 200:
                    break
        assert resp.status_code == 200, f"Thumbnail not ready: {resp.status_code}"
        thumb_size = len(resp.content)

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                thumb_size, "photo/thumb (pre-generated)",
            )
            assert result.status_code == 200
            benchmark_suite.add(result)

    def test_thumbnail_burst(self, perf_client, uploaded_photos, benchmark_suite):
        """Rapid sequential thumbnail requests — simulates grid scrolling."""
        # Request thumbnails for all 3 uploaded photos in quick succession
        photo_ids = [p["photo_id"] for p in uploaded_photos.values()]
        headers = perf_client._auth_headers()

        for round_num in range(ITERATIONS):
            round_start = time.perf_counter()
            count = 0
            for pid in photo_ids:
                url = f"{perf_client.base_url}/api/photos/{pid}/thumb"
                resp = perf_client.session.get(url, headers=headers, timeout=30)
                if resp.status_code == 200:
                    count += 1
            round_time = (time.perf_counter() - round_start) * 1000

            benchmark_suite.add(BenchmarkResult(
                name=f"photo/thumb/burst ({len(photo_ids)} thumbs)",
                file_size_bytes=0,
                ttfb_ms=round_time / len(photo_ids),
                total_time_ms=round_time,
                throughput_mbps=0,
                status_code=200,
            ))


# ── Blob Serving Tests ──────────────────────────────────────────────

class TestBlobServing:
    """Benchmark encrypted blob downloads via /api/blobs/{id}."""

    def test_serve_small_blob(self, perf_client, uploaded_blobs, benchmark_suite):
        """100 KB blob download."""
        blob = uploaded_blobs["small"]
        url = f"{perf_client.base_url}/api/blobs/{blob['blob_id']}"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                blob["size_bytes"], "blob/download/small (100KB)",
            )
            assert result.status_code == 200
            benchmark_suite.add(result)

    def test_serve_large_blob(self, perf_client, uploaded_blobs, benchmark_suite):
        """10 MB blob download."""
        blob = uploaded_blobs["large"]
        url = f"{perf_client.base_url}/api/blobs/{blob['blob_id']}"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                blob["size_bytes"], "blob/download/large (10MB)",
            )
            assert result.status_code == 200
            benchmark_suite.add(result)

    def test_serve_video_blob(self, perf_client, uploaded_blobs, benchmark_suite):
        """20 MB video-sized blob download."""
        blob = uploaded_blobs["video"]
        url = f"{perf_client.base_url}/api/blobs/{blob['blob_id']}"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                blob["size_bytes"], "blob/download/video (20MB)",
            )
            assert result.status_code == 200
            benchmark_suite.add(result)

    def test_blob_etag_304(self, perf_client, uploaded_blobs, benchmark_suite):
        """Blob ETag cache — immutable, should return 304 very fast."""
        blob = uploaded_blobs["large"]
        url = f"{perf_client.base_url}/api/blobs/{blob['blob_id']}"

        resp = perf_client.download_blob(blob["blob_id"])
        etag = resp.headers.get("ETag")
        assert etag, "Blob should return ETag"

        headers = {**perf_client._auth_headers(), "If-None-Match": etag}

        for i in range(ITERATIONS):
            result = _measure_download(
                perf_client.session, url, headers,
                blob["size_bytes"], "blob/download/etag-304 (10MB)",
            )
            assert result.status_code == 304
            benchmark_suite.add(result)


# ── Range Request Tests ──────────────────────────────────────────────

class TestRangeRequests:
    """Benchmark HTTP Range requests — critical for video seeking."""

    def test_range_start_of_video(self, perf_client, uploaded_blobs, benchmark_suite):
        """First 1 MB of 20 MB video — simulates initial video load."""
        blob = uploaded_blobs["video"]
        url = f"{perf_client.base_url}/api/blobs/{blob['blob_id']}"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_range_download(
                perf_client.session, url, headers,
                blob["size_bytes"], 0, 1024 * 1024 - 1,
                "blob/range/first-1MB (of 20MB)",
            )
            assert result.status_code == 206
            benchmark_suite.add(result)

    def test_range_mid_video_seek(self, perf_client, uploaded_blobs, benchmark_suite):
        """1 MB chunk from middle of 20 MB — simulates seeking to midpoint."""
        blob = uploaded_blobs["video"]
        url = f"{perf_client.base_url}/api/blobs/{blob['blob_id']}"
        headers = perf_client._auth_headers()
        mid = blob["size_bytes"] // 2

        for i in range(ITERATIONS):
            result = _measure_range_download(
                perf_client.session, url, headers,
                blob["size_bytes"], mid, mid + 1024 * 1024 - 1,
                "blob/range/mid-seek-1MB (of 20MB)",
            )
            assert result.status_code == 206
            benchmark_suite.add(result)

    def test_range_small_chunk(self, perf_client, uploaded_blobs, benchmark_suite):
        """64 KB chunk — simulates progressive loading / small seek."""
        blob = uploaded_blobs["video"]
        url = f"{perf_client.base_url}/api/blobs/{blob['blob_id']}"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_range_download(
                perf_client.session, url, headers,
                blob["size_bytes"], 0, 64 * 1024 - 1,
                "blob/range/64KB-chunk (of 20MB)",
            )
            assert result.status_code == 206
            benchmark_suite.add(result)

    def test_photo_range_request(self, perf_client, uploaded_photos, benchmark_suite):
        """Range request on a 10 MB photo — first 256 KB."""
        photo = uploaded_photos["large"]
        url = f"{perf_client.base_url}/api/photos/{photo['photo_id']}/file"
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            result = _measure_range_download(
                perf_client.session, url, headers,
                photo["size_bytes"], 0, 256 * 1024 - 1,
                "photo/range/first-256KB (of 10MB)",
            )
            assert result.status_code == 206
            benchmark_suite.add(result)


# ── Upload Speed Tests ───────────────────────────────────────────────

class TestUploadSpeed:
    """Benchmark upload throughput for photos and blobs."""

    def test_upload_small_photo(self, perf_client, benchmark_suite):
        """50 KB photo upload via /api/photos/upload."""
        content = _generate_large_jpeg(SMALL_PHOTO_SIZE)
        url = f"{perf_client.base_url}/api/photos/upload"

        for i in range(ITERATIONS):
            headers = {
                **perf_client._auth_headers(),
                "X-Filename": f"perf_upload_small_{i}_{time.time_ns()}.jpg",
                "X-Mime-Type": "image/jpeg",
                "Content-Type": "application/octet-stream",
            }
            result = _measure_upload(
                perf_client.session, url, headers, content,
                "photo/upload/small (50KB)",
            )
            assert result.status_code in (200, 201)
            benchmark_suite.add(result)

    def test_upload_large_blob(self, perf_client, benchmark_suite):
        """10 MB blob upload via /api/blobs."""
        content = generate_random_bytes(BLOB_LARGE_SIZE)
        url = f"{perf_client.base_url}/api/blobs"

        for i in range(ITERATIONS):
            client_hash = hashlib.sha256(content + str(i).encode()).hexdigest()
            # Vary content to avoid dedup
            varied_content = content + os.urandom(16)
            actual_hash = hashlib.sha256(varied_content).hexdigest()
            headers = {
                **perf_client._auth_headers(),
                "x-blob-type": "photo",
                "x-client-hash": actual_hash,
                "Content-Type": "application/octet-stream",
            }
            result = _measure_upload(
                perf_client.session, url, headers, varied_content,
                "blob/upload/large (10MB)",
            )
            assert result.status_code in (200, 201)
            benchmark_suite.add(result)


# ── Concurrent-ish Burst Tests ───────────────────────────────────────

class TestBurstPatterns:
    """Simulate realistic access patterns — grid loads, gallery scrolls."""

    def test_list_then_thumbs(self, perf_client, uploaded_photos, benchmark_suite):
        """Simulate opening the app: list photos, then fetch all thumbnails.

        This measures the critical path the user experience depends on.
        """
        headers = perf_client._auth_headers()

        for round_num in range(ITERATIONS):
            # Step 1: List photos
            list_start = time.perf_counter()
            resp = perf_client.session.get(
                f"{perf_client.base_url}/api/photos",
                headers=headers, params={"limit": 50}, timeout=30,
            )
            list_time = (time.perf_counter() - list_start) * 1000
            assert resp.status_code == 200
            photos = resp.json().get("photos", [])

            benchmark_suite.add(BenchmarkResult(
                name="pattern/list-photos",
                file_size_bytes=len(resp.content),
                ttfb_ms=list_time,
                total_time_ms=list_time,
                throughput_mbps=0,
                status_code=200,
            ))

            # Step 2: Fetch thumbnails for listed photos
            if photos:
                thumb_start = time.perf_counter()
                for photo in photos[:10]:  # First 10 thumbnails
                    pid = photo.get("id", "")
                    if pid:
                        perf_client.session.get(
                            f"{perf_client.base_url}/api/photos/{pid}/thumb",
                            headers=headers, timeout=30,
                        )
                thumb_total = (time.perf_counter() - thumb_start) * 1000

                benchmark_suite.add(BenchmarkResult(
                    name=f"pattern/fetch-{min(10, len(photos))}-thumbs",
                    file_size_bytes=0,
                    ttfb_ms=thumb_total / min(10, len(photos)),
                    total_time_ms=thumb_total,
                    throughput_mbps=0,
                    status_code=200,
                ))

    def test_photo_list_pagination(self, perf_client, benchmark_suite):
        """Measure list API latency with pagination."""
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            start = time.perf_counter()
            resp = perf_client.session.get(
                f"{perf_client.base_url}/api/photos",
                headers=headers, params={"limit": 100}, timeout=30,
            )
            total = (time.perf_counter() - start) * 1000
            assert resp.status_code == 200

            benchmark_suite.add(BenchmarkResult(
                name="api/list-photos (limit=100)",
                file_size_bytes=len(resp.content),
                ttfb_ms=total,
                total_time_ms=total,
                throughput_mbps=0,
                status_code=200,
            ))

    def test_encrypted_sync_list(self, perf_client, benchmark_suite):
        """Measure encrypted-sync endpoint latency."""
        headers = perf_client._auth_headers()

        for i in range(ITERATIONS):
            start = time.perf_counter()
            resp = perf_client.session.get(
                f"{perf_client.base_url}/api/photos/encrypted-sync",
                headers=headers, params={"limit": 100}, timeout=30,
            )
            total = (time.perf_counter() - start) * 1000
            assert resp.status_code == 200

            benchmark_suite.add(BenchmarkResult(
                name="api/encrypted-sync (limit=100)",
                file_size_bytes=len(resp.content),
                ttfb_ms=total,
                total_time_ms=total,
                throughput_mbps=0,
                status_code=200,
            ))


# ── Summary Report ───────────────────────────────────────────────────

class TestSummary:
    """Print final benchmark summary. Must run last in this module."""

    def test_zz_print_summary(self, benchmark_suite):
        """Print the complete benchmark results table."""
        summary = benchmark_suite.summary()
        print(summary)

        # Also write JSON results to a file for historical tracking
        results_dir = os.path.join(os.path.dirname(__file__), "..", "benchmark_results")
        os.makedirs(results_dir, exist_ok=True)
        timestamp = time.strftime("%Y%m%d_%H%M%S")
        results_path = os.path.join(results_dir, f"serving_speed_{timestamp}.json")
        with open(results_path, "w") as f:
            f.write(benchmark_suite.to_json())
        print(f"\nResults saved to: {results_path}")

        # Basic sanity — we collected results
        assert len(benchmark_suite.results) > 0, "No benchmark results collected"
