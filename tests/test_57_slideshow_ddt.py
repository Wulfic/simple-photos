"""
Test 57: Slideshow — Data-Driven Tests (DDT).

Parametrized tests verifying the server-side data behaviour that
underpins the client-side slideshow feature:
  - Blob type classification (photo, gif, video, audio)
  - Album photo listing returns all items regardless of blob type
  - Blob-type filtering via /api/blobs?blob_type=…
  - Empty album returns empty photo list
  - Mixed-media album ordering preserved (added_at ASC)

The slideshow itself is rendered entirely in the browser; these tests
confirm the APIs deliver correct data for client-side filtering.
"""

import pytest

from helpers import (
    APIClient,
    generate_test_jpeg,
    generate_test_gif,
    generate_random_bytes,
)


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload_blob(client: APIClient, blob_type: str, content: bytes = None) -> str:
    """Upload a blob with the given type and return its id."""
    blob = client.upload_blob(blob_type, content or generate_random_bytes(512))
    return blob["blob_id"]


# ══════════════════════════════════════════════════════════════════════
# DDT: Blob type round-trips correctly
# ══════════════════════════════════════════════════════════════════════

BLOB_TYPE_CASES = [
    pytest.param("photo", generate_test_jpeg, id="photo_type"),
    pytest.param("gif", generate_test_gif, id="gif_type"),
    pytest.param("video", generate_random_bytes, id="video_type"),
    pytest.param("audio", generate_random_bytes, id="audio_type"),
]


@pytest.mark.parametrize("blob_type,content_fn", BLOB_TYPE_CASES)
def test_blob_type_roundtrip(user_client, blob_type, content_fn):
    """Uploading a blob with a given blob_type stores and returns that type."""
    content = content_fn()
    blob = user_client.upload_blob(blob_type, content)
    blob_id = blob["blob_id"]

    # Retrieve via list and verify type
    listing = user_client.list_blobs(blob_type=blob_type)
    ids = [b["id"] for b in listing["blobs"]]
    assert blob_id in ids, f"Blob {blob_id} not found in blob_type={blob_type} listing"


# ══════════════════════════════════════════════════════════════════════
# DDT: Album photo listing returns all items (no type filtering)
# ══════════════════════════════════════════════════════════════════════

def test_album_lists_all_media_types(user_client):
    """A shared album listing returns every item regardless of blob type."""
    album = user_client.create_shared_album("Slideshow Mix")
    album_id = album["id"]

    # Upload one of each type and add to album
    photo_id = _upload_blob(user_client, "photo", generate_test_jpeg())
    gif_id = _upload_blob(user_client, "gif", generate_test_gif())
    video_id = _upload_blob(user_client, "video")
    audio_id = _upload_blob(user_client, "audio")

    for bid in [photo_id, gif_id, video_id, audio_id]:
        user_client.add_album_photo(album_id, bid, ref_type="blob")

    photos = user_client.list_album_photos(album_id)
    refs = [p["photo_ref"] for p in photos]

    assert photo_id in refs
    assert gif_id in refs
    assert video_id in refs
    assert audio_id in refs
    assert len(photos) == 4


# ══════════════════════════════════════════════════════════════════════
# DDT: Blob-type filter returns only matching items
# ══════════════════════════════════════════════════════════════════════

FILTER_CASES = [
    pytest.param("photo", id="filter_photo_only"),
    pytest.param("gif", id="filter_gif_only"),
    pytest.param("video", id="filter_video_only"),
    pytest.param("audio", id="filter_audio_only"),
]


@pytest.mark.parametrize("target_type", FILTER_CASES)
def test_blob_type_filter_excludes_others(user_client, target_type):
    """list_blobs(blob_type=X) returns only blobs of type X."""
    # Upload one of each type
    ids = {}
    for bt in ["photo", "gif", "video", "audio"]:
        content = generate_test_jpeg() if bt == "photo" else (
            generate_test_gif() if bt == "gif" else generate_random_bytes(256)
        )
        ids[bt] = _upload_blob(user_client, bt, content)

    listing = user_client.list_blobs(blob_type=target_type)
    returned_ids = {b["id"] for b in listing["blobs"]}

    # Target should be present
    assert ids[target_type] in returned_ids, (
        f"Expected blob {ids[target_type]} in {target_type} listing"
    )
    # Others should NOT be present
    for other_type, other_id in ids.items():
        if other_type != target_type:
            assert other_id not in returned_ids, (
                f"Blob {other_id} (type={other_type}) should not appear in "
                f"blob_type={target_type} listing"
            )


