"""
Test 97: concurrent `register-encrypted` calls for one hash all get an answer.

Why this exists (TODO.md F6)
----------------------------
`register-encrypted` checks "is this hash already a photo?" and then inserts. The
check and the insert are separate statements, so two registrations of the same
content can both pass the check. `idx_photos_user_hash` (UNIQUE on
`(user_id, photo_hash)`) stops the second insert — but the loser surfaced the raw
constraint error as a **500**, the phone marked the row FAILED, and only a later
retry healed it.

The loser must get the same `200 duplicate` answer the check would have given it,
naming the winner's photo and canonical blobs.
"""

import hashlib
import threading
from concurrent.futures import ThreadPoolExecutor

import requests

from helpers import generate_random_bytes

# Enough simultaneous registrations per hash, over enough hashes, that the old
# check-then-insert loses the race reliably (it did on every run before F6).
CONCURRENT = 12
ROUNDS = 4


def _short_hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:12]


def _upload_pair(client, photo_hash):
    thumb = client.upload_blob("thumbnail", generate_random_bytes(512))["blob_id"]
    # Distinct media bytes per caller: each phone encrypts with a fresh nonce, so
    # the blob ids differ even though the photo (and its hash) is the same.
    media = client.upload_blob("photo", generate_random_bytes(8192),
                               content_hash=photo_hash)["blob_id"]
    return media, thumb


def _register_all_at_once(client, photo_hash, pairs):
    """Fire one registration per pair, released together by a barrier.

    `requests.Session` is not thread-safe, so each call is a plain request with
    the client's own auth headers.
    """
    barrier = threading.Barrier(len(pairs))
    url = client._url("/api/photos/register-encrypted")

    def register(pair):
        media, thumb = pair
        body = {
            "filename": "IMG_0001.jpg",
            "mime_type": "image/jpeg",
            "media_type": "photo",
            "taken_at": "2026-04-18T02:16:41.872Z",
            "encrypted_blob_id": media,
            "encrypted_thumb_blob_id": thumb,
            "photo_hash": photo_hash,
        }
        barrier.wait()
        return requests.post(url, json=body, headers=client._auth_headers(), timeout=30)

    with ThreadPoolExecutor(max_workers=len(pairs)) as pool:
        return list(pool.map(register, pairs))


class TestRegisterEncryptedRace:

    def test_every_concurrent_registration_of_one_hash_answers_the_winner(self, user_client):
        for _ in range(ROUNDS):
            photo_hash = _short_hash(generate_random_bytes(4096))
            pairs = [_upload_pair(user_client, photo_hash) for _ in range(CONCURRENT)]

            responses = _register_all_at_once(user_client, photo_hash, pairs)

            statuses = [r.status_code for r in responses]
            assert all(s in (200, 201) for s in statuses), (
                f"a losing registration must not surface the UNIQUE violation: {statuses} "
                f"{[r.text for r in responses if r.status_code not in (200, 201)]}"
            )
            created = [r for r in responses if r.status_code == 201]
            assert len(created) == 1, f"exactly one registration creates the photo: {statuses}"

            winner_id = created[0].json()["photo_id"]
            winner_pair = pairs[responses.index(created[0])]
            for r in responses:
                if r.status_code != 200:
                    continue
                body = r.json()
                assert body["duplicate"] is True, body
                assert body["photo_id"] == winner_id, body
                assert body["encrypted_blob_id"] == winner_pair[0], (
                    "a duplicate answer must name the winner's canonical blob", body)
                assert body["encrypted_thumb_blob_id"] == winner_pair[1], body

    def test_a_sequential_duplicate_still_answers_the_existing_photo(self, user_client):
        """The non-racing path keeps its answer (guards the refactor of the insert)."""
        photo_hash = _short_hash(generate_random_bytes(4096))
        first, second = (_upload_pair(user_client, photo_hash) for _ in range(2))

        a = _register_all_at_once(user_client, photo_hash, [first])[0]
        b = _register_all_at_once(user_client, photo_hash, [second])[0]

        assert a.status_code == 201, a.text
        assert b.status_code == 200, b.text
        assert b.json()["photo_id"] == a.json()["photo_id"]
        assert b.json()["encrypted_blob_id"] == first[0]
