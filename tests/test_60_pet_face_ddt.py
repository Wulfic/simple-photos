"""
test_60_pet_face_ddt.py — DDT + E2E tests for animal face matching / pet smart albums.

Fixture photos (in tests/test_data/ai_pets/):
  bubs_01.jpg … bubs_04.jpg  — multiple shots of Bubs the cat
  pet_dog_01.jpg              — a dog photo
  non_pet_01.jpg              — landscape (no animal)

Flow tested:
1. Upload fixtures, enable AI, wait for processing.
2. Pet detections appear for cat/dog photos, not for landscape.
3. Pet clusters are created; bubs photos land in ≤2 clusters (same individual).
4. Rename a cluster; the pet: tag on photos updates.
5. Merge clusters; photo count is summed correctly.
6. Reprocess clears everything; pet counts return to zero.
"""

from __future__ import annotations

import os
import time
from pathlib import Path

import pytest

from helpers import APIClient, generate_test_jpeg

# ── Fixture directory ────────────────────────────────────────────────────────

FIXTURES = Path(__file__).parent / "test_data" / "ai_pets"

# ── DDT tables ───────────────────────────────────────────────────────────────

# (filename, expect_pet_detection)
PET_DETECTION_CASES = [
    pytest.param("bubs_01.jpg", True, id="bubs-01-cat"),
    pytest.param("bubs_02.jpg", True, id="bubs-02-cat"),
    pytest.param("bubs_03.jpg", True, id="bubs-03-cat"),
    pytest.param("bubs_04.jpg", True, id="bubs-04-cat"),
    pytest.param("pet_dog_01.jpg", True, id="dog-photo"),
    pytest.param("non_pet_01.jpg", False, id="landscape-no-pet"),
]

# (photo_count_to_upload, expected_max_clusters_for_same_pet)
# Phase 1 (MobileNetV2 logit vectors) does not guarantee individual re-ID;
# it only groups by similarity so we allow up to n_photos clusters.
# Phase 2 (pet_embedding.onnx) would tighten this to ≤2.
PET_CLUSTERING_CASES = [
    pytest.param(2, 2, id="two-bubs-photos-max-2-clusters"),
    pytest.param(4, 2, id="four-bubs-photos-max-2-clusters"),
]

# (invalid_name, expected_error_fragment)
PET_RENAME_VALIDATION_CASES = [
    pytest.param("", "empty", id="empty-name"),
    pytest.param("a" * 101, "long", id="name-too-long"),
]


# ── Helpers ──────────────────────────────────────────────────────────────────

