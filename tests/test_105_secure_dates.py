"""
Test 105: secure-gallery photos sort by their date, and the automatic date
repair can find secured photos stuck at 1970.

Why this exists (TODO.md §4c, P2 — 2026-11-26)
----------------------------------------------
After the P1 repair moved 4,900 downloads off 1970-01-01, the owner reported the
secure gallery was still wrong. Measured on the live box:

  * both secure feeds returned no date and ``ORDER BY gi.added_at DESC`` — a batch
    secured while its dates were wrong kept that order whatever the dates became;
  * 104 secured photos were still at 1970, because the phone finds candidates
    through ``encrypted-sync``, which hides secure photos by design.

So: the feeds now sort by the photo's date and publish it as ``taken_at``, and
``GET /api/photos/dates/needs-repair`` lists the caller's client rows at 1970 —
secure ones included — for the phone's automatic repair.
"""

from helpers import generate_random_bytes
from conftest import USER_PASSWORD

EPOCH = "1970-01-01T00:00:00.000Z"


def _register(client, filename, taken_at):
    """The phone's call order: thumb blob, media blob, register."""
    thumb = client.upload_blob("thumbnail", generate_random_bytes(256))["blob_id"]
    media = client.upload_blob("photo", generate_random_bytes(2048))["blob_id"]
    r = client.post("/api/photos/register-encrypted", json_data={
        "filename": filename,
        "mime_type": "image/jpeg",
        "media_type": "photo",
        "width": 100,
        "height": 100,
        "taken_at": taken_at,
        "encrypted_blob_id": media,
        "encrypted_thumb_blob_id": thumb,
    })
    assert r.status_code == 201, r.text
    return {"photo_id": r.json()["photo_id"], "blob": media}


def _secure(client, name, photos):
    token = client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
    gid = client.create_secure_gallery(name)["gallery_id"]
    for p in photos:  # added in THIS order — the order the grid used to show
        client.add_secure_gallery_item(gid, p["blob"], token)
    return gid, token


def _aggregate(client, token):
    r = client.get("/api/galleries/secure/items", headers={"x-gallery-token": token})
    r.raise_for_status()
    return r.json()["items"]


def test_secure_items_sort_by_photo_date_not_add_order(user_client):
    # Secured oldest-date-last, like a 1970-sorted batch: add order is new, mid, old.
    new = _register(user_client, "new.jpg", "2025-08-05T00:00:00.000Z")
    old = _register(user_client, "old.jpg", "2019-05-01T00:00:00.000Z")
    mid = _register(user_client, "mid.jpg", "2022-01-01T00:00:00.000Z")
    gid, token = _secure(user_client, "P2 order", [old, mid, new])

    items = user_client.list_secure_gallery_items(gid, token)["items"]
    assert [i["taken_at"] for i in items] == [
        "2025-08-05T00:00:00.000Z", "2022-01-01T00:00:00.000Z", "2019-05-01T00:00:00.000Z",
    ], items

    agg = [i for i in _aggregate(user_client, token) if i["gallery_id"] == gid]
    assert [i["taken_at"] for i in agg] == [i["taken_at"] for i in items], \
        "both feeds must agree on order and date"


def test_a_repaired_secure_photo_moves_in_the_secure_grid(user_client):
    stuck = _register(user_client, "tiktok_1.mp4", EPOCH)
    dated = _register(user_client, "dated.jpg", "2021-01-01T00:00:00.000Z")
    gid, token = _secure(user_client, "P2 repair", [dated, stuck])

    before = user_client.list_secure_gallery_items(gid, token)["items"]
    assert before[-1]["taken_at"] == EPOCH, "precondition: the 1970 photo sorts last"

    listed = user_client.get("/api/photos/dates/needs-repair")
    assert listed.status_code == 200, listed.text
    by_id = {p["id"]: p for p in listed.json()["photos"]}
    assert stuck["photo_id"] in by_id, "a SECURED 1970 photo must be listed"
    assert dated["photo_id"] not in by_id
    assert by_id[stuck["photo_id"]]["taken_at"] == EPOCH

    r = user_client.post("/api/photos/dates/repair", json_data={
        "dry_run": False,
        "items": [{"photo_id": stuck["photo_id"], "taken_at": "2025-09-01T00:00:00Z",
                   "expect_taken_at": by_id[stuck["photo_id"]]["taken_at"]}],
    })
    assert r.status_code == 200, r.text
    assert r.json()["counts"]["change"] == 1, r.json()

    after = user_client.list_secure_gallery_items(gid, token)["items"]
    assert after[0]["taken_at"] == "2025-09-01T00:00:00.000Z", after
    listed_after = user_client.get("/api/photos/dates/needs-repair").json()["photos"]
    assert stuck["photo_id"] not in {p["id"] for p in listed_after}


def test_needs_repair_skips_other_users_and_server_files(user_client, second_user_client):
    theirs = _register(second_user_client, "theirs.jpg", EPOCH)
    up = user_client.upload_photo(filename="server_side.jpg")
    listed = {p["id"] for p in user_client.get("/api/photos/dates/needs-repair").json()["photos"]}
    assert theirs["photo_id"] not in listed
    assert up["photo_id"] not in listed
