#!/usr/bin/env bash
set -e

# Resolve project root relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$SCRIPT_DIR/server"

# Determine the real (non-root) user so we can restart the server unprivileged.
# When run via `sudo`, SUDO_USER holds the invoking user; otherwise fall back to
# the current user.
RUN_USER="${SUDO_USER:-$(id -un)}"
RUN_HOME=$(eval echo "~$RUN_USER")

# Locate cargo — it's typically in ~/.cargo/bin which sudo strips from PATH
CARGO_BIN="${RUN_HOME}/.cargo/bin/cargo"
if [[ ! -x "$CARGO_BIN" ]]; then
    CARGO_BIN=$(sudo -u "$RUN_USER" bash -c 'source ~/.cargo/env 2>/dev/null; which cargo 2>/dev/null' || true)
fi
if [[ -z "$CARGO_BIN" || ! -x "$CARGO_BIN" ]]; then
    CARGO_BIN="cargo"  # last resort: hope it's on PATH
fi

echo "=== Simple Photos Server Reset ==="

# ── Rebuild web frontend ─────────────────────────────────────────────────────
echo "Building web frontend..."
WEB_DIR="$SCRIPT_DIR/web"
if [[ -d "$WEB_DIR" ]]; then
    # Drop privileges when running as root via sudo
    if [[ "$RUN_USER" != "$(id -un)" ]]; then
        sudo -u "$RUN_USER" bash -c "cd '$WEB_DIR' && npm run build" \
            || { echo "WARNING: Web frontend build failed — continuing with existing dist"; }
    else
        (cd "$WEB_DIR" && npm run build) \
            || { echo "WARNING: Web frontend build failed — continuing with existing dist"; }
    fi
    # Ensure dist files are world-readable so Docker bind-mount containers
    # (which run as a different uid) can serve them.
    chmod -R a+r "$WEB_DIR/dist" 2>/dev/null || true
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
        cp "$APK_SRC" "$DOWNLOADS_DIR/simple-photos.apk" 2>/dev/null \
            || { echo "WARNING: Could not copy APK to $DOWNLOADS_DIR (permission denied) — continuing"; }
        [[ -f "$DOWNLOADS_DIR/simple-photos.apk" ]] && echo "Android APK copied to $DOWNLOADS_DIR/simple-photos.apk"
    else
        echo "WARNING: APK not found at $APK_SRC after build"
    fi
else
    echo "WARNING: $ANDROID_DIR not found — skipping Android build"
fi

# ── Rebuild server binary ────────────────────────────────────────────────────
echo "Building server binary..."
if [[ "$RUN_USER" != "$(id -un)" ]]; then
    sudo -u "$RUN_USER" bash -c "cd '$SERVER_DIR' && '$CARGO_BIN' build --release" \
        || { echo "ERROR: Server build failed. Aborting reset."; exit 1; }
else
    (cd "$SERVER_DIR" && "$CARGO_BIN" build --release) \
        || { echo "ERROR: Server build failed. Aborting reset."; exit 1; }
fi
echo "Server binary built."

# Kill any running server (root or user-owned)
echo "Stopping server..."
pkill -9 -f simple-photos-server 2>/dev/null && sleep 2 || true

# Wipe database
echo "Wiping database..."
rm -f "$SERVER_DIR/data/db/"*

# Clean server-managed subdirectories under internal storage, preserving any
# manually placed test/sample files (e.g. existing.jpg in the root).
echo "Cleaning internal storage (server-managed dirs only)..."
for subdir in blobs metadata logs .thumbnails .renders .tmp .web_previews .converted uploads; do
    if [[ -d "$SERVER_DIR/data/storage/$subdir" ]]; then
        echo "  Removing $SERVER_DIR/data/storage/$subdir/..."
        rm -rf "$SERVER_DIR/data/storage/$subdir"
    fi
done

# Read storage root from config.toml (the external photo storage location)
CONFIG_FILE="$SERVER_DIR/config.toml"
STORAGE_ROOT=""
if [[ -f "$CONFIG_FILE" ]]; then
    STORAGE_ROOT=$(grep -E '^\s*root\s*=' "$CONFIG_FILE" | head -1 | sed 's/.*=\s*"\(.*\)"/\1/')
fi

