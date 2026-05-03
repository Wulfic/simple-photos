#!/usr/bin/env bash
# Download the GeoNames cities500 dataset for offline reverse geocoding.
#
# After the fix for todo P0-4, the geo-processor refuses to silently run
# without this dataset — it emits an `error!` log and produces no geo_city /
# geo_country values for uploaded photos.  Run this script once at install
# time (or whenever you wipe `server/data/`).
#
# Output: $TARGET (default: server/data/cities500.txt)
#
# License: GeoNames data is licensed CC BY 4.0
# (https://creativecommons.org/licenses/by/4.0/) — attribute geonames.org.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-$REPO_ROOT/server/data/cities500.txt}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

URL="https://download.geonames.org/export/dump/cities500.zip"

mkdir -p "$(dirname "$TARGET")"

echo "[geo] Downloading GeoNames cities500 dataset → $TMPDIR/cities500.zip"
if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 3 -o "$TMPDIR/cities500.zip" "$URL"
elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$TMPDIR/cities500.zip" "$URL"
else
    echo "[geo] ERROR: neither curl nor wget is installed." >&2
    exit 1
fi

echo "[geo] Unzipping → $TARGET"
if command -v unzip >/dev/null 2>&1; then
    unzip -p "$TMPDIR/cities500.zip" cities500.txt > "$TARGET"
else
    echo "[geo] ERROR: unzip is not installed." >&2
    exit 1
fi

LINES="$(wc -l <"$TARGET" | tr -d ' ')"
echo "[geo] Done — $LINES cities written to $TARGET"
echo "[geo] Restart the server to pick up the dataset."
