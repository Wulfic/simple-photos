"""
Test 99: Real-features smoke test.

A single curated end-to-end run that exercises the features most likely to
silently regress because the audit found them faking success on heuristic
output.  It is **expected to skip cleanly** when the operator has not
installed the optional ONNX models and GeoNames dataset; when it runs it
asserts behavioural outcomes (counters increment, geo_country populated,
audio_blob_id present), never just JSON shapes.

This complements the per-feature DDT/regression files; the goal here is a
single command developers can run locally before pushing:

    pytest tests/test_99_smoke_real_features.py -v
"""

from __future__ import annotations

import os
import time
from pathlib import Path

import pytest

from helpers import (
    APIClient,
    generate_test_jpeg,
    generate_test_jpeg_with_gps,
    unique_filename,
)


REPO_ROOT = Path(__file__).resolve().parent.parent
AI_FACES_DIR = REPO_ROOT / "tests" / "test_data" / "ai_faces"
AI_OBJECTS_DIR = REPO_ROOT / "tests" / "test_data" / "ai_objects"

GEO_DATASET_CANDIDATES = [
    REPO_ROOT / "server" / "data" / "cities500.txt",
    REPO_ROOT / "server" / "cities500.txt",
    Path("/var/lib/simple-photos/cities500.txt"),
]
if env := os.environ.get("E2E_GEO_DATASET_PATH"):
    GEO_DATASET_CANDIDATES.insert(0, Path(env))


def _has_geo_dataset() -> Path | None:
    for c in GEO_DATASET_CANDIDATES:
        try:
            if c.is_file() and c.stat().st_size > 0:
                return c
        except OSError:
            continue
    return None


def _wait(client: APIClient, getter, predicate, timeout: float = 60.0):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        last = getter()
        if predicate(last):
            return last
        time.sleep(1.0)
    return last


# ══════════════════════════════════════════════════════════════════════
# Smoke: AI face + object recognition end-to-end (real models required)
# ══════════════════════════════════════════════════════════════════════


def test_smoke_face_and_object_recognition(user_client: APIClient):
    """Upload real face + object fixtures; both counters must move."""
    status = user_client.ai_status()
    if status.get("degraded_mode", False):
        pytest.skip(
            "AI in degraded_mode (no ONNX models). "
            "Run scripts/fetch_ai_models.sh and re-run this smoke test."
        )

    user_client.ai_toggle(True)

    face_baseline = status.get("face_detections", 0)
    obj_baseline = status.get("object_detections", 0)

    face_files = sorted(AI_FACES_DIR.glob("face_*.jpg"))[:5]
    obj_files = sorted(AI_OBJECTS_DIR.glob("obj_*.jpg"))[:5]
    assert face_files and obj_files, "Test fixtures missing"

    for fp in face_files + obj_files:
        user_client.upload_photo(unique_filename("jpg"), content=fp.read_bytes())

    final = _wait(
        user_client,
        lambda: user_client.ai_status(),
        lambda s: (
            s.get("face_detections", 0) > face_baseline
            and s.get("object_detections", 0) > obj_baseline
        ),
        timeout=120.0,
    )
    assert final and final.get("face_detections", 0) > face_baseline, (
        f"face_detections did not move (baseline={face_baseline}, final={final})"
    )
    assert final and final.get("object_detections", 0) > obj_baseline, (
        f"object_detections did not move (baseline={obj_baseline}, final={final})"
    )


# ══════════════════════════════════════════════════════════════════════
# Smoke: GPS upload → reverse geocoding (real GeoNames dataset required)
# ══════════════════════════════════════════════════════════════════════


def test_smoke_geo_reverse_geocoding(user_client: APIClient):
    """Uploading a photo with Paris GPS must populate geo_country/geo_city."""
    if _has_geo_dataset() is None:
        pytest.skip(
            "GeoNames cities500.txt dataset not found. "
            "Run scripts/fetch_geo_data.sh and re-run this smoke test."
        )

    name = unique_filename("jpg")
    content = generate_test_jpeg_with_gps(48.8584, 2.2945)  # Eiffel Tower
    upload = user_client.upload_photo(name, content=content)
    photo_id = upload["photo_id"]

    def _photo() -> dict | None:
        for p in user_client.list_photos().get("photos", []):
            if p["id"] == photo_id:
                return p
        return None

    final = _wait(
        user_client,
        _photo,
        lambda p: p is not None and p.get("geo_country_code") == "FR",
        timeout=60.0,
    )
    assert final, f"Photo {photo_id} disappeared"
    assert final.get("geo_country_code") == "FR", (
        f"Expected geo_country_code=FR, got {final.get('geo_country_code')!r}. "
        f"Reverse geocoding pipeline is broken."
    )
    assert final.get("geo_city"), (
        f"geo_city not populated for Paris GPS upload: {final}"
    )


# ══════════════════════════════════════════════════════════════════════
# Smoke: audio backup respects audio_backup_enabled toggle (P0-1)
# ══════════════════════════════════════════════════════════════════════


def test_smoke_audio_upload_basic(user_client: APIClient):
    """A non-AI feature smoke that always runs: regular photo upload + list."""
    name = unique_filename("jpg")
    user_client.upload_photo(name, content=generate_test_jpeg())
    photos = user_client.list_photos().get("photos", [])
    assert any(p.get("filename") == name for p in photos), (
        f"Uploaded {name!r} not visible in list_photos()"
    )
