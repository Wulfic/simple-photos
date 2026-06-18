#!/usr/bin/env python3
"""Prove the geolocation pipeline end-to-end on THIS machine.

Spins up a throwaway server **from a copy of the already-built release binary**
(so it never fights a running instance for the exe lock and never needs a
rebuild), enables geolocation, uploads the full edge-case sample set from
``tests/geo_fixtures.py``, waits for the real offline geocoder to resolve, then
asserts that Trips, Memories, Locations and the negative/sentinel cases all
come out exactly as expected.

This is the honest answer to "memories aren't being created": if this prints
ALL CHECKS PASSED, the pipeline works and the real-world cause is one of the
preconditions (geo disabled, no GPS EXIF, thresholds, dataset, poll cycle) — not
the clustering code.

    python scripts/verify_geo_samples.py
    python scripts/verify_geo_samples.py --precise   # also hit live Nominatim

The server's own binary is reused as-is; nothing is rebuilt and the user's
running instance is left untouched.
"""

from __future__ import annotations

import argparse
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "tests"))

from helpers import APIClient, wait_for_server  # noqa: E402
from geo_fixtures import (  # noqa: E402
    GROUPS,
    EXPLICIT_DAYS,
    EXPECT_IMPORT_DATED_MEMORY_COUNT,
    EXPECT_IMPORT_DATED_TRIP_COUNT,
    EXPECT_LOCATION_CITY_COUNT,
    EXPECT_MEMORY_PHOTO_COUNTS_DATED,
    EXPECT_TRIP_PHOTO_COUNTS_DATED,
    memory_day,
    photo_bytes,
    total_photos,
)

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SERVER_DIR = os.path.join(REPO, "server")
EXE = os.path.join(SERVER_DIR, "target", "release",
                   "simple-photos-server" + (".exe" if os.name == "nt" else ""))
DATASET = os.path.join(SERVER_DIR, "data", "cities500.txt")

ADMIN_USER = "geo_verify_admin"
ADMIN_PASS = "VerifyGeo!2026xyz"


def _free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def _write_config(path: str, port: int, db_path: str, storage_root: str) -> None:
    esc = lambda p: p.replace("\\", "\\\\")
    cfg = f"""
[server]
host = "127.0.0.1"
port = {port}
base_url = "http://127.0.0.1:{port}"
trust_proxy = true
discovery_port = 0

[database]
path = "{esc(db_path)}"
max_connections = 4

[storage]
root = "{esc(storage_root)}"
default_quota_bytes = 0
max_blob_size_bytes = 104857600

[auth]
jwt_secret = "geo_verify_jwt_secret_must_be_at_least_32_characters_long"
access_token_ttl_secs = 86400
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 4

[web]
static_root = ""

[tls]
enabled = false

[scan]
auto_scan_interval_secs = 0

[geo]
enabled = true
dataset_path = "{esc(DATASET)}"
poll_interval_secs = 2
auto_download_dataset = false

[ai]
enabled = false
"""
    with open(path, "w") as f:
        f.write(cfg)


