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

# Wipe database and storage
echo "Wiping database and storage..."
rm -f "$SERVER_DIR/data/db/"*
rm -rf "$SERVER_DIR/data/storage/"*/*

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
