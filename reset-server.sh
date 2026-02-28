#!/usr/bin/env bash
set -e

# Resolve project root relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$SCRIPT_DIR/server"

# Determine the real (non-root) user so we can restart the server unprivileged.
# When run via `sudo`, SUDO_USER holds the invoking user; otherwise fall back to
# the current user.
RUN_USER="${SUDO_USER:-$(id -un)}"

echo "=== Simple Photos Server Reset ==="

# Kill any running server (root or user-owned)
echo "Stopping server..."
pkill -9 -f simple-photos-server 2>/dev/null && sleep 2 || true

# Wipe database
echo "Wiping database..."
rm -f "$SERVER_DIR/data/db/"*

# Wipe internal storage
rm -rf "$SERVER_DIR/data/storage/"*/*

# Read storage root from config.toml (the external photo storage location)
CONFIG_FILE="$SERVER_DIR/config.toml"
STORAGE_ROOT=""
if [[ -f "$CONFIG_FILE" ]]; then
    STORAGE_ROOT=$(grep -E '^\s*root\s*=' "$CONFIG_FILE" | head -1 | sed 's/.*=\s*"\(.*\)"/\1/')
fi

# Clean server-managed subdirectories under the storage root, preserving user photos
if [[ -n "$STORAGE_ROOT" && -d "$STORAGE_ROOT" ]]; then
    echo "Cleaning storage root subdirectories in: $STORAGE_ROOT"
    for subdir in blobs metadata logs uploads; do
        if [[ -d "$STORAGE_ROOT/$subdir" ]]; then
            echo "  Removing $STORAGE_ROOT/$subdir/..."
            rm -rf "$STORAGE_ROOT/$subdir"
        fi
    done
else
    echo "Warning: Could not determine storage root from config — skipping external cleanup"
fi

# Ensure data directories are owned by the real user so the server can write
chown -R "$RUN_USER:$RUN_USER" "$SERVER_DIR/data" 2>/dev/null || true

echo "Data cleared."

# Restart server as the real user (not root)
echo "Starting server as $RUN_USER..."
if [[ "$RUN_USER" != "$(id -un)" ]]; then
    # We're root via sudo — drop privileges. Use nohup + setsid so the server
    # survives after this script exits.
    sudo -u "$RUN_USER" setsid nohup "$SERVER_DIR/target/release/simple-photos-server" \
        > /dev/null 2>&1 &
else
    cd "$SERVER_DIR"
    nohup ./target/release/simple-photos-server > /dev/null 2>&1 &
    disown
fi
sleep 3

# Verify setup state
STATUS=$(curl -s http://localhost:8080/api/setup/status 2>/dev/null || echo '{"error":"server not responding"}')
echo "Setup status: $STATUS"
echo "=== Reset complete ==="