# Clean server-managed subdirectories under the storage root, preserving user photos
if [[ -n "$STORAGE_ROOT" && -d "$STORAGE_ROOT" ]]; then
    echo "Cleaning storage root subdirectories in: $STORAGE_ROOT"
    for subdir in blobs metadata logs .thumbnails .renders .tmp .web_previews .converted uploads; do
        if [[ -d "$STORAGE_ROOT/$subdir" ]]; then
            echo "  Removing $STORAGE_ROOT/$subdir/..."
            rm -rf "$STORAGE_ROOT/$subdir"
        fi
    done
    echo "Original photos preserved."
else
    echo "Warning: Could not determine storage root from config — skipping external cleanup"
fi

# Ensure data directories are owned by the real user so the server can write
chown -R "$RUN_USER:$RUN_USER" "$SERVER_DIR/data" 2>/dev/null || true

echo "Data cleared."

# Reset Docker backup instance (backup-1 only)
DOCKER_DIR="$SCRIPT_DIR/docker-instances"
if [[ -d "$DOCKER_DIR" ]]; then
    echo "Resetting Docker backup instance..."
    BACKUP_DATA="$DOCKER_DIR/backup-1/data"
    if [[ -d "$BACKUP_DATA" ]]; then
        rm -rf "$BACKUP_DATA/db/"* "$BACKUP_DATA/storage/"* 2>/dev/null || true
        # The container runs as appuser (uid 999). Ensure the bind-mounted
        # data dirs are writable after the wipe recreates them as the host user.
        mkdir -p "$BACKUP_DATA/db" "$BACKUP_DATA/storage"
        chown -R 999:999 "$BACKUP_DATA" 2>/dev/null || chmod -R 777 "$BACKUP_DATA" 2>/dev/null || true
        echo "  backup-1 data wiped"
    fi

    # Start (or recreate) the container so it picks up the clean state.
    # --force-recreate ensures we start even when the container was stopped.
    if command -v docker &>/dev/null && [[ -f "$DOCKER_DIR/docker-compose.yml" ]]; then
        echo "Stopping any extra backup containers..."
        docker compose -f "$DOCKER_DIR/docker-compose.yml" stop backup-2 backup-3 2>/dev/null || true
        
        echo "Starting backup container..."
        docker compose -f "$DOCKER_DIR/docker-compose.yml" up -d --build --force-recreate backup-1 2>/dev/null || true

        # Wait for backup-1 to start accepting requests
        echo -n "Waiting for backup-1"
        BK1_READY=false
        for attempt in $(seq 1 20); do
            if curl -sf http://localhost:8081/health >/dev/null 2>&1; then
                echo " ready!"
                BK1_READY=true
                break
            fi
            echo -n "."
            sleep 1
        done

        if [[ "$BK1_READY" == "true" ]]; then
            echo "  backup-1 is running and waiting for setup (http://localhost:8081)"
        else
            echo "  Warning: backup-1 did not become healthy in time"
        fi
    fi
fi

# Restart server as the real user (not root)
LOG_FILE="${TMPDIR:-/tmp}/simple-photos-server.log"
echo "Starting server as $RUN_USER (log: $LOG_FILE)..."
if [[ "$RUN_USER" != "$(id -un)" ]]; then
    # We're root via sudo — drop privileges and cd to SERVER_DIR so relative
    # config paths (./data/db/...) resolve correctly.
    sudo -u "$RUN_USER" bash -c "cd '$SERVER_DIR' && setsid nohup ./target/release/simple-photos-server > '$LOG_FILE' 2>&1 &"
else
    cd "$SERVER_DIR"
    nohup ./target/release/simple-photos-server > "$LOG_FILE" 2>&1 &
    disown
fi

# Wait for server to become responsive (up to 10 seconds)
echo -n "Waiting for server"
for i in $(seq 1 10); do
    if curl -sf http://localhost:8080/api/setup/status > /dev/null 2>&1; then
        echo " ready!"
        break
    fi
    echo -n "."
    sleep 1
done

# Verify setup state
STATUS=$(curl -s http://localhost:8080/api/setup/status 2>/dev/null || echo '{"error":"server not responding"}')
if echo "$STATUS" | grep -q '"error"'; then
    echo "WARNING: Server may have failed to start. Check $LOG_FILE"
    tail -20 "$LOG_FILE" 2>/dev/null
fi
echo "Setup status: $STATUS"
echo "=== Reset complete ==="
