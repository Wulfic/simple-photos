"""
Test 14: Library Export — end-to-end regression test for the library export
         feature that packages media into downloadable zip archives.

Covers:
- Starting an export job
- Polling for completion
- Listing export files
- Downloading zip files and verifying content
- Duplicate export rejection (conflict while running)
- Cleanup/deletion of export jobs
- Non-admin user export (export is user-scoped, not admin-only)
"""

import io
import time
import zipfile

import pytest
from helpers import (
    APIClient,
    encrypt_blob_payload,
    generate_random_bytes,
    generate_test_jpeg,
    unique_filename,
)

# Must match conftest.py TEST_ENCRYPTION_KEY
_TEST_KEY_HEX = "a" * 64


class TestLibraryExport:
    """Core export workflow: trigger, poll, download, verify."""

    def test_export_no_prior_jobs(self, user_client):
        """GET /api/export/status returns 404 when no export exists."""
        r = user_client.get("/api/export/status")
        assert r.status_code == 404

    def test_export_no_files_initially(self, user_client):
        """GET /api/export/files returns empty list when no exports exist."""
        r = user_client.get("/api/export/files")
        assert r.status_code == 200
        data = r.json()
        assert data["files"] == []

    def test_start_export_with_blobs(self, user_client):
        """Upload encrypted blobs, start export, poll to completion, download & verify."""
        # 1. Upload encrypted blobs (matching the real client workflow)
        blob_ids = []
        raw_contents = {}  # blob_id -> original raw bytes
        filenames = {}     # blob_id -> filename used in envelope
        for i in range(3):
            raw_data = generate_random_bytes(2048 + i)
            fname = f"test_photo_{i}.jpg"
            encrypted = encrypt_blob_payload(raw_data, _TEST_KEY_HEX, filename=fname)
            blob = user_client.upload_blob("photo", encrypted)
            blob_ids.append(blob["blob_id"])
            raw_contents[blob["blob_id"]] = raw_data
            filenames[blob["blob_id"]] = fname

        assert len(blob_ids) == 3

        # 2. Start export with 10 GB limit (smallest option)
        r = user_client.post("/api/export", json_data={
            "size_limit": 10_737_418_240,  # 10 GB
        })
        assert r.status_code == 200
        job = r.json()
        assert job["status"] in ("pending", "running")
        assert job["id"]
        job_id = job["id"]

        # 2b. While job is still pending/running, files must NOT be visible
        r_early = user_client.get("/api/export/files")
        assert r_early.status_code == 200
        assert r_early.json()["files"] == [], \
            "Export files must not be visible before job completes"

        # 3. Poll for completion (timeout after 30 seconds)
        deadline = time.time() + 30
        final_status = None
        while time.time() < deadline:
            r = user_client.get("/api/export/status")
            assert r.status_code == 200
            data = r.json()
            final_status = data["job"]["status"]
            if final_status in ("completed", "failed"):
                break
            if final_status in ("pending", "running"):
                assert data["files"] == [], \
                    "Files appeared in status response before job completed"
            time.sleep(0.5)

        assert final_status == "completed", f"Export did not complete: {data['job']}"

        # 4. Verify files are listed (only AFTER completion)
        files = data["files"]
        assert len(files) >= 1
        first_file = files[0]
        assert first_file["filename"].startswith("export_part_")
        assert first_file["filename"].endswith(".zip")
        assert first_file["size_bytes"] > 0
        assert first_file["download_url"]

        # 5. Download the first zip and verify it's a valid, non-corrupt zip
        r = user_client.get(first_file['download_url'])
        assert r.status_code == 200
        assert r.headers.get("Content-Type") == "application/zip"

        zip_data = io.BytesIO(r.content)
        with zipfile.ZipFile(zip_data, "r") as zf:
            bad_file = zf.testzip()
            assert bad_file is None, f"Zip integrity check failed on: {bad_file}"

            names = zf.namelist()

            # Should contain manifest.json with valid JSON
            assert "manifest.json" in names
            import json
            manifest = json.loads(zf.read("manifest.json"))
            assert manifest["export_version"] == 1
            assert manifest["blob_count"] == 3
            assert len(manifest["blobs"]) == 3

            # Should contain photo files under photos/ directory
            photo_files = [n for n in names if n.startswith("photos/")]
            assert len(photo_files) == 3, \
                f"Expected 3 photo files in zip, found {len(photo_files)}: {photo_files}"

            # Verify each exported file contains the RAW media bytes
            # (not the JSON envelope or base64 data)
            for blob_id, raw_data in raw_contents.items():
                # The export uses the blob_id + extension as filename
                # (since no photos-table entry exists for these test blobs)
                zip_entry = f"photos/{blob_id}.jpg"
                assert zip_entry in names, \
                    f"Missing photo entry: {zip_entry}. Found: {names}"
                extracted = zf.read(zip_entry)
                assert extracted == raw_data, \
                    f"Blob {blob_id} content mismatch: expected {len(raw_data)} bytes, got {len(extracted)}"

        # 6. GET /api/export/files also returns the files
        r = user_client.get("/api/export/files")
        assert r.status_code == 200
        listed_files = r.json()["files"]
        assert len(listed_files) == len(files)

        # 7. Clean up: delete the export
        r = user_client.delete(f"/api/export/{job_id}")
        assert r.status_code == 204

        # 8. Verify files are gone
        r = user_client.get("/api/export/files")
        assert r.status_code == 200
        assert r.json()["files"] == []

    def test_export_conflict_when_running(self, user_client):
        """Cannot start a second export while one is pending/running."""
        # Upload at least one encrypted blob
        content = generate_random_bytes(1024)
        encrypted = encrypt_blob_payload(content, _TEST_KEY_HEX)
        user_client.upload_blob("photo", encrypted)

        # Start first export
        r = user_client.post("/api/export", json_data={
            "size_limit": 10_737_418_240,
        })
        assert r.status_code == 200
        job_id = r.json()["id"]

        # Attempt second export immediately (while first is pending/running)
        r2 = user_client.post("/api/export", json_data={
            "size_limit": 10_737_418_240,
        })
        # Should fail with 409 Conflict
        assert r2.status_code == 409

        # Wait for the first to finish, then clean up
        deadline = time.time() + 30
        while time.time() < deadline:
            r = user_client.get("/api/export/status")
            if r.json()["job"]["status"] in ("completed", "failed"):
                break
            time.sleep(0.5)

        user_client.delete(f"/api/export/{job_id}")

    def test_export_invalid_size_limit(self, user_client):
        """Reject size limits outside the valid range."""
        # Too small (< 1 GB)
        r = user_client.post("/api/export", json_data={
            "size_limit": 100_000_000,  # 100 MB
        })
        assert r.status_code == 400

        # Too large (> 50 GB)
        r = user_client.post("/api/export", json_data={
            "size_limit": 100_000_000_000,  # 100 GB
        })
        assert r.status_code == 400