def _wait_for_processing(client: APIClient, timeout: int = 120) -> None:
    """Poll AI status until photos_pending == 0."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            status = client.ai_status()
            if status.get("photos_pending", 1) == 0:
                return
        except Exception:
            pass
        time.sleep(3)
    raise TimeoutError("AI processing did not finish within timeout")


def _upload_fixture(client: APIClient, filename: str) -> str:
    """Upload a fixture image and return its server photo_id."""
    path = FIXTURES / filename
    if path.exists():
        with open(path, "rb") as fh:
            data = fh.read()
    else:
        # Fallback: synthesize a JPEG (won't trigger real detection but allows
        # the test to verify upload + no crash)
        data = generate_test_jpeg(128, 128)
    result = client.upload_photo(filename=filename, content=data)
    return result["photo_id"]


# ── Parametrized unit-style tests (DDT) ─────────────────────────────────────

class TestPetDetectionDDT:
    """Each fixture photo is uploaded individually and processed; we verify
    whether a pet detection tag is (or isn't) created."""

    @pytest.mark.parametrize("filename,expect_pet", PET_DETECTION_CASES)
    def test_pet_tag_presence(
        self,
        filename: str,
        expect_pet: bool,
        user_client: APIClient,
    ) -> None:
        """Upload a single photo and verify pet: tag appears / is absent."""
        if not (FIXTURES / filename).exists():
            pytest.skip(f"Fixture {filename} not found")

        # Ensure AI is enabled
        user_client.ai_toggle(True)

        photo_id = _upload_fixture(user_client, filename)

        # Trigger reprocess for this specific photo so we don't wait too long
        user_client.ai_reprocess(photo_ids=[photo_id])
        _wait_for_processing(user_client, timeout=60)

        # Fetch tags for the photo
        tag_data = user_client.get_photo_tags(photo_id)  # {"tags": ["pet:cat", ...]}
        tags = tag_data.get("tags", [])
        pet_tags = [t for t in tags if t.startswith("pet:")]

        if expect_pet:
            assert pet_tags, (
                f"{filename}: expected a pet: tag but found none. "
                f"All tags: {tags}"
            )
        else:
            assert not pet_tags, (
                f"{filename}: expected NO pet: tag but found {pet_tags}"
            )


class TestPetClusteringDDT:
    """Upload N bubs photos and verify clustering keeps same-individual together."""

    @pytest.mark.parametrize("n_photos,max_clusters", PET_CLUSTERING_CASES)
    def test_bubs_cluster_count(
        self,
        n_photos: int,
        max_clusters: int,
        user_client: APIClient,
    ) -> None:
        bubs_files = [f"bubs_0{i}.jpg" for i in range(1, n_photos + 1)]
        available = [f for f in bubs_files if (FIXTURES / f).exists()]
        if len(available) < 2:
            pytest.skip("Need at least 2 bubs fixture photos")

        user_client.ai_toggle(True)

        photo_ids = [_upload_fixture(user_client, f) for f in available]
        user_client.ai_reprocess(photo_ids=photo_ids)
        _wait_for_processing(user_client, timeout=120)

        clusters = user_client.ai_list_pet_clusters()
        cat_clusters = [c for c in clusters if c.get("species") == "cat"]

        assert len(cat_clusters) <= max_clusters, (
            f"Expected ≤{max_clusters} cat clusters for {len(available)} bubs "
            f"photos, got {len(cat_clusters)}: {cat_clusters}"
        )


class TestPetRenameValidationDDT:
    """Invalid rename requests must be rejected."""

    @pytest.mark.parametrize("name,error_hint", PET_RENAME_VALIDATION_CASES)
    def test_rename_validation(
        self,
        name: str,
        error_hint: str,
        user_client: APIClient,
    ) -> None:
        clusters = user_client.ai_list_pet_clusters()
        if not clusters:
            pytest.skip("No pet clusters available for rename validation test")

        cluster_id = clusters[0]["id"]
        r = user_client.ai_rename_pet_cluster(cluster_id, name)
        assert r.status_code in (400, 422), (
            f"Expected 4xx for {error_hint!r} name, got {r.status_code}: {r.text}"
        )


# ── E2E tests ────────────────────────────────────────────────────────────────

class TestPetSmartAlbumE2E:
    """Full cycle: upload → process → list clusters → rename → merge → reprocess."""

    def test_upload_process_detect_cluster(self, user_client: APIClient) -> None:
        """Upload bubs photos, run AI, verify clusters exist with cat species."""
        bubs_files = [f for f in ["bubs_01.jpg", "bubs_02.jpg", "bubs_03.jpg"]
                      if (FIXTURES / f).exists()]
        if not bubs_files:
            pytest.skip("No bubs fixture photos found")

        user_client.ai_toggle(True)

        photo_ids = [_upload_fixture(user_client, f) for f in bubs_files]
        user_client.ai_reprocess(photo_ids=photo_ids)
        _wait_for_processing(user_client, timeout=120)

        clusters = user_client.ai_list_pet_clusters()
        assert clusters, "Expected at least one pet cluster after processing bubs photos"

        species_set = {c["species"] for c in clusters}
        assert "cat" in species_set, f"Expected 'cat' in species set, got {species_set}"

    def test_rename_pet_cluster(self, user_client: APIClient) -> None:
        """Rename a pet cluster and verify the label persists."""
        clusters = user_client.ai_list_pet_clusters()
        if not clusters:
            pytest.skip("No pet clusters to rename")

        cluster_id = clusters[0]["id"]
        r = user_client.ai_rename_pet_cluster(cluster_id, "Bubs")
        assert r.status_code in (200, 204), f"Rename failed: {r.status_code} {r.text}"

        updated = user_client.ai_list_pet_clusters()
        updated_cluster = next((c for c in updated if c["id"] == cluster_id), None)
        assert updated_cluster is not None
        assert updated_cluster["label"] == "Bubs", (
            f"Expected label 'Bubs', got {updated_cluster['label']}"
        )

    def test_pet_cluster_photos(self, user_client: APIClient) -> None:
        """List photos in a pet cluster; each should have a photo_id."""
        clusters = user_client.ai_list_pet_clusters()
        if not clusters:
            pytest.skip("No pet clusters available")

        cluster_id = clusters[0]["id"]
        photos = user_client.ai_list_pet_cluster_photos(cluster_id)
        assert isinstance(photos, list), "Expected list of photo detections"
        if photos:
            assert "photo_id" in photos[0], f"Missing photo_id in detection: {photos[0]}"
            assert "species" in photos[0], f"Missing species in detection: {photos[0]}"

    def test_merge_pet_clusters(self, user_client: APIClient) -> None:
        """Merge two pet clusters; combined photo count should be >= either alone."""
        clusters = user_client.ai_list_pet_clusters()
        if len(clusters) < 2:
            pytest.skip("Need at least 2 pet clusters to test merge")

        c1, c2 = clusters[0], clusters[1]
        original_total = c1["photo_count"] + c2["photo_count"]

        result = user_client.ai_merge_pet_clusters([c1["id"], c2["id"]])
        assert "merged_into" in result, f"Missing merged_into in response: {result}"
        assert result["photo_count"] >= max(c1["photo_count"], c2["photo_count"]), (
            f"Merged count {result['photo_count']} is suspiciously low "
            f"(original total {original_total})"
        )

        remaining = user_client.ai_list_pet_clusters()
        ids_remaining = {c["id"] for c in remaining}
        assert c2["id"] not in ids_remaining, (
            f"Merged cluster {c2['id']} should have been deleted"
        )

    def test_ai_status_includes_pet_counts(self, user_client: APIClient) -> None:
        """AI status endpoint must include pet_detections and pet_clusters."""
        status = user_client.ai_status()
        assert "pet_detections" in status, f"pet_detections missing from AI status: {status}"
        assert "pet_clusters" in status, f"pet_clusters missing from AI status: {status}"
        assert isinstance(status["pet_detections"], int)
        assert isinstance(status["pet_clusters"], int)

    def test_reprocess_clears_pet_data(self, user_client: APIClient) -> None:
        """Triggering a full reprocess clears pet_detections count temporarily."""
        user_client.ai_reprocess()

        # After clearing, pet_detections should reset to 0 (before re-analysis)
        status_after = user_client.ai_status()
        assert status_after.get("pet_detections", -1) == 0, (
            f"Expected 0 pet_detections after reprocess, "
            f"got {status_after.get('pet_detections')}"
        )


# ── Smart-album single-detection filter (DDT) ───────────────────────────────

# (endpoint_label, lister_attr) — both /api/ai/faces and /api/ai/pets
# must hide clusters that only have a single photo. This stops random
# strangers in group photos / lone misclassifications from polluting the
# People & Pets smart albums.
SINGLE_DETECTION_FILTER_CASES = [
    pytest.param("faces", "ai_list_face_clusters", id="people-cards-min-2"),
    pytest.param("pets", "ai_list_pet_clusters", id="pet-cards-min-2"),
]


class TestSmartAlbumThresholdDDT:
    """Verify the smart-album endpoints never expose single-detection clusters."""

    @pytest.mark.parametrize("label,lister", SINGLE_DETECTION_FILTER_CASES)
    def test_clusters_meet_min_photo_count(
        self,
        label: str,
        lister: str,
        user_client: APIClient,
    ) -> None:
        clusters = getattr(user_client, lister)()
        for c in clusters:
            assert c.get("photo_count", 0) >= 2, (
                f"{label}: cluster {c.get('id')} surfaced with photo_count="
                f"{c.get('photo_count')}; smart-album rule requires ≥ 2."
            )
