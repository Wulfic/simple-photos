"""
Test 67: Backup sync of extended metadata — tags, AI, geolocation, user
settings, and extended EXIF/subtype columns on the photos table.

Verifies that the new metadata-sync channel introduced to extend the
backup payload faithfully replicates EVERY data type produced by the
primary's enrichment pipeline, including data the backup server never
generates locally (AI face/object detection, reverse geocoding).

Strategy:
  1. Create one user on the primary, upload one photo.
  2. Insert synthetic AI / geo / tag / user-settings rows directly into
     the primary's SQLite DB and patch the photos row with extended
     EXIF + geo + subtype columns.  This bypasses the AI/geo runtimes
     (which need ONNX models / network) but exercises the exact same
     replication path on the wire.
  3. Trigger a backup sync.
  4. Read the backup's SQLite DB directly and assert that every row
     and column round-tripped intact.

Each table / column-group is its own DDT row so a single regression
points to the exact field that broke.
"""

import os
import sqlite3
import time
from datetime import datetime, timezone

import pytest
from helpers import (
    APIClient,
    generate_test_jpeg,
    random_username,
    trigger_and_wait,
    unique_filename,
)
from conftest import USER_PASSWORD


# ── Test fixtures (module-scoped — one sync round per module) ─────────

_state: dict = {}


@pytest.fixture(scope="module")
def populated_and_synced(primary_admin, primary_server, backup_server,
                         backup_configured):
    """
    One-time setup for the whole module:
      • create user, upload one photo on primary
      • inject extended metadata rows directly into primary DB
      • trigger sync to backup
      • return paths needed by the per-row tests
    """
    # ── User + photo ─────────────────────────────────────────────────
    username = random_username("ext_meta_")
    created = primary_admin.admin_create_user(username, USER_PASSWORD)
    user_id = created["user_id"]

    user = APIClient(primary_server.base_url)
    user.login(username, USER_PASSWORD)

    photo = user.upload_photo(unique_filename(), content=generate_test_jpeg(20, 20))
    photo_id = photo["photo_id"]

    # ── Tags via API (covers initial transfer + later round-trip) ────
    user.add_tag(photo_id, "vacation").raise_for_status()
    user.add_tag(photo_id, "beach").raise_for_status()

    # ── Direct DB injection on primary ───────────────────────────────
    now = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    pdb = sqlite3.connect(primary_server.db_path)
    try:
        pdb.execute(
            """UPDATE photos SET
                geo_city = ?, geo_state = ?, geo_country = ?, geo_country_code = ?,
                photo_year = ?, photo_month = ?,
                photo_subtype = ?, burst_id = ?,
                camera_make = ?, lens_model = ?, iso_speed = ?, f_number = ?,
                exposure_time = ?, focal_length = ?, flash = ?, white_balance = ?,
                exposure_program = ?, metering_mode = ?, orientation = ?,
                software = ?, artist = ?, copyright = ?, description = ?,
                user_comment = ?, color_space = ?, exposure_bias = ?,
                scene_type = ?, digital_zoom = ?, exif_overrides = ?
               WHERE id = ?""",
            (
                "Paris", "Île-de-France", "France", "FR",
                2025, 7,
                "panorama", "burst-abc-123",
                "CanonMake", "RF 24-70 f/2.8L", 200, 5.6,
                "1/250", 50.0, "off", "auto",
                "manual", "matrix", 1,
                "TestSoftware v1.0", "Test Artist", "© 2025 Test", "test description",
                "test user comment", "sRGB", -0.33,
                "directly photographed", 1.5, '{"Make":"CanonOverride"}',
                photo_id,
            ),
        )

        # face_clusters
        pdb.execute(
            "INSERT INTO face_clusters (id, user_id, label, representative, "
            "photo_count, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            (9001, user_id, "Alice", "rep-photo-id", 7, now, now),
        )
        # face_detections
        pdb.execute(
            "INSERT INTO face_detections (id, photo_id, user_id, cluster_id, "
            "bbox_x, bbox_y, bbox_w, bbox_h, confidence, embedding, created_at) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                7001, photo_id, user_id, 9001,
                0.10, 0.20, 0.30, 0.40, 0.95,
                bytes([1, 2, 3, 4, 5, 6, 7, 8]),
                now,
            ),
        )
        # object_detections
        pdb.execute(
            "INSERT INTO object_detections (id, photo_id, user_id, class_name, "
            "confidence, bbox_x, bbox_y, bbox_w, bbox_h, created_at) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                8001, photo_id, user_id, "dog",
                0.88, 0.05, 0.05, 0.50, 0.50, now,
            ),
        )
        # ai_processed_photos
        pdb.execute(
            "INSERT INTO ai_processed_photos (photo_id, user_id, processed_at) "
            "VALUES (?, ?, ?)",
            (photo_id, user_id, now),
        )
        # user_settings
        pdb.execute(
            "INSERT INTO user_settings (user_id, key, value, updated_at) "
            "VALUES (?, ?, ?, ?)",
            (user_id, "ai_enabled", "true", now),
        )
        pdb.execute(
            "INSERT INTO user_settings (user_id, key, value, updated_at) "
            "VALUES (?, ?, ?, ?)",
            (user_id, "geo_enabled", "false", now),
        )
        pdb.commit()
    finally:
        pdb.close()

    # ── Sync ─────────────────────────────────────────────────────────
    trigger_and_wait(primary_admin, backup_configured)
    # Allow the metadata sync (last phase) a moment to flush.
    time.sleep(1.0)

    _state["primary_db"] = primary_server.db_path
    _state["backup_db"] = backup_server.db_path
    _state["user_id"] = user_id
    _state["photo_id"] = photo_id
    return _state