class _Check:
    def __init__(self) -> None:
        self.failures = 0

    def __call__(self, label: str, ok: bool, detail: str = "") -> None:
        mark = "PASS" if ok else "FAIL"
        print(f"  [{mark}] {label}" + (f"  ({detail})" if detail else ""))
        if not ok:
            self.failures += 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--precise", action="store_true",
                    help="also enable precise geocoding and check live street resolution")
    ap.add_argument("--keep", action="store_true", help="keep the temp dir + server log")
    args = ap.parse_args()

    if not os.path.isfile(EXE):
        print(f"ERROR: server binary not found at {EXE}\n"
              f"Build it first: (cd server && cargo build --release)")
        return 2
    if not os.path.isfile(DATASET):
        print(f"ERROR: GeoNames dataset not found at {DATASET}")
        return 2

    tmp = tempfile.mkdtemp(prefix="geo_verify_")
    db_dir = os.path.join(tmp, "db"); os.makedirs(db_dir)
    storage = os.path.join(tmp, "storage"); os.makedirs(storage)
    db_path = os.path.join(db_dir, "sp.db")
    cfg_path = os.path.join(tmp, "config.toml")
    log_path = os.path.join(tmp, "server.log")

    # Copy the binary so a running instance can't lock us out (and we never
    # have to rebuild). Copy is fine even while the original is executing.
    run_exe = os.path.join(tmp, os.path.basename(EXE))
    shutil.copy2(EXE, run_exe)

    port = _free_port()
    base_url = f"http://127.0.0.1:{port}"
    _write_config(cfg_path, port, db_path, storage)

    print(f"Launching throwaway server on {base_url}")
    print(f"  exe : {run_exe}")
    print(f"  data: {DATASET}\n")

    log = open(log_path, "w")
    proc = subprocess.Popen([run_exe], cwd=tmp, stdout=log, stderr=subprocess.STDOUT,
                            env={**os.environ, "SIMPLE_PHOTOS_CONFIG": cfg_path,
                                 "RUST_LOG": "info"})
    check = _Check()
    try:
        wait_for_server(base_url, timeout=30)
        client = APIClient(base_url)
        client.setup_init(ADMIN_USER, ADMIN_PASS)
        client.login(ADMIN_USER, ADMIN_PASS)
        client.geo_update_settings(enabled=True)
        if args.precise:
            client.geo_update_settings(precise_enabled=True)

        # Upload the whole edge-case set.
        seq = 0
        for g in GROUPS:
            for idx in range(g.count):
                client.upload_photo(filename=f"{g.key}_{seq}.jpg",
                                    content=photo_bytes(g, idx, seq, captioned=False))
                seq += 1
        print(f"Uploaded {seq} photos (expected {total_photos()}). "
              f"Waiting for the geocoder...\n")

        # Wait for resolution: locations stabilise at the expected city count.
        deadline = time.time() + 60
        locations = []
        while time.time() < deadline:
            locations = client.geo_locations()
            if len(locations) >= EXPECT_LOCATION_CITY_COUNT:
                break
            time.sleep(1.5)

        trips = client.get("/api/geo/trips").json()
        memories = client.get("/api/geo/memories").json()
        settings = client.geo_settings()

        # Split deterministic, explicitly-dated albums from the import-dated
        # no-date group (sydney) so wall-clock timing never makes this flaky.
        dated_trips = [t for t in trips if t["start_date"] in EXPLICIT_DAYS]
        import_trips = [t for t in trips if t["start_date"] not in EXPLICIT_DAYS]
        dated_mems = [m for m in memories if memory_day(m) in EXPLICIT_DAYS]
        import_mems = [m for m in memories if memory_day(m) not in EXPLICIT_DAYS]

        print("Results:")
        check("locations resolved (distinct cities)",
              len(locations) == EXPECT_LOCATION_CITY_COUNT,
              f"got {len(locations)}, want {EXPECT_LOCATION_CITY_COUNT}")
        check("dated trips (explicit-date groups)",
              sorted(t["photo_count"] for t in dated_trips) == EXPECT_TRIP_PHOTO_COUNTS_DATED,
              f"got {sorted(t['photo_count'] for t in dated_trips)}, "
              f"want {EXPECT_TRIP_PHOTO_COUNTS_DATED}")
        check("dated memories (explicit-date groups)",
              sorted(m["photo_count"] for m in dated_mems) == EXPECT_MEMORY_PHOTO_COUNTS_DATED,
              f"got {sorted(m['photo_count'] for m in dated_mems)}, "
              f"want {EXPECT_MEMORY_PHOTO_COUNTS_DATED}")
        check("no-date group dates to import time (1 extra trip)",
              len(import_trips) == 1 and import_trips[0]["photo_count"] == EXPECT_IMPORT_DATED_TRIP_COUNT,
              f"import trips={[t['photo_count'] for t in import_trips]}")
        check("no-date group dates to import time (1 extra memory)",
              len(import_mems) == 1 and import_mems[0]["photo_count"] == EXPECT_IMPORT_DATED_MEMORY_COUNT,
              f"import memories={[m['photo_count'] for m in import_mems]}")
        # 42 of 47 carry GPS; timeline_nogps (5) carry none.
        check("photos_with_location (GPS incl. ocean sentinel)",
              settings["photos_with_location"] == 42,
              f"got {settings['photos_with_location']}, want 42")
        check("photos_without_location (no-GPS group)",
              settings["photos_without_location"] == 5,
              f"got {settings['photos_without_location']}, want 5")
        check("unique_cities excludes the '' ocean sentinel",
              settings["unique_cities"] == EXPECT_LOCATION_CITY_COUNT,
              f"got {settings['unique_cities']}, want {EXPECT_LOCATION_CITY_COUNT}")

        if args.precise:
            # Give the rate-limited (1 req/s) precise backfill time to run.
            time.sleep(max(5, len(locations)) + 10)
            memories = client.get("/api/geo/memories").json()
            nyc = [m for m in memories if m["country"] in ("United States",)
                   and m["photo_count"] == 3]
            label = nyc[0]["name"] if nyc else "(none)"
            check("precise street address surfaced in NYC memory title",
                  any("Avenue" in m["name"] or "Street" in m["name"] for m in memories),
                  f"NYC memory title = {label!r}")

        print()
        if check.failures == 0:
            print("ALL CHECKS PASSED - the geo pipeline works end-to-end on this machine.")
            print("If memories don't appear in your app, it's a precondition: enable geo "
                  "in Settings, ensure photos have GPS EXIF + dates, and wait a poll cycle.")
        else:
            print(f"{check.failures} CHECK(S) FAILED - see server log: {log_path}")
        return 1 if check.failures else 0
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        log.close()
        if args.keep:
            print(f"\n(kept temp dir: {tmp})")
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
