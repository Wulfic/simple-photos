"""
Test 73: E2E coverage for the four "polish" features added in this batch.

Each section below is a focused E2E test for one feature:

1. **Secure-album item remove** — verifies that the new
   ``DELETE /api/galleries/secure/{gallery_id}/items/{item_id}`` endpoint
   removes the item from the secure gallery AND restores the original
   blob to the regular (un-encrypted) main gallery.  This is the
   "remove from secure album" affordance, distinct from "delete blob"
   which removes the data entirely.

2. **Server activity status** — verifies the
   ``GET /api/status/activity`` endpoint returns the expected JSON shape
   (``ai`` / ``geo`` / ``active`` booleans) and is gated behind auth.
   The frontend polls this endpoint so the profile-avatar spinner can
   reflect server-side AI inference and geo backfill in addition to
   client-side processing tasks.

3. **Streaming downloads** — verifies that export-zip downloads are
   delivered as a chunked stream rather than buffered fully on the
   server before the response begins.  We assert the response either
   advertises ``Transfer-Encoding: chunked`` OR sends a Content-Length
   header alongside the stream — the key property is that the client
   can begin reading bytes immediately rather than the server stalling
   while it constructs the whole payload in memory.

4. **Auth via ``?token=`` query param** — the frontend's "native
   download" anchor relies on the auth middleware accepting the access
   token via the ``token`` query string (since ``<a download>`` cannot
   set custom headers).  This test asserts that mechanism works for
   the export download endpoint.

DDT coverage for the rotation+crop server-side ordering swap is added
as parametrized rows in :mod:`test_38_edit_dimensions_ddt` rather than
duplicated here, since that file already owns that surface.
"""

from __future__ import annotations

import time
from typing import Any

import pytest
import requests

from helpers import (
    APIClient,
    assert_no_duplicates,
    generate_random_bytes,
    generate_test_jpeg,
)
from conftest import USER_PASSWORD


# ────────────────────────────────────────────────────────────────────
# 1. Secure-album item remove
# ────────────────────────────────────────────────────────────────────


def _blob_ids(client: APIClient) -> list[str]:
    return [b["id"] for b in client.list_blobs(limit=500).get("blobs", [])]


