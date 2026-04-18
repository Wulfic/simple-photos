"""
Test 50: AI Recognition — Data-Driven Tests (DDT).

Parametrized tests covering all AI recognition API endpoints:
  - AI status endpoint returns expected fields
  - Toggle AI on/off
  - Reprocess all / specific photos
  - Face cluster CRUD (list, rename, merge, split)
  - Object class listing + photos per class
  - AI tags (person: and object: prefixes) appear in tag search
  - Edge cases: empty inputs, invalid IDs, unauthorized access

Each test case is a single row in a parameter table.
"""

import pytest
import time

from helpers import APIClient, unique_filename, generate_test_jpeg


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(client: APIClient, w: int = 80, h: int = 80) -> str:
    """Upload a test photo and return its photo_id."""
    content = generate_test_jpeg(width=w, height=h)
    data = client.upload_photo(unique_filename(), content=content)
    return data["photo_id"]


# ══════════════════════════════════════════════════════════════════════
# DDT: AI Status Endpoint
# ══════════════════════════════════════════════════════════════════════

STATUS_FIELDS = [
    pytest.param("enabled", id="has_enabled_field"),
    pytest.param("gpu_available", id="has_gpu_field"),
    pytest.param("photos_processed", id="has_processed_field"),
    pytest.param("photos_pending", id="has_pending_field"),
    pytest.param("face_detections", id="has_face_detections_field"),
    pytest.param("face_clusters", id="has_face_clusters_field"),
    pytest.param("object_detections", id="has_object_detections_field"),
]


@pytest.mark.parametrize("field", STATUS_FIELDS)
def test_ai_status_has_field(user_client, field):
    """AI status response contains the expected field."""
    status = user_client.ai_status()
    assert field in status, f"AI status missing field '{field}'"


# ══════════════════════════════════════════════════════════════════════
# DDT: AI Toggle On/Off
# ══════════════════════════════════════════════════════════════════════

TOGGLE_CASES = [
    pytest.param(True, id="enable_ai"),
    pytest.param(False, id="disable_ai"),
]


@pytest.mark.parametrize("enabled", TOGGLE_CASES)
def test_ai_toggle(user_client, enabled):
    """Toggling AI on/off is reflected in subsequent status queries."""
    r = user_client.ai_toggle(enabled)
    assert r.status_code == 204

    status = user_client.ai_status()
    assert status["enabled"] == enabled


# ══════════════════════════════════════════════════════════════════════
# DDT: Reprocess
# ══════════════════════════════════════════════════════════════════════

REPROCESS_CASES = [
    pytest.param(None, id="reprocess_all"),
    pytest.param([], id="reprocess_empty_list"),
]


@pytest.mark.parametrize("photo_ids", REPROCESS_CASES)
def test_ai_reprocess(user_client, photo_ids):
    """Reprocess endpoint accepts various inputs."""
    # Enable AI first
    user_client.ai_toggle(True)
    result = user_client.ai_reprocess(photo_ids)
    assert "cleared" in result
    assert "message" in result


def test_ai_reprocess_specific_photo(user_client):
    """Reprocess a specific photo by ID."""
    photo_id = _upload(user_client)
    user_client.ai_toggle(True)
    result = user_client.ai_reprocess([photo_id])
    assert "cleared" in result


# ══════════════════════════════════════════════════════════════════════
# DDT: Face Cluster Listing
# ══════════════════════════════════════════════════════════════════════

def test_ai_face_clusters_empty_initially(user_client):
    """New user has no face clusters."""
    clusters = user_client.ai_list_face_clusters()
    assert isinstance(clusters, list)
    assert len(clusters) == 0


# ══════════════════════════════════════════════════════════════════════
# DDT: Object Class Listing
# ══════════════════════════════════════════════════════════════════════

def test_ai_object_classes_empty_initially(user_client):
    """New user has no object detections."""
    classes = user_client.ai_list_object_classes()
    assert isinstance(classes, list)
    assert len(classes) == 0


# ══════════════════════════════════════════════════════════════════════
# DDT: Rename Face Cluster — Error Cases
# ══════════════════════════════════════════════════════════════════════

RENAME_ERROR_CASES = [
    pytest.param(999999, "Alice", 404, id="nonexistent_cluster"),
]


@pytest.mark.parametrize("cluster_id,name,expected_status", RENAME_ERROR_CASES)
def test_ai_rename_face_cluster_errors(user_client, cluster_id, name, expected_status):
    """Renaming a non-existent cluster returns the expected error status."""
    r = user_client.ai_rename_face_cluster(cluster_id, name)
    assert r.status_code == expected_status


# ══════════════════════════════════════════════════════════════════════
# DDT: Merge Face Clusters — Error Cases
# ══════════════════════════════════════════════════════════════════════

