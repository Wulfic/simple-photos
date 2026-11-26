"""
Test 104: batch photo-date repair — dry run by default, compare-and-set, undo.

Why this exists (TODO.md §4b, P1b — 2026-11-26)
-----------------------------------------------
5,003 of 5,146 Android-uploaded rows on the live box were stored at
1970-01-01: the phone read only MediaStore DATE_TAKEN, which is NULL for a
downloaded file. Only the phone knows the right date, so it sends corrections
to `POST /api/photos/dates/repair`. That endpoint rewrites thousands of rows the
owner can see, so every guard is pinned here over real HTTP:

  * no `dry_run` field ⇒ nothing is written, but the answer says what would be;
  * an empty list, an epoch date and a missing `expect_taken_at` are refused;
  * another user's photo answers `not_found` and is untouched;
  * a row changed since the phone looked is skipped (`stale`), not overwritten;
  * a server-file row (web upload / Takeout / scan) is `protected` — a phone can
    hold its id after a content-hash dedup, but its date came from the file;
  * an applied batch moves the row in encrypted-sync AND in the delta feed;
  * undo (also dry-run by default) puts the original value back, once.
"""

EPOCH = "1970-01-01T00:00:00.000Z"
LANDED = "2024-03-05T10:20:30.400Z"


def _register(client, filename, taken_at):
    thumb = client.upload_blob("thumbnail", b"t" * 256)["blob_id"]
    media = client.upload_blob("photo", filename.encode() * 64)["blob_id"]
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
    return r.json()["photo_id"]


def _taken_at(client, photo_id):
    for rec in client.encrypted_sync_all():
        if rec["id"] == photo_id:
            return rec["taken_at"]
    raise AssertionError(f"{photo_id} missing from encrypted-sync")


def _repair(client, items, **extra):
    return client.post("/api/photos/dates/repair", json_data={"items": items, **extra})


def _item(photo_id, expect=EPOCH, taken_at=LANDED):
    return {"photo_id": photo_id, "taken_at": taken_at, "expect_taken_at": expect}


def test_guards_refuse_bad_input_and_touch_nothing(user_client):
    pid = _register(user_client, "tumblr_guard.jpg", EPOCH)

    for body in (
        {"items": []},
        {"items": [_item(pid, taken_at=EPOCH)]},
        {"items": [{"photo_id": pid, "taken_at": LANDED}]},  # no expect_taken_at
        {"items": [_item(pid), _item(pid)]},
    ):
        r = user_client.post("/api/photos/dates/repair",
                             json_data={**body, "dry_run": False})
        assert r.status_code == 400, (body, r.status_code, r.text)

    assert _taken_at(user_client, pid) == EPOCH


def test_dry_run_is_the_default_and_writes_nothing(user_client):
    pid = _register(user_client, "tumblr_dry.jpg", EPOCH)
    head = user_client.encrypted_sync(since=0, limit=1)["head_seq"]

    r = _repair(user_client, [_item(pid)])
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["dry_run"] is True
    assert body["batch_id"] is None
    assert body["counts"]["change"] == 1
    assert body["items"][0]["outcome"] == "change"

    assert _taken_at(user_client, pid) == EPOCH
    delta = user_client.encrypted_sync(since=head)
    assert pid not in [p["id"] for p in delta["photos"]], "a dry run must not nominate"


def test_another_users_photo_is_not_found(user_client, second_user_client):
    theirs = _register(second_user_client, "tumblr_theirs.jpg", EPOCH)

    r = _repair(user_client, [_item(theirs)], dry_run=False)
    assert r.status_code == 200, r.text
    assert r.json()["counts"] == {"change": 0, "unchanged": 0, "stale": 0, "not_found": 1, "protected": 0}
    assert r.json()["batch_id"] is None
    assert _taken_at(second_user_client, theirs) == EPOCH


def test_apply_moves_the_row_skips_stale_and_undo_restores(user_client):
    bad = _register(user_client, "tumblr_bad.jpg", EPOCH)
    edited = _register(user_client, "tumblr_edited.jpg", "2025-06-01T00:00:00.000Z")
    head = user_client.encrypted_sync(since=0, limit=1)["head_seq"]

    r = _repair(user_client, [_item(bad), _item(edited)], dry_run=False)
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["dry_run"] is False
    assert body["counts"] == {"change": 1, "unchanged": 0, "stale": 1, "not_found": 0, "protected": 0}
    batch = body["batch_id"]
    assert batch

    assert _taken_at(user_client, bad) == LANDED
    assert _taken_at(user_client, edited) == "2025-06-01T00:00:00.000Z"
    delta = user_client.encrypted_sync(since=head)
    moved = {p["id"]: p["taken_at"] for p in delta["photos"]}
    assert moved.get(bad) == LANDED, "the delta feed must carry the repaired date"

    # Re-sending the same batch is idempotent: the row already holds it.
    again = _repair(user_client, [_item(bad)], dry_run=False).json()
    assert again["counts"]["unchanged"] == 1 and again["batch_id"] is None

    # Undo: dry run first (the default), then for real, then never twice.
    dry = user_client.post("/api/photos/dates/repair/undo", json_data={"batch_id": batch})
    assert dry.status_code == 200, dry.text
    assert dry.json()["dry_run"] is True and dry.json()["counts"]["change"] == 1
    assert _taken_at(user_client, bad) == LANDED

    done = user_client.post("/api/photos/dates/repair/undo",
                            json_data={"batch_id": batch, "dry_run": False})
    assert done.status_code == 200, done.text
    assert done.json()["counts"]["change"] == 1
    assert _taken_at(user_client, bad) == EPOCH

    twice = user_client.post("/api/photos/dates/repair/undo",
                             json_data={"batch_id": batch, "dry_run": False})
    assert twice.status_code == 404


def test_undo_refuses_an_empty_or_foreign_batch(user_client, second_user_client):
    pid = _register(user_client, "tumblr_mine.jpg", EPOCH)
    batch = _repair(user_client, [_item(pid)], dry_run=False).json()["batch_id"]

    empty = user_client.post("/api/photos/dates/repair/undo",
                             json_data={"batch_id": "  ", "dry_run": False})
    assert empty.status_code == 400

    foreign = second_user_client.post("/api/photos/dates/repair/undo",
                                      json_data={"batch_id": batch, "dry_run": False})
    assert foreign.status_code == 404
    assert _taken_at(user_client, pid) == LANDED


def test_a_server_file_row_is_protected(user_client):
    """A web upload stores a real `file_path`; its date is the server's to keep."""
    up = user_client.upload_photo(filename="server_side.jpg")
    pid = up["photo_id"]
    assert up["file_path"], up

    def stored():
        # Not via encrypted-sync: an unencrypted upload may not be in that feed yet.
        r = user_client.get(f"/api/photos/{pid}/metadata/full")
        assert r.status_code == 200, r.text
        return r.json()["taken_at"]

    before = stored()

    r = _repair(user_client, [_item(pid, expect=before)], dry_run=False)
    assert r.status_code == 200, r.text
    assert r.json()["counts"]["protected"] == 1, r.json()
    assert r.json()["batch_id"] is None
    assert stored() == before
