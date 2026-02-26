#!/usr/bin/env bash
set -e

# Resolve project root relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$SCRIPT_DIR/server"

echo "=== Simple Photos Server Reset ==="

# Kill any running server
echo "Stopping server..."
pkill -f simple-photos-server 2>/dev/null && sleep 1 || true

# Wipe database and storage
echo "Wiping database and storage..."
rm -f "$SERVER_DIR/data/db/"*
rm -rf "$SERVER_DIR/data/storage/"*/*

echo "Data cleared."

# Restart server
echo "Starting server..."
cd "$SERVER_DIR" && ./target/release/simple-photos-server &
sleep 2

# Verify setup state
STATUS=$(curl -s http://localhost:8080/api/setup/status)
echo "Setup status: $STATUS"
echo "=== Reset complete ==="
