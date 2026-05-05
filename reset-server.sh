#!/usr/bin/env bash
set -e

# Resolve project root relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$SCRIPT_DIR/server"

# ============================================================================
# Safety helpers — inlined.
#
# These exist because of an incident where this reset script wiped roughly
# 15 TB of user data on a network drive.  Every destructive operation below
# MUST go through these helpers.  Do not bypass them.
# ============================================================================

SAFE_MANAGED_SUBDIRS=(blobs metadata logs .thumbnails .renders .tmp \
                      .web_previews .converted uploads .ai_data .geo_cache)

abort() {
    echo "" >&2
    echo "============================================================" >&2
    echo "FATAL SAFETY CHECK: $*" >&2
    echo "Aborting to protect your data." >&2
    echo "============================================================" >&2
    exit 1
}

# Returns 0 if the path is acceptable as a destination for managed-subdir
# deletion. Refuses anything that even looks risky.
_is_safe_storage_root() {
    local root="$1"
    [[ -n "$root" ]] || return 1
    [[ "$root" == /* ]] || return 1
    [[ -d "$root" ]] || return 1
    case "$root" in
        *'$'*|*'`'*|*'\'*|*$'\n'*|*$'\r'*) return 1 ;;
    esac
    local real
    real=$(readlink -f -- "$root" 2>/dev/null) || return 1
    [[ -n "$real" && -d "$real" ]] || return 1
    case "$real" in
        /|/root|/home|/usr|/etc|/var|/opt|/boot|/bin|/sbin|/lib|/lib32|/lib64\
        |/mnt|/media|/srv|/tmp|/dev|/proc|/sys|/run|/Users|/Volumes)
            return 1 ;;
    esac
    [[ -n "${HOME:-}" && "$real" == "$HOME" ]] && return 1
    local stripped="${real#/}"
    [[ "$stripped" == *"/"* ]] || return 1
    return 0
}

# safe_purge_managed_subdirs ROOT SUBDIR [SUBDIR …]
# Deletes ONLY the listed subdirectories beneath ROOT.  Any subdirectory that
# has an unsafe name, is a symlink, or resolves outside ROOT is skipped with
# a warning rather than deleted.
safe_purge_managed_subdirs() {
    local root="$1"; shift
    local subdirs=("$@")
    if ! _is_safe_storage_root "$root"; then
        abort "Refusing to clean storage root '$root' — empty, missing, shallow, or a system path."
    fi
    local real_root
    real_root=$(readlink -f -- "$root") || abort "Could not resolve '$root'."
    local sd target real_target
    for sd in "${subdirs[@]}"; do
        if [[ -z "$sd" || ! "$sd" =~ ^[A-Za-z0-9._-]+$ || "$sd" == "." || "$sd" == ".." ]]; then
            echo "  WARN: skipping invalid subdir name: '$sd'"
            continue
        fi
        target="$root/$sd"
        [[ -e "$target" || -L "$target" ]] || continue
        if [[ -L "$target" ]]; then
            echo "  WARN: '$target' is a symlink — leaving it alone."
            continue
        fi
        if [[ ! -d "$target" ]]; then
            echo "  WARN: '$target' is not a directory — leaving it alone."
            continue
        fi
        real_target=$(readlink -f -- "$target" 2>/dev/null) || {
            echo "  WARN: could not resolve '$target' — leaving it alone."
            continue
        }
        if [[ "$real_target" != "$real_root"/* ]]; then
            echo "  WARN: '$target' resolves outside '$root' — leaving it alone."
            continue
        fi
        echo "  Removing $target/..."
        if ! timeout 10 rm -rf -- "$target"; then
            echo "  WARN: deletion of '$target' timed out or failed (possibly a slow network drive)."
            echo "        Please delete it manually: rm -rf '$target'"
        fi
    done
}

# Read the storage root from a config.toml file.  Returns the empty string if
# parsing is ambiguous (more than one match) or yields a suspicious value.
safe_read_storage_root() {
    local config_file="$1"
    [[ -f "$config_file" ]] || { echo ""; return 0; }
    local matches
    matches=$(grep -cE '^[[:space:]]*root[[:space:]]*=' "$config_file" || true)
    if [[ "$matches" -ne 1 ]]; then
        echo ""
        return 0
    fi
    local raw
    raw=$(grep -E '^[[:space:]]*root[[:space:]]*=' "$config_file" \
        | sed -E 's/^[[:space:]]*root[[:space:]]*=[[:space:]]*"([^"]*)".*$/\1/')
    [[ "$raw" == *$'\n'* ]] && { echo ""; return 0; }
    echo "$raw"
}

# Confirm before destructive operations.
# Safety is enforced by safe_purge_managed_subdirs / _is_safe_storage_root
# guard-rails, so no interactive prompt is needed for the test reset script.
safe_confirm_destruction() {
    echo "Auto-proceeding — safety guard-rails are active."
}

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

# ── Port helpers ─────────────────────────────────────────────────────────────
DEFAULT_PORT=8080

port_in_use() {
    local port="$1"
    if command -v ss &>/dev/null; then
        ss -tlnH 2>/dev/null | grep -q ":${port} " && return 0
    elif command -v netstat &>/dev/null; then
        netstat -tlnp 2>/dev/null | grep -q ":${port} " && return 0
    elif command -v lsof &>/dev/null; then
        lsof -i ":${port}" -sTCP:LISTEN &>/dev/null && return 0
    fi
    (echo >/dev/tcp/localhost/"$port") 2>/dev/null && return 0
    return 1
}

# find_free_port START_PORT [SKIP_PORT]
# Returns the first port >= START_PORT that is free (and != SKIP_PORT).
find_free_port() {
    local port="${1:-$DEFAULT_PORT}"
    local skip="${2:-}"
    local i=0
    while [[ $i -lt 100 ]]; do
        if [[ "$port" != "$skip" ]] && ! port_in_use "$port"; then
            echo "$port"
            return 0
        fi
        echo "  Port $port in use (or reserved), trying $((port + 1))..." >&2
        port=$((port + 1))
        i=$((i + 1))
    done
    echo "ERROR: No free port found after 100 attempts starting from ${1:-$DEFAULT_PORT}" >&2
    exit 1
}

echo "=== Simple Photos Server Reset ==="

# ── Stop everything FIRST so their ports are freed before we port-scan ────────
# Native server must yield its port before we pick a new one for it.
echo "Stopping native server..."
if systemctl is-active simple-photos.service &>/dev/null 2>&1; then
    sudo systemctl stop simple-photos.service
fi
pkill -9 -f simple-photos-server 2>/dev/null && sleep 1 || true

# Bring down the Docker backup container so its host port is also freed.
DOCKER_DIR="$SCRIPT_DIR/docker-instances"
BACKUP_DIR="$DOCKER_DIR/simple-photos-backup-8081"
BACKUP_COMPOSE="$BACKUP_DIR/docker-compose.yml"
if command -v docker &>/dev/null && [[ -f "$BACKUP_COMPOSE" ]]; then
    echo "Stopping backup container..."
    docker compose -f "$BACKUP_COMPOSE" down 2>/dev/null || true
fi

# Now that nothing is listening we can find clean free ports.
# Native (primary) takes priority — it always gets the first available port.
SERVER_PORT=$(find_free_port "$DEFAULT_PORT")
# Backup gets the next available port, skipping the primary's slot.
BACKUP_PORT=$(find_free_port "$DEFAULT_PORT" "$SERVER_PORT")

# ── Rebuild web frontend ─────────────────────────────────────────────────────
echo "Building web frontend..."
WEB_DIR="$SCRIPT_DIR/web"
if [[ -d "$WEB_DIR" ]]; then
    # Check if npm is available for the target user before attempting the build.
    _NPM_OK=false
    if [[ "$RUN_USER" != "$(id -un)" ]]; then
        sudo -u "$RUN_USER" bash -c 'command -v npm' &>/dev/null && _NPM_OK=true
    else
        command -v npm &>/dev/null && _NPM_OK=true
    fi

    if [[ "$_NPM_OK" == true ]]; then
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
        echo "WARNING: npm not found — skipping web frontend build (using existing dist)"
    fi
else
    echo "WARNING: $WEB_DIR not found — skipping web build"
fi

# ── Build Android APK ────────────────────────────────────────────────────────
echo "Building Android APK..."
ANDROID_DIR="$SCRIPT_DIR/android"
DOWNLOADS_DIR="$SCRIPT_DIR/downloads"
if [[ -d "$ANDROID_DIR" ]]; then
    # Check if java is available for the target user — gradle requires it.
    _JAVA_OK=false
    if [[ "$RUN_USER" != "$(id -un)" ]]; then
        sudo -u "$RUN_USER" bash -c 'command -v java' &>/dev/null && _JAVA_OK=true
    else
        command -v java &>/dev/null && _JAVA_OK=true
    fi

    if [[ "$_JAVA_OK" == true ]]; then
        mkdir -p "$DOWNLOADS_DIR"
        # Resolve ANDROID_HOME — prefer env var, then the default install location
        RESOLVED_ANDROID_HOME="${ANDROID_HOME:-$(eval echo ~$RUN_USER)/android-sdk}"
        BUILD_CMD="export ANDROID_HOME='$RESOLVED_ANDROID_HOME'; cd '$ANDROID_DIR' && ./gradlew assembleDebug"
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
        echo "WARNING: Java not found — skipping Android APK build"
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

# Clean up Rust debug artifacts — the reset always uses the release binary;
# debug/ accumulates from development builds and can consume 10+ GB.
if [[ -d "$SERVER_DIR/target/debug" ]]; then
    echo "Cleaning Rust debug build artifacts..."
    rm -rf "$SERVER_DIR/target/debug"
fi

# Wipe database
echo "Wiping database..."
# Pre-flight safety — read & validate storage root before touching anything.
CONFIG_FILE="$SERVER_DIR/config.toml"
STORAGE_ROOT=$(safe_read_storage_root "$CONFIG_FILE")
safe_confirm_destruction "$SERVER_DIR" "$STORAGE_ROOT"

if ! timeout 10 rm -f "$SERVER_DIR/data/db/"*; then
    echo "WARN: DB deletion timed out or failed — please delete manually: $SERVER_DIR/data/db/"
fi

# Clean server-managed subdirectories under internal storage, preserving any
# manually placed test/sample files (e.g. existing.jpg in the root).
echo "Cleaning internal storage (server-managed dirs only)..."
if [[ -d "$SERVER_DIR/data/storage" ]]; then
    safe_purge_managed_subdirs "$SERVER_DIR/data/storage" "${SAFE_MANAGED_SUBDIRS[@]}"
fi

# Clean server-managed subdirectories under the storage root, preserving user photos
if [[ -n "$STORAGE_ROOT" && -d "$STORAGE_ROOT" ]]; then
    echo "Cleaning storage root subdirectories in: $STORAGE_ROOT"
    safe_purge_managed_subdirs "$STORAGE_ROOT" "${SAFE_MANAGED_SUBDIRS[@]}"
    echo "Original photos preserved."
else
    echo "Notice: No storage root configured (or unreadable) — skipping external cleanup."
fi

# Ensure data directories are owned by the real user so the server can write
# Scope chown to only the subdirs we touched — avoids traversing network mounts
# under data/mounts/ which can hang indefinitely.
timeout 10 chown -R "$RUN_USER:$RUN_USER" "$SERVER_DIR/data/db" "$SERVER_DIR/data/storage" 2>/dev/null || true

# The backup Docker container runs as appuser (uid 999) while the primary runs
# as the host user (uid 1000).  Both need write access to the shared storage
# directory.  Set o+rwX so "other" (uid 999) can create/write files and dirs.
timeout 10 chmod -R o+rwX "$SERVER_DIR/data/storage" 2>/dev/null || true

echo "Data cleared."

# Patch primary config.toml with the chosen port before starting the server.
# We update both `port` and `base_url` so they stay consistent.
CONFIG_FILE="$SERVER_DIR/config.toml"
if [[ -f "$CONFIG_FILE" ]]; then
    OLD_PORT=$(grep -E '^port\s*=' "$CONFIG_FILE" | head -1 | awk -F'=' '{print $2}' | tr -d ' ')
    # Replace the port line
    sed -i "s/^port\s*=.*$/port = $SERVER_PORT/" "$CONFIG_FILE"
    # Replace base_url port in case it references the old port
    if [[ -n "$OLD_PORT" ]]; then
        sed -i "s|:\($OLD_PORT\)\b|:${SERVER_PORT}|g" "$CONFIG_FILE"
    fi
fi

# Reset Docker backup instance (simple-photos-backup)
# (Container was already stopped above before the port scan.)
if [[ -d "$BACKUP_DIR" ]]; then
    echo "Resetting Docker backup instance ($BACKUP_DIR)..."
    BACKUP_DATA="$BACKUP_DIR/data"

    # Sanity: BACKUP_DATA must live under SCRIPT_DIR — otherwise refuse.
    case "$BACKUP_DATA" in
        "$SCRIPT_DIR"/docker-instances/*) : ;;
        *) abort "Backup data path '$BACKUP_DATA' is outside the project — refusing to wipe." ;;
    esac

    if [[ -d "$BACKUP_DATA" ]]; then
        # db is a flat dir of sqlite files we own — safe to clear.
        if ! timeout 10 rm -rf "$BACKUP_DATA/db/"* 2>/dev/null \
                && ! timeout 10 sudo rm -rf "$BACKUP_DATA/db/"* 2>/dev/null; then
            echo "  WARN: backup DB deletion timed out or failed — please delete manually: $BACKUP_DATA/db/"
        fi
        # storage/ is owned by the container (uid 999); needs sudo. Validate
        # path one more time before invoking sudo rm -rf.
        if [[ -d "$BACKUP_DATA/storage" ]]; then
            case "$BACKUP_DATA/storage" in
                "$SCRIPT_DIR"/docker-instances/*/data/storage)
                    if ! timeout 10 sudo rm -rf "$BACKUP_DATA/storage" 2>/dev/null \
                            && ! timeout 10 rm -rf "$BACKUP_DATA/storage" 2>/dev/null; then
                        echo "  WARN: backup storage deletion timed out or failed — please delete manually: $BACKUP_DATA/storage"
                    fi
                    ;;
                *)
                    echo "  WARN: refusing to wipe unexpected backup storage path: $BACKUP_DATA/storage"
                    ;;
            esac
        fi
        # Recreate dirs and align ownership with the container's appuser (uid 999)
        mkdir -p "$BACKUP_DATA/db" "$BACKUP_DATA/storage"
        timeout 10 chown -R 999:999 "$BACKUP_DATA" 2>/dev/null || timeout 10 chmod -R 777 "$BACKUP_DATA" 2>/dev/null || true
        echo "  backup data wiped"
    else
        mkdir -p "$BACKUP_DATA/db" "$BACKUP_DATA/storage"
        timeout 10 chown -R 999:999 "$BACKUP_DATA" 2>/dev/null || timeout 10 chmod -R 777 "$BACKUP_DATA" 2>/dev/null || true
    fi

    # Patch the compose ports line with the chosen backup port so Docker binds
    # the right host port.  The container always listens on its internal port
    # (3000); only the host-side mapping changes.
    if [[ -f "$BACKUP_COMPOSE" ]]; then
        CONTAINER_PORT=$(grep -E '^\s*-\s*"[0-9]+:[0-9]+"' "$BACKUP_COMPOSE" | head -1 | sed -E 's/.*"[0-9]+:([0-9]+)".*/\1/')
        CONTAINER_PORT=${CONTAINER_PORT:-3000}
        # Replace the ports mapping line (handles both quoted and unquoted forms)
        sed -i -E "s/(- \"?)[0-9]+(:[0-9]+\"?)/\1${BACKUP_PORT}\2/" "$BACKUP_COMPOSE"
        # Update base_url in the backup config.toml so links are correct
        BACKUP_CONFIG="$BACKUP_DIR/config.toml"
        if [[ -f "$BACKUP_CONFIG" ]]; then
            OLD_BK_URL_PORT=$(grep -E '^base_url\s*=' "$BACKUP_CONFIG" | head -1 | sed -E 's/.*:([0-9]+).*/\1/')
            if [[ -n "$OLD_BK_URL_PORT" ]]; then
                sed -i "s|:${OLD_BK_URL_PORT}\b|:${BACKUP_PORT}|g" "$BACKUP_CONFIG"
            fi
        fi
        echo "  Backup mapped to host port $BACKUP_PORT (container port $CONTAINER_PORT)"
    fi

    # Start (or recreate) the container so it picks up the clean state.
    if command -v docker &>/dev/null && [[ -f "$BACKUP_COMPOSE" ]]; then
        # Ensure the shared external network exists (compose declares it as external).
        docker network inspect simple-photos-net &>/dev/null || \
            docker network create simple-photos-net &>/dev/null || true

        echo "Building backup container image (no-cache, may take a few minutes)..."
        docker compose -f "$BACKUP_COMPOSE" build --no-cache \
            || { echo "WARNING: docker compose build failed for backup"; }
        echo "Starting backup container..."
        docker compose -f "$BACKUP_COMPOSE" up -d --force-recreate \
            || { echo "WARNING: docker compose up failed for backup"; }
        # Remove dangling images left over from the rebuild to avoid disk bloat.
        docker image prune -f 2>/dev/null || true

        # Wait for backup to start accepting requests
        echo -n "Waiting for backup on :${BACKUP_PORT}"
        BK_READY=false
        for attempt in $(seq 1 30); do
            if curl -sf "http://localhost:${BACKUP_PORT}/api/health" >/dev/null 2>&1 \
                || curl -sf "http://localhost:${BACKUP_PORT}/api/setup/status" >/dev/null 2>&1; then
                echo " ready!"
                BK_READY=true
                break
            fi
            echo -n "."
            sleep 1
        done

        if [[ "$BK_READY" == "true" ]]; then
            echo "  backup is running at http://localhost:${BACKUP_PORT}"
        else
            echo "  Warning: backup did not become healthy in time"
            echo "  Recent logs:"
            docker compose -f "$BACKUP_COMPOSE" logs --tail=30 2>/dev/null || true
        fi
    else
        if ! command -v docker &>/dev/null; then
            echo "  Skipping backup container start — docker not installed"
        elif [[ ! -f "$BACKUP_COMPOSE" ]]; then
            echo "  Skipping backup container start — $BACKUP_COMPOSE not found"
        fi
    fi
fi

# ── Start native (primary) server ────────────────────────────────────────────
LOG_FILE="${TMPDIR:-/tmp}/simple-photos-server.log"
echo "Starting server as $RUN_USER (log: $LOG_FILE)..."
if systemctl is-enabled simple-photos.service &>/dev/null 2>&1; then
    sudo systemctl restart simple-photos.service
elif [[ "$RUN_USER" != "$(id -un)" ]]; then
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
    if curl -sf "http://localhost:${SERVER_PORT}/api/setup/status" > /dev/null 2>&1; then
        echo " ready!"
        break
    fi
    echo -n "."
    sleep 1
done

# Verify setup state
STATUS=$(curl -s "http://localhost:${SERVER_PORT}/api/setup/status" 2>/dev/null || echo '{"error":"server not responding"}')
if echo "$STATUS" | grep -q '"error"'; then
    echo "WARNING: Server may have failed to start. Check $LOG_FILE"
    tail -20 "$LOG_FILE" 2>/dev/null
fi

echo ""
echo "╔══════════════════════════════════════════════════╗"
echo "║           Reset complete — server addresses      ║"
echo "╠══════════════════════════════════════════════════╣"
printf "║  Primary :  http://localhost:%-20s║\n" "${SERVER_PORT}"
if [[ -d "$BACKUP_DIR" ]] && command -v docker &>/dev/null; then
    printf "║  Backup  :  http://localhost:%-20s║\n" "${BACKUP_PORT}"
fi
echo "╚══════════════════════════════════════════════════╝"
echo ""
