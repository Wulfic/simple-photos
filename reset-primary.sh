#!/usr/bin/env bash
set -e

# Reset the primary server only (wipe DB + storage, rebuild, restart).
# Leaves backup containers untouched.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$SCRIPT_DIR/server"

RUN_USER="${SUDO_USER:-$(id -un)}"
RUN_HOME=$(eval echo "~$RUN_USER")

CARGO_BIN="${RUN_HOME}/.cargo/bin/cargo"
if [[ ! -x "$CARGO_BIN" ]]; then
    CARGO_BIN=$(sudo -u "$RUN_USER" bash -c 'source ~/.cargo/env 2>/dev/null; which cargo 2>/dev/null' || true)
fi
if [[ -z "$CARGO_BIN" || ! -x "$CARGO_BIN" ]]; then
    CARGO_BIN="cargo"
fi

echo "=== Reset Primary Server (backup untouched) ==="

# ── Build web frontend ───────────────────────────────────────────────────────
echo "Building web frontend..."
WEB_DIR="$SCRIPT_DIR/web"
if [[ -d "$WEB_DIR" ]]; then
    if [[ "$RUN_USER" != "$(id -un)" ]]; then
        sudo -u "$RUN_USER" bash -c "cd '$WEB_DIR' && npm run build" \
            || { echo "WARNING: Web frontend build failed — continuing with existing dist"; }
    else
        (cd "$WEB_DIR" && npm run build) \
            || { echo "WARNING: Web frontend build failed — continuing with existing dist"; }
    fi
    echo "Web frontend built."
else
    echo "WARNING: $WEB_DIR not found — skipping web build"
fi

# ── Build Android APK ────────────────────────────────────────────────────────
echo "Building Android APK..."
ANDROID_DIR="$SCRIPT_DIR/android"
DOWNLOADS_DIR="$SCRIPT_DIR/downloads"
if [[ -d "$ANDROID_DIR" ]]; then
    mkdir -p "$DOWNLOADS_DIR"
    BUILD_CMD="cd '$ANDROID_DIR' && ./gradlew assembleDebug"
    if [[ "$RUN_USER" != "$(id -un)" ]]; then
        sudo -u "$RUN_USER" bash -c "$BUILD_CMD" \
            || { echo "WARNING: Android APK build failed — continuing without APK"; }
    else
        (eval "$BUILD_CMD") \
            || { echo "WARNING: Android APK build failed — continuing without APK"; }
    fi
    APK_SRC="$ANDROID_DIR/app/build/outputs/apk/debug/app-debug.apk"
    if [[ -f "$APK_SRC" ]]; then
        cp "$APK_SRC" "$DOWNLOADS_DIR/simple-photos.apk"
        echo "Android APK copied to $DOWNLOADS_DIR/simple-photos.apk"
    else
        echo "WARNING: APK not found at $APK_SRC after build"
    fi
else
    echo "WARNING: $ANDROID_DIR not found — skipping Android build"
fi

# ── Build server binary ─────────────────────────────────────────────────────
echo "Building server binary..."
if [[ "$RUN_USER" != "$(id -un)" ]]; then
    sudo -u "$RUN_USER" bash -c "cd '$SERVER_DIR' && '$CARGO_BIN' build --release" \
        || { echo "ERROR: Server build failed. Aborting."; exit 1; }
else
    (cd "$SERVER_DIR" && "$CARGO_BIN" build --release) \
        || { echo "ERROR: Server build failed. Aborting."; exit 1; }
fi
echo "Server binary built."

# ── Stop primary server ──────────────────────────────────────────────────────
echo "Stopping server..."
pkill -9 -f simple-photos-server 2>/dev/null && sleep 2 || true

# ── Wipe database ────────────────────────────────────────────────────────────
echo "Wiping database..."
rm -f "$SERVER_DIR/data/db/"*

# ── Clean server-managed storage dirs ────────────────────────────────────────
echo "Cleaning internal storage (server-managed dirs only)..."
for subdir in blobs metadata logs .thumbnails; do
    if [[ -d "$SERVER_DIR/data/storage/$subdir" ]]; then
        echo "  Removing $SERVER_DIR/data/storage/$subdir/..."
        rm -rf "$SERVER_DIR/data/storage/$subdir"
    fi
done

# Clean server-managed subdirectories under external storage root
CONFIG_FILE="$SERVER_DIR/config.toml"
STORAGE_ROOT=""
if [[ -f "$CONFIG_FILE" ]]; then
    STORAGE_ROOT=$(grep -E '^\s*root\s*=' "$CONFIG_FILE" | head -1 | sed 's/.*=\s*"\(.*\)"/\1/')
fi
if [[ -n "$STORAGE_ROOT" && -d "$STORAGE_ROOT" ]]; then
    echo "Cleaning storage root subdirectories in: $STORAGE_ROOT"
    for subdir in blobs metadata logs .thumbnails; do
        if [[ -d "$STORAGE_ROOT/$subdir" ]]; then
            echo "  Removing $STORAGE_ROOT/$subdir/..."
            rm -rf "$STORAGE_ROOT/$subdir"
        fi
    done
    echo "Original photos preserved."
else
    echo "Warning: Could not determine storage root from config — skipping external cleanup"
fi

chown -R "$RUN_USER:$RUN_USER" "$SERVER_DIR/data" 2>/dev/null || true
echo "Data cleared."

# ── Restart server ───────────────────────────────────────────────────────────
LOG_FILE="${TMPDIR:-/tmp}/simple-photos-server.log"
echo "Starting server as $RUN_USER (log: $LOG_FILE)..."
if [[ "$RUN_USER" != "$(id -un)" ]]; then
    sudo -u "$RUN_USER" bash -c "cd '$SERVER_DIR' && setsid nohup ./target/release/simple-photos-server > '$LOG_FILE' 2>&1 &"
else
    cd "$SERVER_DIR"
    nohup ./target/release/simple-photos-server > "$LOG_FILE" 2>&1 &
    disown
fi

echo -n "Waiting for server"
for i in $(seq 1 10); do
    if curl -sf http://localhost:8080/api/setup/status > /dev/null 2>&1; then
        echo " ready!"
        break
    fi
    echo -n "."
    sleep 1
done

STATUS=$(curl -s http://localhost:8080/api/setup/status 2>/dev/null || echo '{"error":"server not responding"}')
if echo "$STATUS" | grep -q '"error"'; then
    echo "WARNING: Server may have failed to start. Check $LOG_FILE"
    tail -20 "$LOG_FILE" 2>/dev/null
fi
echo "Setup status: $STATUS"
echo "=== Reset complete (backup containers untouched) ==="