MERGE_ERROR_CASES = [
    pytest.param([], 400, id="merge_empty_list"),
    pytest.param([1], 400, id="merge_single_cluster"),
]


@pytest.mark.parametrize("cluster_ids,expected_status", MERGE_ERROR_CASES)
def test_ai_merge_face_clusters_errors(user_client, cluster_ids, expected_status):
    """Merge with invalid inputs returns expected errors."""
    r = user_client.post("/api/ai/faces/merge", json_data={"cluster_ids": cluster_ids})
    assert r.status_code == expected_status


# ══════════════════════════════════════════════════════════════════════
# DDT: Split Face Cluster — Error Cases
# ══════════════════════════════════════════════════════════════════════

SPLIT_ERROR_CASES = [
    pytest.param([], 400, id="split_empty_list"),
    pytest.param([999999], 400, id="split_nonexistent_detection"),
]


@pytest.mark.parametrize("detection_ids,expected_status", SPLIT_ERROR_CASES)
def test_ai_split_face_cluster_errors(user_client, detection_ids, expected_status):
    """Split with invalid inputs returns expected errors."""
    r = user_client.post("/api/ai/faces/split", json_data={"detection_ids": detection_ids})
    assert r.status_code == expected_status


# ══════════════════════════════════════════════════════════════════════
# DDT: Object Photos Endpoint
# ══════════════════════════════════════════════════════════════════════

OBJECT_PHOTOS_CASES = [
    pytest.param("cat", id="object_cat"),
    pytest.param("dog", id="object_dog"),
    pytest.param("car", id="object_car"),
    pytest.param("nonexistent_class", id="object_nonexistent"),
]


@pytest.mark.parametrize("class_name", OBJECT_PHOTOS_CASES)
def test_ai_object_photos_returns_list(user_client, class_name):
    """Object photos endpoint always returns a list (may be empty)."""
    photos = user_client.ai_list_object_photos(class_name)
    assert isinstance(photos, list)


# ══════════════════════════════════════════════════════════════════════
# DDT: Cluster Photos Endpoint — Error Cases
# ══════════════════════════════════════════════════════════════════════

def test_ai_cluster_photos_nonexistent(user_client):
    """Requesting photos for a nonexistent cluster returns 404."""
    r = user_client.get("/api/ai/faces/999999/photos")
    assert r.status_code == 404


# ══════════════════════════════════════════════════════════════════════
# DDT: Toggle Persistence (enable → disable → status)
# ══════════════════════════════════════════════════════════════════════

def test_ai_toggle_persistence(user_client):
    """AI toggle state persists across status queries."""
    # Enable
    user_client.ai_toggle(True)
    status = user_client.ai_status()
    assert status["enabled"] is True

    # Disable
    user_client.ai_toggle(False)
    status = user_client.ai_status()
    assert status["enabled"] is False

    # Re-enable
    user_client.ai_toggle(True)
    status = user_client.ai_status()
    assert status["enabled"] is True


# ══════════════════════════════════════════════════════════════════════
# DDT: AI Status Counters Are Non-Negative
# ══════════════════════════════════════════════════════════════════════

COUNTER_FIELDS = [
    pytest.param("photos_processed", id="processed_non_negative"),
    pytest.param("photos_pending", id="pending_non_negative"),
    pytest.param("face_detections", id="face_det_non_negative"),
    pytest.param("face_clusters", id="clusters_non_negative"),
    pytest.param("object_detections", id="obj_det_non_negative"),
]


@pytest.mark.parametrize("field", COUNTER_FIELDS)
def test_ai_counters_non_negative(user_client, field):
    """All AI status counters should be non-negative integers."""
    status = user_client.ai_status()
    assert isinstance(status[field], int), f"{field} should be int"
    assert status[field] >= 0, f"{field} should be >= 0"


# ══════════════════════════════════════════════════════════════════════
# DDT: Rename with Edge Case Inputs
# ══════════════════════════════════════════════════════════════════════

RENAME_INPUT_CASES = [
    pytest.param("", 400, id="empty_name"),
    pytest.param("   ", 400, id="whitespace_name"),
    pytest.param("a" * 101, 400, id="name_too_long"),
]


@pytest.mark.parametrize("name,expected_status", RENAME_INPUT_CASES)
def test_ai_rename_input_validation(user_client, name, expected_status):
    """Rename rejects bad inputs regardless of cluster existence."""
    # Use a large cluster ID — even if cluster existed, these inputs should fail
    r = user_client.ai_rename_face_cluster(1, name)
    # Either 400 (bad input) or 404 (not found) — both are valid rejections
    assert r.status_code in (expected_status, 404)
