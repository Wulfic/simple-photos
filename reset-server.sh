#!/usr/bin/env bash
set -e

echo "=== Simple Photos Server Reset ==="

# Kill any running server
echo "Stopping server..."
pkill -f simple-photos-server 2>/dev/null && sleep 1 || true

# Wipe database and storage
echo "Wiping database and storage..."
rm -f ~/repos/simple-photos/server/data/db/*
rm -rf ~/repos/simple-photos/server/data/storage/*/*

echo "Data cleared."

# Restart server
echo "Starting server..."
cd ~/repos/simple-photos/server && ./target/release/simple-photos-server &
sleep 2

# Verify setup state
STATUS=$(curl -s http://localhost:8080/api/setup/status)
echo "Setup status: $STATUS"
echo "=== Reset complete ==="