# ══════════════════════════════════════════════════════════════════════
# DDT: Empty album returns empty photo list
# ══════════════════════════════════════════════════════════════════════

def test_empty_album_no_photos(user_client):
    """An album with no items returns an empty list."""
    album = user_client.create_shared_album("Empty Album")
    photos = user_client.list_album_photos(album["id"])
    assert photos == []


# ══════════════════════════════════════════════════════════════════════
# DDT: Album with only videos returns no photos via blob-type filter
# ══════════════════════════════════════════════════════════════════════

def test_album_only_videos_no_photo_blobs(user_client):
    """When an album contains only videos, blob-type=photo listing excludes them."""
    album = user_client.create_shared_album("Video Only")
    album_id = album["id"]

    v1 = _upload_blob(user_client, "video")
    v2 = _upload_blob(user_client, "video")
    user_client.add_album_photo(album_id, v1, ref_type="blob")
    user_client.add_album_photo(album_id, v2, ref_type="blob")

    # Album lists both
    photos = user_client.list_album_photos(album_id)
    assert len(photos) == 2

    # But blob-type=photo does not include them
    photo_blobs = user_client.list_blobs(blob_type="photo")
    photo_ids = {b["id"] for b in photo_blobs["blobs"]}
    assert v1 not in photo_ids
    assert v2 not in photo_ids


# ══════════════════════════════════════════════════════════════════════
# DDT: Mixed-media album ordering preserved
# ══════════════════════════════════════════════════════════════════════

def test_mixed_media_album_order(user_client):
    """Album photo listing preserves insertion order (added_at ASC)."""
    album = user_client.create_shared_album("Ordered Mix")
    album_id = album["id"]

    ordered_ids = []
    for bt in ["photo", "video", "gif", "photo", "audio"]:
        content = (generate_test_jpeg() if bt == "photo"
                   else generate_test_gif() if bt == "gif"
                   else generate_random_bytes(256))
        bid = _upload_blob(user_client, bt, content)
        user_client.add_album_photo(album_id, bid, ref_type="blob")
        ordered_ids.append(bid)

    photos = user_client.list_album_photos(album_id)
    refs = [p["photo_ref"] for p in photos]
    assert refs == ordered_ids, (
        f"Album order not preserved.\nExpected: {ordered_ids}\nGot: {refs}"
    )


# ══════════════════════════════════════════════════════════════════════
# DDT: Photo count — photos + gifs only (slideshow-eligible)
# ══════════════════════════════════════════════════════════════════════

def test_album_photo_count_excludes_video_audio(user_client):
    """
    Slideshow shows only photos and GIFs.
    Verify that blob listing by type correctly counts slideshow-eligible items.
    """
    photo1 = _upload_blob(user_client, "photo", generate_test_jpeg())
    photo2 = _upload_blob(user_client, "photo", generate_test_jpeg())
    gif1 = _upload_blob(user_client, "gif", generate_test_gif())
    _upload_blob(user_client, "video")
    _upload_blob(user_client, "audio")

    # Photos listing
    photo_listing = user_client.list_blobs(blob_type="photo")
    photo_ids = {b["id"] for b in photo_listing["blobs"]}
    # GIF listing
    gif_listing = user_client.list_blobs(blob_type="gif")
    gif_ids = {b["id"] for b in gif_listing["blobs"]}

    slideshow_eligible = photo_ids | gif_ids
    assert photo1 in slideshow_eligible
    assert photo2 in slideshow_eligible
    assert gif1 in slideshow_eligible
    assert len(slideshow_eligible) >= 3  # at least our 3 uploads
