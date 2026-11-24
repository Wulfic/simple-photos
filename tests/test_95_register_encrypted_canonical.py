"""
Test 95: `POST /api/photos/register-encrypted` tells a re-uploading client which
blobs are canonical.

Why this exists (TODO.md, D2/S1 — measured on CT132, 2026-11-17)
----------------------------------------------------------------
A reinstalled phone re-uploads photos the server already holds. The server's
register dedups by content hash and answers with the EXISTING photo id
(`duplicate: true`) — but it never said which blobs that existing photo uses. The
phone therefore marked its row with the existing photo id *and its own freshly
uploaded blob id*, which:

- is referenced by nothing server-side (an orphan the server keeps forever), and
- is not in `/galleries/secure/blob-ids`, so when the existing photo is secured the
  phone's blob-keyed secure filter showed it in the main grid (158 such rows live).

The phone cannot fix that without being told the canonical ids, so a duplicate
answer must carry them. These tests replay the phone's exact call order:
thumbnail blob, media blob, then register.
"""

import hashlib

from helpers import generate_random_bytes


def _short_hash(data: bytes) -> str:
    # The phone's and the server's `photo_hash`: sha256(original)[..6] as hex.
    return hashlib.sha256(data).hexdigest()[:12]


def _register(client, *, blob_id, thumb_id, photo_hash, filename="IMG_0001.jpg"):
    return client.post("/api/photos/register-encrypted", json_data={
        "filename": filename,
        "mime_type": "image/jpeg",
        "media_type": "photo",
        "width": 4032,
        "height": 3024,
        "taken_at": "2026-04-18T02:16:41.872Z",
        "encrypted_blob_id": blob_id,
        "encrypted_thumb_blob_id": thumb_id,
        "photo_hash": photo_hash,
    })


def _first_upload(client):
    """The photo as the server first learned it: media + thumb, registered."""
    original = generate_random_bytes(4096)
    photo_hash = _short_hash(original)
    thumb = client.upload_blob("thumbnail", generate_random_bytes(512))["blob_id"]
    media = client.upload_blob("photo", generate_random_bytes(8192),
                               content_hash=photo_hash)["blob_id"]
    r = _register(client, blob_id=media, thumb_id=thumb, photo_hash=photo_hash)
    assert r.status_code == 201, r.text
    return {"photo_id": r.json()["photo_id"], "blob": media, "thumb": thumb,
            "photo_hash": photo_hash}


class TestDuplicateRegisterNamesCanonicalBlobs:

    def test_hash_match_returns_the_existing_photos_blobs(self, user_client):
        """The reinstall case: the second upload got a NEW blob id (its server-side
        twin carried no content hash, so blob dedup could not collapse it), and only
        the photo-hash arm of register recognises it."""
        first = _first_upload(user_client)

        # Second device / reinstall: new thumb, and a media blob with no content
        # hash header — exactly what lands when the original's blob has none.
        thumb2 = user_client.upload_blob("thumbnail", generate_random_bytes(512))["blob_id"]
        media2 = user_client.upload_blob("photo", generate_random_bytes(8192))["blob_id"]
        assert media2 != first["blob"], "precondition: the re-upload must be a new blob"

        r = _register(user_client, blob_id=media2, thumb_id=thumb2,
                      photo_hash=first["photo_hash"])
        assert r.status_code == 200, r.text
        body = r.json()
        assert body["duplicate"] is True
        assert body["photo_id"] == first["photo_id"]
        # The point of the change: the answer names the blobs the server actually
        # serves for that photo, not the ones this client just sent.
        assert body.get("encrypted_blob_id") == first["blob"], body
        assert body.get("encrypted_thumb_blob_id") == first["thumb"], body

    def test_blob_match_returns_the_existing_photos_blobs(self, user_client):
        """The same-blob arm (a client re-registering a blob it already registered)
        must answer with the same shape, so a client needs one code path."""
        first = _first_upload(user_client)
        r = _register(user_client, blob_id=first["blob"], thumb_id=None,
                      photo_hash=first["photo_hash"])
        assert r.status_code == 200, r.text
        body = r.json()
        assert body["duplicate"] is True
        assert body["photo_id"] == first["photo_id"]
        assert body.get("encrypted_blob_id") == first["blob"], body
        assert body.get("encrypted_thumb_blob_id") == first["thumb"], body

    def test_a_new_photo_is_still_created_and_not_marked_duplicate(self, user_client):
        """Guard: the canonical-id fields belong to the duplicate answer only. A
        genuinely new photo keeps the 201 shape every client already parses."""
        original = generate_random_bytes(4096)
        thumb = user_client.upload_blob("thumbnail", generate_random_bytes(512))["blob_id"]
        media = user_client.upload_blob("photo", generate_random_bytes(8192),
                                        content_hash=_short_hash(original))["blob_id"]
        r = _register(user_client, blob_id=media, thumb_id=thumb,
                      photo_hash=_short_hash(original))
        assert r.status_code == 201, r.text
        assert "duplicate" not in r.json()
