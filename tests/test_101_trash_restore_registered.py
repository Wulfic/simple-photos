"""
Test 101: restoring a trashed photo brings back the photo, not just its bytes.

Why this exists (TODO.md, F11 — 2026-11-25)
-------------------------------------------
Both clients trash a photo with `POST /api/blobs/{blob}/trash`. That handler
deletes the `photos` row (so encrypted-sync stops listing it) and records only
blob-level facts in `trash_items`. `POST /api/trash/{id}/restore` re-inserts the
`blobs` rows and nothing else. A registered (Android) photo has no file on disk
and no other source for its row, so after a restore:

  * Android pulls `/photos/encrypted-sync`, which reads `photos` — the photo is
    gone from every gallery, although its bytes are still on disk;
  * web looks fine only on the device that trashed it (it rebuilds the tile
    from its own Dexie trash copy); every other device has lost it.

The filed suspicion was the other delete path, `DELETE /api/photos/{id}`, whose
trash row drops `encrypted_blob_id` / `encrypted_thumb_blob_id`. No client calls
it, but it is a public endpoint, so it is held to the same contract.

Deleting the `photos` row also CASCADEs (foreign_keys is on) every row that
hangs off it; a user's edit copies must survive a trash → restore round trip.
"""

import hashlib

from helpers import generate_random_bytes


def _short_hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:12]


def _register_like_android(client, filename):
    """The phone's call order: thumb blob, media blob, register."""
    original = generate_random_bytes(4096)
    photo_hash = _short_hash(original)
    thumb_bytes = generate_random_bytes(512)
    media_bytes = generate_random_bytes(8192)
    thumb = client.upload_blob("thumbnail", thumb_bytes)["blob_id"]
    # v2 (chunked) — the format every large phone upload uses.
    media = client.upload_blob("photo", media_bytes,
                               content_hash=photo_hash, blob_format=2)["blob_id"]
    r = client.post("/api/photos/register-encrypted", json_data={
        "filename": filename,
        "mime_type": "image/jpeg",
        "media_type": "photo",
        "width": 4032,
        "height": 3024,
        "taken_at": "2026-04-18T02:16:41.872Z",
        "encrypted_blob_id": media,
        "encrypted_thumb_blob_id": thumb,
        "photo_hash": photo_hash,
    })
    assert r.status_code == 201, r.text
    return {"photo_id": r.json()["photo_id"], "blob": media, "thumb": thumb,
            "original_size": len(original),
            "media_bytes": media_bytes, "thumb_bytes": thumb_bytes,
            "photo_hash": photo_hash, "filename": filename}


def _sync_record(client, photo_id):
    for rec in client.encrypted_sync_all():
        if rec["id"] == photo_id:
            return rec
    return None


def _trash_like_the_clients(client, reg):
    """What PhotoRepository.deletePhoto / web trashPhotos send.

    `size_bytes` is the phone's `photo.sizeBytes` — the ORIGINAL file's size,
    not the encrypted blob's. A restore that trusts it serves a wrong length.
    """
    out = client.soft_delete_blob(
        reg["blob"], filename=reg["filename"], mime_type="image/jpeg",
        media_type="photo", size_bytes=reg["original_size"],
        width=4032, height=3024, taken_at="2026-04-18T02:16:41.872Z",
        thumbnail_blob_id=reg["thumb"],
    )
    return out["trash_id"]


def _assert_restored(client, reg):
    rec = _sync_record(client, reg["photo_id"])
    assert rec is not None, (
        "restored photo is not in /photos/encrypted-sync — every client that "
        "syncs from the server has lost it"
    )
    assert rec["encrypted_blob_id"] == reg["blob"]
    assert rec["encrypted_thumb_blob_id"] == reg["thumb"]
    assert rec["photo_hash"] == reg["photo_hash"]
    assert rec["filename"] == reg["filename"]
    assert rec["taken_at"] == "2026-04-18T02:16:41.872Z"
    media = client.download_blob(reg["blob"])
    assert media.status_code == 200
    assert len(media.content) == len(reg["media_bytes"]), "served a wrong length"
    assert media.content == reg["media_bytes"]
    assert media.headers.get("X-Blob-Format") == "2", (
        "a v2 blob restored as v1 cannot be decrypted"
    )
    thumb = client.download_blob(reg["thumb"])
    assert thumb.status_code == 200
    assert len(thumb.content) == len(reg["thumb_bytes"]), "served a wrong length"
    assert thumb.content == reg["thumb_bytes"]


class TestRestoreRegisteredPhoto:

    def test_blob_trash_then_restore_returns_the_photo_row(self, user_client):
        reg = _register_like_android(user_client, "PXL_F11_blob.jpg")
        assert _sync_record(user_client, reg["photo_id"]) is not None

        trash_id = _trash_like_the_clients(user_client, reg)
        assert _sync_record(user_client, reg["photo_id"]) is None, (
            "a trashed photo must leave encrypted-sync"
        )

        r = user_client.restore_trash(trash_id)
        assert r.status_code in (200, 204), r.text
        _assert_restored(user_client, reg)

    def test_photo_delete_then_restore_keeps_its_blob_ids(self, user_client):
        reg = _register_like_android(user_client, "PXL_F11_photo.jpg")

        r = user_client.delete(f"/api/photos/{reg['photo_id']}")
        assert r.status_code == 200, r.text
        trash_id = r.json()["trash_id"]
        assert _sync_record(user_client, reg["photo_id"]) is None

        r = user_client.restore_trash(trash_id)
        assert r.status_code in (200, 204), r.text
        _assert_restored(user_client, reg)

    def test_edit_copy_survives_trash_and_restore(self, user_client):
        reg = _register_like_android(user_client, "PXL_F11_copy.jpg")
        r = user_client.post(f"/api/photos/{reg['photo_id']}/copies", json_data={
            "name": "Cropped", "edit_metadata": '{"crop":{"x":0.1}}',
        })
        assert r.status_code in (200, 201), r.text

        trash_id = _trash_like_the_clients(user_client, reg)
        r = user_client.restore_trash(trash_id)
        assert r.status_code in (200, 204), r.text

        r = user_client.get(f"/api/photos/{reg['photo_id']}/copies")
        assert r.status_code == 200, r.text
        body = r.json()
        copies = body.get("copies", body) if isinstance(body, dict) else body
        assert [c["name"] for c in copies] == ["Cropped"], body

    def test_tag_survives_trash_and_restore(self, user_client):
        reg = _register_like_android(user_client, "PXL_F11_tag.jpg")
        r = user_client.post(f"/api/photos/{reg['photo_id']}/tags",
                             json_data={"tag": "beach"})
        assert r.status_code in (200, 201, 204), r.text

        trash_id = _trash_like_the_clients(user_client, reg)
        r = user_client.restore_trash(trash_id)
        assert r.status_code in (200, 204), r.text

        r = user_client.get(f"/api/photos/{reg['photo_id']}/tags")
        assert r.status_code == 200, r.text
        assert "beach" in str(r.json()), r.json()