class TestExportIsolation:
    """Verify export is user-scoped — one user cannot access another's exports."""

    def test_cross_user_export_isolation(self, user_client, primary_admin):
        """User cannot see or download another user's export files."""
        # User uploads and exports
        content = generate_random_bytes(1024)
        encrypted = encrypt_blob_payload(content, _TEST_KEY_HEX)
        user_client.upload_blob("photo", encrypted)

        r = user_client.post("/api/export", json_data={
            "size_limit": 10_737_418_240,
        })
        assert r.status_code == 200
        job_id = r.json()["id"]

        # Wait for completion
        deadline = time.time() + 30
        while time.time() < deadline:
            r = user_client.get("/api/export/status")
            data = r.json()
            if data["job"]["status"] in ("completed", "failed"):
                break
            time.sleep(0.5)

        assert data["job"]["status"] == "completed"
        file_id = data["files"][0]["id"]

        # Admin user should get 404 trying to download user's export file
        r = primary_admin.get(f"/api/export/files/{file_id}/download")
        assert r.status_code == 404

        # Admin should not see user's files in their export list
        r = primary_admin.get("/api/export/files")
        assert r.status_code == 200
        admin_file_ids = [f["id"] for f in r.json()["files"]]
        assert file_id not in admin_file_ids

        # Cleanup
        user_client.delete(f"/api/export/{job_id}")

    def test_export_requires_auth(self, primary_server):
        """Export endpoints require authentication."""
        import requests
        base = primary_server.base_url

        r = requests.post(f"{base}/api/export",
                          json={"size_limit": 10_737_418_240})
        assert r.status_code == 401

        r = requests.get(f"{base}/api/export/status")
        assert r.status_code == 401

        r = requests.get(f"{base}/api/export/files")
        assert r.status_code == 401


class TestExportEmptyLibrary:
    """Export with no blobs produces a completed job with no files or a single empty/manifest-only zip."""

    def test_export_empty_library(self, user_client):
        """Export completes cleanly even if the user has no blobs."""
        # Fresh user_client comes with a new user each time via fixture,
        # but they may have blobs from other tests. We just test the
        # workflow completes without error.
        r = user_client.post("/api/export", json_data={
            "size_limit": 10_737_418_240,
        })
        assert r.status_code == 200
        job_id = r.json()["id"]

        deadline = time.time() + 30
        while time.time() < deadline:
            r = user_client.get("/api/export/status")
            data = r.json()
            if data["job"]["status"] in ("completed", "failed"):
                break
            time.sleep(0.5)

        # Should complete (not fail)
        assert data["job"]["status"] == "completed"

        # Cleanup
        user_client.delete(f"/api/export/{job_id}")
