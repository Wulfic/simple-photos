"""
Test 98: `POST /api/admin/maintenance/orphan-thumbnails` — the guarded sweeper.

Why this exists (TODO.md F1 — measured on a copy of CT132's DB, 2026-11-25)
-------------------------------------------------------------------------
A phone re-uploading what the server already holds leaves one stored thumbnail
per photo that nothing references: 18,067 of them (754 MB) on CT132. The sweeper
moves them to a quarantine; restore reverses a run; purge is the only unlink.

Everything this suite uploads is younger than the 24h grace period, so here the
sweeper can only *report* — which is exactly the property worth pinning on a
live server: a fresh upload whose registration has not landed yet is never
touched, not even by an `apply`. The move / restore / purge paths themselves are
covered by the unit tests in `server/src/maintenance/orphan_thumbs_tests.rs`.
"""

import uuid

from helpers import APIClient, generate_random_bytes

SWEEP = "/api/admin/maintenance/orphan-thumbnails"


def _sweep(client, **body):
    return client.post(SWEEP, json_data=body)


def _registered_photo(client):
    """So the server's reference set is never empty (that refuses, by design)."""
    thumb = client.upload_blob("thumbnail", generate_random_bytes(512))["blob_id"]
    media = client.upload_blob("photo", generate_random_bytes(4096))["blob_id"]
    r = client.post("/api/photos/register-encrypted", json_data={
        "filename": "IMG_0098.jpg",
        "mime_type": "image/jpeg",
        "media_type": "photo",
        "taken_at": "2026-04-18T02:16:41.872Z",
        "encrypted_blob_id": media,
        "encrypted_thumb_blob_id": thumb,
    })
    assert r.status_code in (200, 201), r.text


class TestOrphanThumbnailSweep:

    def test_default_is_a_dry_run_that_keeps_a_young_orphan(self, admin_client):
        _registered_photo(admin_client)
        before = _sweep(admin_client)
        assert before.status_code == 200, before.text

        orphan = admin_client.upload_blob("thumbnail", generate_random_bytes(256))["blob_id"]
        after = _sweep(admin_client)

        assert after.status_code == 200, after.text
        a, b = after.json(), before.json()
        assert a["dry_run"] is True
        assert a["run_id"] is None
        assert a["unreferenced"] == b["unreferenced"] + 1
        assert a["kept_young"] == b["kept_young"] + 1, "a fresh upload is inside the grace period"
        assert orphan not in a["sample"]
        assert admin_client.get(f"/api/blobs/{orphan}").status_code == 200

    def test_apply_requires_the_exact_count(self, admin_client):
        _registered_photo(admin_client)
        n = _sweep(admin_client).json()["candidates"]

        missing = _sweep(admin_client, apply=True)
        wrong = _sweep(admin_client, apply=True, confirm_count=n + 1)

        assert missing.status_code == 400, missing.text
        assert wrong.status_code == 409, wrong.text

    def test_apply_never_touches_a_young_orphan(self, admin_client):
        _registered_photo(admin_client)
        orphan = admin_client.upload_blob("thumbnail", generate_random_bytes(256))["blob_id"]
        n = _sweep(admin_client).json()["candidates"]

        r = _sweep(admin_client, apply=True, confirm_count=n, limit=5000)

        assert r.status_code == 200, r.text
        body = r.json()
        assert body["dry_run"] is False
        uuid.UUID(body["run_id"])
        assert body["failed"] == 0
        assert admin_client.get(f"/api/blobs/{orphan}").status_code == 200, \
            "an apply quarantined an upload still inside its grace period"

    def test_restore_and_purge_need_a_real_run_id(self, admin_client):
        for path in (f"{SWEEP}/restore", f"{SWEEP}/purge"):
            for run_id in ("", "../../blobs", "*"):
                r = admin_client.post(path, json_data={"run_id": run_id, "apply": True,
                                                       "confirm_count": 0})
                assert r.status_code == 400, f"{path} {run_id!r}: {r.text}"

    def test_an_unknown_run_is_an_empty_dry_run(self, admin_client):
        run_id = str(uuid.uuid4())
        for path in (f"{SWEEP}/restore", f"{SWEEP}/purge"):
            r = admin_client.post(path, json_data={"run_id": run_id})
            assert r.status_code == 200, r.text
            assert r.json()["dry_run"] is True
            assert r.json()["rows"] == 0

    def test_non_admin_is_forbidden(self, user_client):
        for path in (SWEEP, f"{SWEEP}/restore", f"{SWEEP}/purge"):
            r = user_client.post(path, json_data={"run_id": str(uuid.uuid4())})
            assert r.status_code == 403, f"{path}: {r.text}"

    def test_requires_auth(self, primary_server):
        anon = APIClient(primary_server.base_url)
        r = anon.post(SWEEP, json_data={})
        assert r.status_code == 401, r.text
