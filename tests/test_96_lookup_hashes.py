"""
Test 96: `POST /api/photos/lookup-hashes` tells a client what it need not upload.

Why this exists (TODO.md F2a — measured on CT132, 2026-11-25)
-------------------------------------------------------------
A reinstalled phone re-uploads every photo it holds. In the first four minutes of
the S21+ reinstall the server reported 298/298 media uploads as content-hash
duplicates and discarded them — but each one also uploaded a thumbnail that WAS
stored, then registered as a duplicate, leaving one orphan thumbnail per photo.

The phone already hashes before uploading. This endpoint lets it ask first, and
adopt the existing photo's ids without sending any bytes.
"""

import hashlib

from helpers import APIClient, generate_random_bytes


def _short_hash(data: bytes) -> str:
    # The phone's and the server's `photo_hash`: sha256(original)[..6] as hex.
    return hashlib.sha256(data).hexdigest()[:12]


def _registered_photo(client):
    """A photo exactly as the phone leaves it: thumb + media blob, registered."""
    original = generate_random_bytes(4096)
    photo_hash = _short_hash(original)
    thumb = client.upload_blob("thumbnail", generate_random_bytes(512))["blob_id"]
    media = client.upload_blob("photo", generate_random_bytes(8192),
                               content_hash=photo_hash)["blob_id"]
    r = client.post("/api/photos/register-encrypted", json_data={
        "filename": "IMG_0001.jpg",
        "mime_type": "image/jpeg",
        "media_type": "photo",
        "taken_at": "2026-04-18T02:16:41.872Z",
        "encrypted_blob_id": media,
        "encrypted_thumb_blob_id": thumb,
        "photo_hash": photo_hash,
    })
    assert r.status_code == 201, r.text
    return {"photo_id": r.json()["photo_id"], "blob": media, "thumb": thumb,
            "photo_hash": photo_hash}


def _lookup(client, hashes):
    return client.post("/api/photos/lookup-hashes", json_data={"hashes": hashes})


class TestLookupHashes:

    def test_a_held_hash_names_the_photo_and_its_blobs(self, user_client):
        photo = _registered_photo(user_client)
        miss = _short_hash(generate_random_bytes(4096))

        r = _lookup(user_client, [photo["photo_hash"], miss])

        assert r.status_code == 200, r.text
        matches = r.json()["matches"]
        assert matches == {
            photo["photo_hash"]: {
                "photo_id": photo["photo_id"],
                "encrypted_blob_id": photo["blob"],
                "encrypted_thumb_blob_id": photo["thumb"],
            }
        }, "a miss must be absent, a hit must carry the canonical ids"

    def test_another_users_photo_is_a_miss(self, user_client, second_user_client):
        theirs = _registered_photo(second_user_client)

        r = _lookup(user_client, [theirs["photo_hash"]])

        assert r.status_code == 200, r.text
        assert r.json()["matches"] == {}, "lookup leaked another user's photo"

    def test_more_than_500_hashes_is_rejected(self, user_client):
        r = _lookup(user_client, ["aaaaaaaaaaaa"] * 501)
        assert r.status_code == 400, r.text

    def test_lookup_requires_auth(self, primary_server):
        r = _lookup(APIClient(primary_server.base_url), ["aaaaaaaaaaaa"])
        assert r.status_code == 401, r.text