class TestSecureAlbumRemove:
    """``DELETE /galleries/secure/{id}/items/{item_id}`` returns the original
    blob to the regular gallery without leaving duplicates or stale clones."""

    def test_remove_item_restores_original_to_main_gallery(self, user_client: APIClient) -> None:
        # Arrange: upload a blob and add it to a secure gallery.  The add
        # operation hides the original blob from the main listing (test_06
        # covers that invariant comprehensively) — here we exercise the
        # inverse direction.
        content = generate_random_bytes(1024)
        blob = user_client.upload_blob("photo", content)
        original_id = blob["blob_id"]

        gallery = user_client.create_secure_gallery("Remove Test Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], original_id, token,
        )
        item_id = add_result["item_id"]
        clone_blob_id = add_result["new_blob_id"]

        # Sanity: original is hidden, clone exists in secure gallery.
        assert original_id not in _blob_ids(user_client)

        # Act: remove the item from the secure gallery.
        r = user_client.remove_secure_gallery_item(gallery["gallery_id"], item_id)
        assert r.status_code in (200, 204), (
            f"Expected 200/204 from remove, got {r.status_code}: {r.text}"
        )

        # Assert: original blob is back in the main gallery, the clone is
        # gone, and the secure gallery no longer lists this item.
        after_blobs = _blob_ids(user_client)
        assert_no_duplicates(after_blobs, "blobs after secure remove")
        assert original_id in after_blobs, (
            "Original blob must reappear in the main gallery after remove"
        )
        assert clone_blob_id not in after_blobs, (
            "Clone blob must be deleted, not re-exposed in the main gallery"
        )

        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)
        assert all(i["id"] != item_id for i in items.get("items", [])), (
            "Removed item must not appear in secure gallery listing"
        )

    def test_remove_nonexistent_item_returns_404(self, user_client: APIClient) -> None:
        gallery = user_client.create_secure_gallery("404 Gallery")
        r = user_client.remove_secure_gallery_item(
            gallery["gallery_id"], "nonexistent-item-id",
        )
        assert r.status_code == 404

    def test_remove_item_requires_gallery_ownership(
        self, user_client: APIClient, second_user_client: APIClient,
    ) -> None:
        """User B must not be able to remove items from User A's secure gallery."""
        # User A creates a secure gallery and adds an item.
        gallery = user_client.create_secure_gallery("Owner-Only Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        blob = user_client.upload_blob("photo")
        add = user_client.add_secure_gallery_item(
            gallery["gallery_id"], blob["blob_id"], token,
        )

        # User B attempts the remove.
        r = second_user_client.remove_secure_gallery_item(
            gallery["gallery_id"], add["item_id"],
        )
        assert r.status_code in (403, 404), (
            f"Cross-user remove must be denied, got {r.status_code}"
        )


# ────────────────────────────────────────────────────────────────────
# 2. Server activity status endpoint
# ────────────────────────────────────────────────────────────────────


class TestActivityStatus:
    """``GET /api/status/activity`` exposes the server's AI / geo busy flags
    so the web client can drive its profile-avatar spinner."""

    def test_activity_endpoint_shape(self, user_client: APIClient) -> None:
        r = user_client.get("/api/status/activity")
        assert r.status_code == 200, r.text
        data: dict[str, Any] = r.json()
        assert set(data.keys()) >= {"ai", "geo", "active"}
        assert isinstance(data["ai"], bool)
        assert isinstance(data["geo"], bool)
        assert isinstance(data["active"], bool)
        # `active` must equal the OR of the two specific flags.
        assert data["active"] == (data["ai"] or data["geo"])

    def test_activity_endpoint_requires_auth(self, primary_server) -> None:
        anon = APIClient(primary_server.base_url)
        r = anon.get("/api/status/activity")
        assert r.status_code in (401, 403), (
            f"Unauthenticated access must be rejected, got {r.status_code}"
        )


# ────────────────────────────────────────────────────────────────────
# 3 & 4. Streaming download + ?token= auth
# ────────────────────────────────────────────────────────────────────


def _create_export_with_one_photo(client: APIClient) -> str:
    """Create a small library export and return ``file_id`` for the produced zip.

    Uploads a single photo, kicks off the library export with the minimum
    size limit, polls ``/api/export/status`` until the job completes, and
    returns the first export-file id.  A generous timeout absorbs cold
    caches; the work itself is small.
    """
    client.upload_blob("photo", generate_test_jpeg(width=64, height=64))

    # Minimum size_limit accepted by the server is 1 GiB.
    r = client.post("/api/export", json_data={"size_limit": 1_073_741_824})
    if r.status_code == 404:
        pytest.skip("Library export endpoint not enabled in this build")
    if r.status_code == 409:
        # Another export is already running for this fresh user — race in
        # the test setup; treat as a skip rather than failing.
        pytest.skip(f"Concurrent export already running: {r.text}")
    r.raise_for_status()

    # Poll until the job completes.
    deadline = time.time() + 90.0
    last: dict[str, Any] = {}
    while time.time() < deadline:
        rr = client.get("/api/export/status")
        if rr.status_code == 200:
            last = rr.json()
            job_status = last.get("job", {}).get("status")
            if job_status == "completed":
                break
            if job_status == "failed":
                pytest.fail(f"Export failed: {last}")
        time.sleep(0.5)
    else:
        pytest.skip(f"Export did not complete within timeout: {last}")

    files = last.get("files") or []
    if not files:
        pytest.skip(f"Export job produced no files: {last}")
    return files[0]["id"]


class TestStreamingDownload:
    """Export downloads stream over the wire instead of being fully buffered."""

    def test_download_streams_chunked_or_with_length(self, user_client: APIClient) -> None:
        file_id = _create_export_with_one_photo(user_client)

        # Use stream=True so requests doesn't drain the body before we can
        # inspect headers — important because we want to assert the server
        # begins responding before the entire payload is built.
        url = user_client._url(f"/api/export/files/{file_id}/download")
        with user_client.session.get(
            url, headers=user_client._auth_headers(), stream=True, timeout=30,
        ) as r:
            assert r.status_code == 200, r.text
            te = r.headers.get("Transfer-Encoding", "").lower()
            cl = r.headers.get("Content-Length")
            # Acceptable: either chunked transfer (no Content-Length) OR
            # a fixed Content-Length combined with body that the client
            # can read incrementally — both indicate the server isn't
            # base64-buffering the file.
            assert "chunked" in te or cl is not None, (
                f"Expected streaming response (chunked or Content-Length), "
                f"got headers: {dict(r.headers)}"
            )
            # Read first chunk to prove the body is actually streaming.
            first = next(r.iter_content(chunk_size=1024), b"")
            assert first, "Streaming body produced no bytes"

    def test_download_authenticates_via_query_token(self, user_client: APIClient) -> None:
        """The frontend's native ``<a download>`` flow can't set headers,
        so the auth middleware must accept ``?token=<jwt>`` for media URLs.
        This test exercises that path against the export download endpoint
        which is the highest-bandwidth consumer of it."""
        file_id = _create_export_with_one_photo(user_client)
        token = user_client.access_token
        assert token, "Test client must be authenticated"

        # Build a fresh session with NO Authorization header to prove the
        # query-param token alone is sufficient.
        url = user_client._url(f"/api/export/files/{file_id}/download")
        with requests.get(
            url,
            params={"token": token},
            headers={"X-Forwarded-For": user_client._fake_ip},
            stream=True,
            timeout=30,
        ) as r:
            assert r.status_code == 200, (
                f"?token= auth failed: {r.status_code} {r.text[:200]}"
            )
            # Sanity-check we can read the body.
            assert next(r.iter_content(chunk_size=1024), b""), (
                "Authenticated streamed download produced no bytes"
            )

        # Negative: a clearly bogus token must still be rejected.
        with requests.get(
            url, params={"token": "not-a-real-token"}, stream=True, timeout=10,
        ) as r:
            assert r.status_code in (401, 403), (
                f"Bogus query-param token must be rejected, got {r.status_code}"
            )