def _backup_row(query, params=()):
    db = sqlite3.connect(f"file:{_state['backup_db']}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    try:
        return db.execute(query, params).fetchone()
    finally:
        db.close()


def _backup_rows(query, params=()):
    db = sqlite3.connect(f"file:{_state['backup_db']}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    try:
        return db.execute(query, params).fetchall()
    finally:
        db.close()


# ── DDT: extended photos columns ─────────────────────────────────────

PHOTO_COLUMN_CASES = [
    pytest.param("geo_city",         "Paris",                          id="geo_city"),
    pytest.param("geo_state",        "Île-de-France",                  id="geo_state"),
    pytest.param("geo_country",      "France",                         id="geo_country"),
    pytest.param("geo_country_code", "FR",                             id="geo_country_code"),
    pytest.param("photo_year",       2025,                             id="photo_year"),
    pytest.param("photo_month",      7,                                id="photo_month"),
    pytest.param("photo_subtype",    "panorama",                       id="photo_subtype"),
    pytest.param("burst_id",         "burst-abc-123",                  id="burst_id"),
    pytest.param("camera_make",      "CanonMake",                      id="camera_make"),
    pytest.param("lens_model",       "RF 24-70 f/2.8L",                id="lens_model"),
    pytest.param("iso_speed",        200,                              id="iso_speed"),
    pytest.param("f_number",         5.6,                              id="f_number"),
    pytest.param("exposure_time",    "1/250",                          id="exposure_time"),
    pytest.param("focal_length",     50.0,                             id="focal_length"),
    pytest.param("flash",            "off",                            id="flash"),
    pytest.param("white_balance",    "auto",                           id="white_balance"),
    pytest.param("exposure_program", "manual",                         id="exposure_program"),
    pytest.param("metering_mode",    "matrix",                         id="metering_mode"),
    pytest.param("orientation",      1,                                id="orientation"),
    pytest.param("software",         "TestSoftware v1.0",              id="software"),
    pytest.param("artist",           "Test Artist",                    id="artist"),
    pytest.param("copyright",        "© 2025 Test",                    id="copyright"),
    pytest.param("description",      "test description",               id="description"),
    pytest.param("user_comment",     "test user comment",              id="user_comment"),
    pytest.param("color_space",      "sRGB",                           id="color_space"),
    pytest.param("exposure_bias",    -0.33,                            id="exposure_bias"),
    pytest.param("scene_type",       "directly photographed",          id="scene_type"),
    pytest.param("digital_zoom",     1.5,                              id="digital_zoom"),
    pytest.param("exif_overrides",   '{"Make":"CanonOverride"}',       id="exif_overrides"),
]


@pytest.mark.parametrize("column,expected", PHOTO_COLUMN_CASES)
def test_photo_extended_column_synced(populated_and_synced, column, expected):
    """Every extended photos column must replicate to the backup."""
    row = _backup_row(
        f"SELECT {column} AS val FROM photos WHERE id = ?",
        (populated_and_synced["photo_id"],),
    )
    assert row is not None, f"photo {populated_and_synced['photo_id']} missing on backup"
    actual = row["val"]
    if isinstance(expected, float):
        assert abs(actual - expected) < 1e-6, (
            f"{column}: backup={actual!r} expected={expected!r}"
        )
    else:
        assert actual == expected, (
            f"{column}: backup={actual!r} expected={expected!r}"
        )


# ── DDT: extension tables ────────────────────────────────────────────

def _photo_tags_check(s):
    rows = _backup_rows(
        "SELECT tag FROM photo_tags WHERE photo_id = ? AND user_id = ? ORDER BY tag",
        (s["photo_id"], s["user_id"]),
    )
    return sorted(r["tag"] for r in rows)


def _face_cluster_check(s):
    row = _backup_row(
        "SELECT label, representative, photo_count FROM face_clusters WHERE id = ?",
        (9001,),
    )
    return row and (row["label"], row["representative"], row["photo_count"])


def _face_detection_check(s):
    row = _backup_row(
        "SELECT photo_id, user_id, cluster_id, bbox_x, bbox_y, bbox_w, bbox_h, "
        "confidence, embedding FROM face_detections WHERE id = ?",
        (7001,),
    )
    if row is None:
        return None
    return (
        row["photo_id"], row["user_id"], row["cluster_id"],
        row["bbox_x"], row["bbox_y"], row["bbox_w"], row["bbox_h"],
        row["confidence"], bytes(row["embedding"]) if row["embedding"] else None,
    )


def _object_detection_check(s):
    row = _backup_row(
        "SELECT photo_id, user_id, class_name, confidence, bbox_x, bbox_y, "
        "bbox_w, bbox_h FROM object_detections WHERE id = ?",
        (8001,),
    )
    if row is None:
        return None
    return (
        row["photo_id"], row["user_id"], row["class_name"], row["confidence"],
        row["bbox_x"], row["bbox_y"], row["bbox_w"], row["bbox_h"],
    )


def _ai_processed_check(s):
    row = _backup_row(
        "SELECT 1 AS one FROM ai_processed_photos WHERE photo_id = ? AND user_id = ?",
        (s["photo_id"], s["user_id"]),
    )
    return row is not None


def _user_settings_check(s):
    rows = _backup_rows(
        "SELECT key, value FROM user_settings WHERE user_id = ? ORDER BY key",
        (s["user_id"],),
    )
    return [(r["key"], r["value"]) for r in rows]


TABLE_CASES = [
    pytest.param(_photo_tags_check,    ["beach", "vacation"],            id="photo_tags"),
    pytest.param(_face_cluster_check,  ("Alice", "rep-photo-id", 7),     id="face_clusters"),
    pytest.param(_ai_processed_check,  True,                              id="ai_processed_photos"),
    pytest.param(_user_settings_check, [("ai_enabled", "true"),
                                        ("geo_enabled", "false")],        id="user_settings"),
]


@pytest.mark.parametrize("checker,expected", TABLE_CASES)
def test_extension_table_synced(populated_and_synced, checker, expected):
    """Every extension table must replicate to the backup."""
    actual = checker(populated_and_synced)
    assert actual == expected, f"got {actual!r} expected {expected!r}"


def test_object_detection_full_payload(populated_and_synced):
    """Object detection row replicates all bbox + class + confidence fields."""
    s = populated_and_synced
    actual = _object_detection_check(s)
    assert actual is not None, "object_detection 8001 missing on backup"
    photo_id, user_id, class_name, confidence, bx, by, bw, bh = actual
    assert photo_id == s["photo_id"]
    assert user_id == s["user_id"]
    assert class_name == "dog"
    assert abs(confidence - 0.88) < 1e-6
    assert (round(bx, 4), round(by, 4), round(bw, 4), round(bh, 4)) == (0.05, 0.05, 0.50, 0.50)


def test_face_detection_full_payload(populated_and_synced):
    """Face detection row replicates bbox + embedding bytes + cluster link."""
    s = populated_and_synced
    actual = _face_detection_check(s)
    assert actual is not None, "face_detection 7001 missing on backup"
    photo_id, user_id, cluster_id, bx, by, bw, bh, conf, embedding = actual
    assert photo_id == s["photo_id"]
    assert user_id == s["user_id"]
    assert cluster_id == 9001
    assert (round(bx, 4), round(by, 4), round(bw, 4), round(bh, 4)) == (0.10, 0.20, 0.30, 0.40)
    assert abs(conf - 0.95) < 1e-6
    assert embedding == bytes([1, 2, 3, 4, 5, 6, 7, 8])


# ── Pruning: a row deleted on primary must be removed on backup ──────

def test_metadata_pruning_after_delete(populated_and_synced, primary_admin,
                                       backup_configured):
    """
    Removing a tag on the primary, then re-syncing, must delete it on
    the backup (full-state semantics).
    """
    s = populated_and_synced

    # Sanity: vacation present right now
    pre = _photo_tags_check(s)
    assert "vacation" in pre

    # Delete on primary directly (avoid auth complexity for system rows).
    pdb = sqlite3.connect(s["primary_db"])
    try:
        pdb.execute(
            "DELETE FROM photo_tags WHERE photo_id = ? AND user_id = ? AND tag = ?",
            (s["photo_id"], s["user_id"], "vacation"),
        )
        pdb.commit()
    finally:
        pdb.close()

    trigger_and_wait(primary_admin, backup_configured)
    time.sleep(1.0)

    post = _photo_tags_check(s)
    assert "vacation" not in post, f"tag not pruned on backup: {post}"
    assert "beach" in post, "untouched tag must remain"
