"""
E2E DDT: Thumbnail generation across image formats.

Verifies:
- Every supported format yields a non-empty thumbnail JPEG.
- The thumbnail decodes to a valid image (proves it's not random/empty bytes).
- The thumbnail is NOT a flat colour (i.e. not the grey [50,50,50]
  placeholder fallback).  Pixel variance > a small threshold.

Real-world samples (~/Desktop/Sample_files/image/) are used when present;
otherwise we synthesise the format with PIL.  A missing real-world sample
is skipped (not failed) so CI / contributors without the sample folder
still get coverage from the synthesised cases.
"""

from __future__ import annotations

import io
import os
import time
from pathlib import Path

import pytest

from helpers import (
    APIClient,
    generate_test_jpeg,
    generate_test_bmp,
    generate_test_png,
    unique_filename,
)


# ── helpers ────────────────────────────────────────────────────────────────

SAMPLE_ROOT = Path(os.environ.get(
    "SAMPLE_FILES_ROOT", str(Path.home() / "Desktop" / "Sample_files")
))


def _wait_thumb(client: APIClient, photo_id: str, timeout: float = 15.0) -> bytes:
    """Poll the /thumb endpoint until a non-pending response arrives."""
    deadline = time.time() + timeout
    last_status = None
    while time.time() < deadline:
        r = client.get_photo_thumb(photo_id)
        last_status = r.status_code
        if r.status_code == 200 and r.content:
            return r.content
        time.sleep(0.3)
    raise TimeoutError(
        f"Thumbnail for {photo_id} not ready (last status {last_status})"
    )


def _assert_real_thumbnail(content: bytes, *, min_variance: float = 5.0):
    """Assert the bytes decode as a non-flat image."""
    from PIL import Image, ImageStat

    assert content, "Empty thumbnail content"
    img = Image.open(io.BytesIO(content))
    img.load()
    assert img.size[0] > 0 and img.size[1] > 0
    # Variance of luminance — a flat grey/black placeholder is ~0.
    stat = ImageStat.Stat(img.convert("L"))
    variance = stat.var[0] if stat.var else 0.0
    assert variance >= min_variance, (
        f"Thumbnail looks like a flat placeholder (variance={variance:.2f} "
        f"< threshold={min_variance}); image size={img.size}"
    )


def _gen_synth_tiff() -> bytes:
    from PIL import Image
    buf = io.BytesIO()
    img = Image.new("RGB", (256, 256))
    # Gradient so the variance check passes.
    for y in range(256):
        for x in range(256):
            img.putpixel((x, y), (x, y, (x + y) % 256))
    img.save(buf, format="TIFF")
    return buf.getvalue()


def _gen_synth_webp() -> bytes:
    from PIL import Image
    buf = io.BytesIO()
    img = Image.new("RGB", (256, 256))
    for y in range(256):
        for x in range(256):
            img.putpixel((x, y), (x, (x + y) % 256, y))
    img.save(buf, format="WEBP")
    return buf.getvalue()


def _gen_synth_gradient_jpeg() -> bytes:
    from PIL import Image
    buf = io.BytesIO()
    img = Image.new("RGB", (256, 256))
    for y in range(256):
        for x in range(256):
            img.putpixel((x, y), (x, y, 128))
    img.save(buf, format="JPEG", quality=90)
    return buf.getvalue()


def _gen_synth_gradient_png() -> bytes:
    from PIL import Image
    buf = io.BytesIO()
    img = Image.new("RGB", (256, 256))
    for y in range(256):
        for x in range(256):
            img.putpixel((x, y), (x, y, 200 - x // 2))
    img.save(buf, format="PNG")
    return buf.getvalue()


def _gen_synth_gradient_bmp() -> bytes:
    from PIL import Image
    buf = io.BytesIO()
    img = Image.new("RGB", (128, 128))
    for y in range(128):
        for x in range(128):
            img.putpixel((x, y), (x * 2, y * 2, 100))
    img.save(buf, format="BMP")
    return buf.getvalue()


def _gen_synth_gradient_avif() -> bytes:
    from PIL import Image
    buf = io.BytesIO()
    img = Image.new("RGB", (256, 256))
    for y in range(256):
        for x in range(256):
            img.putpixel((x, y), ((x + y) % 256, y, x))
    try:
        img.save(buf, format="AVIF")
    except Exception:
        pytest.skip("PIL AVIF plugin not available on this system")
    return buf.getvalue()


def _real(name: str) -> bytes | None:
    path = SAMPLE_ROOT / "image" / name
    if not path.exists():
        return None
    return path.read_bytes()


# ── DDT cases ──────────────────────────────────────────────────────────────
# (filename, mime, content_loader)

THUMBNAIL_FORMAT_CASES = [
    pytest.param(
        "synthetic.jpg", "image/jpeg", _gen_synth_gradient_jpeg,
        id="jpeg_synthetic",
    ),
    pytest.param(
        "synthetic.png", "image/png", _gen_synth_gradient_png,
        id="png_synthetic",
    ),
    pytest.param(
        "synthetic.bmp", "image/bmp", _gen_synth_gradient_bmp,
        id="bmp_synthetic",
    ),
    pytest.param(
        "synthetic.webp", "image/webp", _gen_synth_webp,
        id="webp_synthetic",
    ),
    pytest.param(
        "synthetic.tiff", "image/tiff", _gen_synth_tiff,
        id="tiff_synthetic_now_supported",
    ),
]


@pytest.mark.parametrize("filename,mime,loader", THUMBNAIL_FORMAT_CASES)
def test_thumbnail_format_renders_real_image(
    user_client: APIClient, filename: str, mime: str, loader
):
    content = loader()
    if not content:
        pytest.skip(f"{filename}: no content available")
    photo = user_client.upload_photo(
        filename=unique_filename(ext=Path(filename).suffix.lstrip('.')),
        content=content,
        mime_type=mime,
    )
    thumb = _wait_thumb(user_client, photo["photo_id"])
    _assert_real_thumbnail(thumb)


# ── Real-sample cases (skipped if file missing) ────────────────────────────

REAL_SAMPLE_CASES = [
    pytest.param(
        "Sample BMP FIle for Testing.bmp", "image/bmp",
        id="real_bmp_large",
    ),
    pytest.param(
        "Tiff-Image-File-Download.tiff", "image/tiff",
        id="real_tiff_large",
    ),
    pytest.param(
        "Cat_November_2010-1a.jpg.webp", "image/webp",
        id="real_webp",
    ),
    pytest.param(
        "american-landmarks-statue-of-liberty_1.avif", "image/avif",
        id="real_avif",
    ),
    pytest.param(
        "Large-Sample-png-Image-download-for-Testing.png", "image/png",
        id="real_large_png",
    ),
]


@pytest.mark.parametrize("filename,mime", REAL_SAMPLE_CASES)
def test_thumbnail_real_sample_renders(
    user_client: APIClient, filename: str, mime: str
):
    content = _real(filename)
    if content is None:
        pytest.skip(f"Real sample {filename} not present")
    photo = user_client.upload_photo(
        filename=unique_filename(ext=Path(filename).suffix.lstrip('.')),
        content=content,
        mime_type=mime,
    )
    thumb = _wait_thumb(user_client, photo["photo_id"])
    _assert_real_thumbnail(thumb)
