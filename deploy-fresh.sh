#!/usr/bin/env bash
# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  Simple Photos — Fresh Deploy (Docker instance)                           ║
# ║                                                                           ║
# ║  One command to redeploy a Docker instance from a branch with EVERYTHING  ║
# ║  wired in: server image, web bundle, AI face/object models, offline geo   ║
# ║  dataset, and the Android APK — then (optionally) wipe the DB +           ║
# ║  AI/geo models & datasets are always downloaded and mounted so they're    ║
# ║  present, but left DISABLED by default (enable via Settings/config.toml).║
# ║  server-managed storage so it comes back as a brand-new, first-run setup  ║
# ║  WITHOUT deleting your original photos / import sources.                   ║
# ║                                                                           ║
# ║  Run it *inside the host that owns the Docker instance* (e.g. LXC 132     ║
# ║  `lxc-photos`). Paths are resolved from the compose file, not assumed.    ║
# ║                                                                           ║
# ║  Usage:                                                                   ║
# ║    ./deploy-fresh.sh                      # branch=dev, fresh wipe, keep originals
# ║    ./deploy-fresh.sh --branch main                                        ║
# ║    ./deploy-fresh.sh --no-wipe            # update+rebuild only, keep data ║
# ║    ./deploy-fresh.sh --instance simple-photos                             ║
# ║    ./deploy-fresh.sh --skip-assets        # don't (re)provision models/geo/apk
# ║    ./deploy-fresh.sh --yes                # don't prompt before wiping     ║
# ║                                                                           ║
# ║  SAFETY: the only data ever deleted is the DB and a fixed allow-list of   ║
# ║  server-managed subdirectories (blobs, thumbnails, caches…). Originals    ║
# ║  and import sources (e.g. Takeout/) are NEVER touched. Provisioned assets ║
# ║  (models / geo / apk) live outside the storage root and survive the wipe. ║
# ║  Every deletion is routed through guard-rails that refuse drive roots,    ║
# ║  system paths, shallow paths, and symlinks. Do not bypass them.           ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
set -euo pipefail

# ── Resolve repo root from this script's location ────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$SCRIPT_DIR"

# ── Defaults / CLI ───────────────────────────────────────────────────────────
BRANCH="dev"
INSTANCE="simple-photos"
DO_WIPE=true          # wipe DB + managed storage for a true fresh setup
WIPE_TAKEOUT=false    # never delete import sources unless explicitly asked
SKIP_ASSETS=false     # set to skip model/geo/apk provisioning
AUTO_YES=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --branch)        BRANCH="$2"; shift 2 ;;
        --instance)      INSTANCE="$2"; shift 2 ;;
        --no-wipe)       DO_WIPE=false; shift ;;
        --wipe-takeout)  WIPE_TAKEOUT=true; shift ;;
        --skip-assets)   SKIP_ASSETS=true; shift ;;
        --yes|-y)        AUTO_YES=true; shift ;;
        -h|--help)       sed -n '2,40p' "$0" | sed 's/^# ║\?\s\?//; s/\s*║$//'; exit 0 ;;
        *) echo "Unknown option: $1 (try --help)" >&2; exit 1 ;;
    esac
done

INSTANCE_DIR="$REPO_DIR/docker-instances/$INSTANCE"
COMPOSE="$INSTANCE_DIR/docker-compose.yml"
CONFIG="$INSTANCE_DIR/config.toml"

# Asset locations (outside the storage root → untouched by the wipe).
MODELS_DIR="$REPO_DIR/server/models"
GEO_DIR="$REPO_DIR/server/data"
DOWNLOADS_DIR="$REPO_DIR/downloads"
# Mirror that hosts the two large buffalo_l face models on github.com (the
# HuggingFace Xet CDN is unreachable on some networks). Same as install.sh.
MODEL_MIRROR="https://github.com/Wulfic/simple-photos/releases/download/assets-models"

# Server-managed subdirs that are safe to purge for a fresh start. Anything not
# on this list (your originals, Takeout/, hand-placed files) is preserved.
SAFE_MANAGED_SUBDIRS=(blobs metadata logs .thumbnails .renders .tmp \
                      .web_previews .converted uploads .ai_data .geo_cache)

# ── Pretty output ────────────────────────────────────────────────────────────
c_blue='\033[0;34m'; c_green='\033[0;32m'; c_yellow='\033[1;33m'; c_red='\033[0;31m'; c_bold='\033[1m'; c_nc='\033[0m'
info()  { echo -e "${c_blue}ℹ${c_nc} $*"; }
ok()    { echo -e "${c_green}✓${c_nc} $*"; }
warn()  { echo -e "${c_yellow}⚠${c_nc} $*"; }
err()   { echo -e "${c_red}✗${c_nc} $*" >&2; }
step()  { echo -e "\n${c_bold}━━━ $* ━━━${c_nc}"; }
abort() { echo -e "\n${c_red}FATAL SAFETY CHECK:${c_nc} $*\n${c_red}Aborting to protect your data.${c_nc}" >&2; exit 1; }

# ── Safety guard-rail: is this a sane destination for managed-subdir deletion? ─
_is_safe_storage_root() {
    local root="$1"
    [[ -n "$root" ]] || return 1
    [[ "$root" == /* ]] || return 1
    [[ -d "$root" ]] || return 1
    case "$root" in *'$'*|*'`'*|*'\'*|*$'\n'*|*$'\r'*) return 1 ;; esac
    local real; real=$(readlink -f -- "$root" 2>/dev/null) || return 1
    [[ -n "$real" && -d "$real" ]] || return 1
    case "$real" in
        /|/root|/home|/usr|/etc|/var|/opt|/boot|/bin|/sbin|/lib|/lib32|/lib64\
        |/mnt|/media|/srv|/tmp|/dev|/proc|/sys|/run|/Users|/Volumes) return 1 ;;
    esac
    [[ -n "${HOME:-}" && "$real" == "$HOME" ]] && return 1
    [[ "${real#/}" == *"/"* ]] || return 1   # require ≥2 path segments
    return 0
}

# safe_purge_managed_subdirs ROOT SUBDIR…  — deletes ONLY the listed subdirs
# beneath ROOT; skips anything with an unsafe name, a symlink, or that resolves
# outside ROOT.
safe_purge_managed_subdirs() {
    local root="$1"; shift
    local subdirs=("$@")
    _is_safe_storage_root "$root" || abort "Refusing to clean storage root '$root' — empty, missing, shallow, or a system path."
    local real_root; real_root=$(readlink -f -- "$root")
    local sd target real_target
    for sd in "${subdirs[@]}"; do
        [[ "$sd" =~ ^[A-Za-z0-9._-]+$ && "$sd" != "." && "$sd" != ".." ]] || { warn "skipping invalid subdir name: '$sd'"; continue; }
        target="$root/$sd"
        [[ -e "$target" || -L "$target" ]] || continue
        [[ -L "$target" ]] && { warn "'$target' is a symlink — leaving it alone."; continue; }
        [[ -d "$target" ]] || { warn "'$target' is not a directory — leaving it alone."; continue; }
        real_target=$(readlink -f -- "$target" 2>/dev/null) || { warn "could not resolve '$target' — leaving it alone."; continue; }
        [[ "$real_target" == "$real_root"/* ]] || { warn "'$target' resolves outside '$root' — leaving it alone."; continue; }
        echo "    removing $target/ ..."
        rm -rf -- "$target" 2>/dev/null || sudo rm -rf -- "$target" 2>/dev/null || warn "deletion of '$target' failed — remove manually."
    done
}

# Extract the HOST side of a compose bind-mount given the CONTAINER target.
# e.g. host_path_for /data/storage  →  /mnt/vault/Dev_Stuff/TEMP
host_path_for() {
    local container_target="$1"
    grep -E "^[[:space:]]*-[[:space:]]*[^#]+:${container_target}(:|$)" "$COMPOSE" \
        | head -1 \
        | sed -E "s|^[[:space:]]*-[[:space:]]*||; s|:${container_target}(:.*)?$||" \
        | tr -d '"'
}

# Download a file to $1 from URL $2 (idempotent; uses a .part temp file).
_dl() {
    local out="$1" url="$2"
    [[ -s "$out" ]] && { info "have $(basename "$out")"; return 0; }
    info "downloading $(basename "$out")…"
    curl -fL --retry 3 -o "$out.part" "$url" && mv "$out.part" "$out"
}

# ── Provision AI models, geo dataset, and the Android APK ─────────────────────
provision_assets() {
    step "Provision assets (AI models / geo / APK)"
    mkdir -p "$MODELS_DIR" "$GEO_DIR" "$DOWNLOADS_DIR"

    # AI face + object models. buffalo_l models come from the github.com mirror
    # (fall back to HuggingFace); the other two are already on github.com.
    _dl "$MODELS_DIR/det_10g.onnx"   "$MODEL_MIRROR/det_10g.onnx" \
        || _dl "$MODELS_DIR/det_10g.onnx"   "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx"
    _dl "$MODELS_DIR/w600k_r50.onnx" "$MODEL_MIRROR/w600k_r50.onnx" \
        || _dl "$MODELS_DIR/w600k_r50.onnx" "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx"
    _dl "$MODELS_DIR/mobilenetv2-12.onnx" "https://github.com/onnx/models/raw/refs/heads/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx" || warn "mobilenet (object detection) download failed"
    _dl "$MODELS_DIR/ultraface-RFB-320.onnx" "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx" || warn "ultraface (fallback detector) download failed"

    # Offline GeoNames reverse-geocoding dataset (+ state/region names).
    if [[ ! -s "$GEO_DIR/cities500.txt" ]]; then
        command -v unzip >/dev/null 2>&1 || { (sudo apt-get update -qq && sudo apt-get install -y -qq unzip) || apt-get install -y -qq unzip || warn "unzip unavailable"; }
        if command -v unzip >/dev/null 2>&1; then
            local z; z="$(mktemp --suffix=.zip)"
            if curl -fL --retry 3 -o "$z" "https://download.geonames.org/export/dump/cities500.zip"; then
                unzip -p "$z" cities500.txt > "$GEO_DIR/cities500.txt" && ok "cities500.txt ($(wc -l < "$GEO_DIR/cities500.txt") rows)"
            else warn "cities500 download failed (geo will self-heal at runtime via auto_download_dataset)"; fi
            rm -f "$z"
        fi
    else info "have cities500.txt"; fi
    _dl "$GEO_DIR/admin1CodesASCII.txt" "https://download.geonames.org/export/dump/admin1CodesASCII.txt" || warn "admin1 names download failed (falls back to 2-char codes)"

    # Android APK served at GET /downloads/android. Keep an existing one (e.g. a
    # locally built dev APK dropped here); otherwise fall back to the matching
    # GitHub release asset. Building the APK requires the Android SDK and is done
    # outside this script (locally or in CI) then copied into downloads/.
    if [[ -s "$DOWNLOADS_DIR/simple-photos.apk" ]]; then
        info "have simple-photos.apk ($(du -h "$DOWNLOADS_DIR/simple-photos.apk" | cut -f1))"
    else
        local ver tag
        ver="$(grep -E '^[[:space:]]*version[[:space:]]*=' "$REPO_DIR/server/Cargo.toml" 2>/dev/null | head -1 | sed -E 's/.*"([^"]+)".*/\1/')"
        tag="v${ver:-latest}"
        if _dl "$DOWNLOADS_DIR/simple-photos.apk" "https://github.com/Wulfic/simple-photos/releases/download/${tag}/simple-photos-${ver}.apk"; then
            ok "fetched release APK ($tag)"
        else
            warn "No APK present and release download failed — the in-app download page will 404."
            warn "Build it with: (cd android && ./gradlew assembleDebug) then copy app-debug.apk → downloads/simple-photos.apk"
        fi
    fi
}

# ── Ensure AI/geo config sections + model/geo/apk mounts exist (idempotent) ──
# NOTE: AI and geo models/datasets are always provisioned (see provision_assets)
# so they're present on disk, but we deliberately do NOT force `enabled = true`
# here — that's an opt-in the user flips in Settings (or config.toml) after
# first run. We only make sure the [ai]/[geo] sections exist at all.
wire_features() {
    step "Wire features (AI/geo sections + mounts; NOT auto-enabled)"
    # Ensure the sections exist (default enabled=false) without touching an
    # existing enabled value — respect whatever the user already configured.
    grep -qE '^\[ai\]'  "$CONFIG" || printf '\n[ai]\nenabled = false\n'  >> "$CONFIG"
    grep -qE '^\[geo\]' "$CONFIG" || printf '\n[geo]\nenabled = false\n' >> "$CONFIG"
    ok "AI + geo config sections present (left as-is; not force-enabled)"

    # Add the model / geo / apk bind-mounts right after the storage volume.
    local m
    for m in "$MODELS_DIR:/app/models:ro" "$GEO_DIR:/app/data:ro" "$DOWNLOADS_DIR:/app/downloads:ro"; do
        if grep -qF "$m" "$COMPOSE"; then
            info "mount present: $m"
        else
            sed -i "\|:/data/storage|a\\      - $m" "$COMPOSE"
            ok "added mount: $m"
        fi
    done
}

# ── Pre-flight ───────────────────────────────────────────────────────────────
step "Pre-flight"
command -v docker >/dev/null || abort "docker not found on PATH."
command -v git    >/dev/null || abort "git not found on PATH."
command -v npm    >/dev/null || abort "npm not found on PATH (needed to build web/dist)."
command -v curl   >/dev/null || abort "curl not found on PATH (needed to fetch assets)."
[[ -f "$COMPOSE" ]] || abort "Compose file not found: $COMPOSE"
[[ -f "$CONFIG"  ]] || abort "Config file not found: $CONFIG"

DB_HOST_DIR="$(host_path_for /data/db)"
STORAGE_HOST_DIR="$(host_path_for /data/storage)"
[[ -n "$DB_HOST_DIR" ]] || abort "Could not resolve DB host dir from $COMPOSE"
[[ -n "$STORAGE_HOST_DIR" ]] || abort "Could not resolve storage host dir from $COMPOSE"
HOST_PORT="$(grep -E '^[[:space:]]*-[[:space:]]*"?[0-9]+:[0-9]+' "$COMPOSE" | head -1 | grep -oE '[0-9]+:[0-9]+' | head -1 | cut -d: -f1)"
HOST_PORT="${HOST_PORT:-8080}"

info "Repo            : $REPO_DIR"
info "Branch          : $BRANCH"
info "Instance        : $INSTANCE  ($COMPOSE)"
info "DB host dir     : $DB_HOST_DIR"
info "Storage host dir: $STORAGE_HOST_DIR"
info "Host port       : $HOST_PORT"
info "Fresh wipe      : $DO_WIPE   (wipe Takeout: $WIPE_TAKEOUT)"

if $DO_WIPE; then
    # Validate the storage root NOW, before doing any slow work, so a misconfig
    # fails fast and loud rather than after a 10-minute rebuild.
    _is_safe_storage_root "$STORAGE_HOST_DIR" || abort "Storage root '$STORAGE_HOST_DIR' failed safety validation."
    echo
    warn "This will DELETE the database and these managed dirs under:"
    warn "    $STORAGE_HOST_DIR"
    warn "    ${SAFE_MANAGED_SUBDIRS[*]}"
    if $WIPE_TAKEOUT; then warn "    + Takeout/ (import sources) — explicitly requested"; fi
    warn "Originals / import sources are otherwise PRESERVED."
    if ! $AUTO_YES; then
        read -r -p "  Proceed with fresh wipe? [y/N] " reply
        [[ "$reply" =~ ^[Yy]$ ]] || abort "User declined."
    fi
fi

# ── Backups (always, before touching anything) ──────────────────────────────
step "Backup"
TS="$(date +%Y%m%d-%H%M%S)"
BK="/root/sp-deploy-backups/$TS"
mkdir -p "$BK"
if [[ -n "$(cd "$REPO_DIR" && git status --porcelain)" ]]; then
    (cd "$REPO_DIR" && git diff --ignore-cr-at-eol) > "$BK/uncommitted.patch" 2>/dev/null || true
    ok "Saved uncommitted changes → $BK/uncommitted.patch"
fi
if [[ -f "$DB_HOST_DIR/simple-photos.db" ]]; then
    if command -v sqlite3 >/dev/null 2>&1 && sqlite3 "$DB_HOST_DIR/simple-photos.db" ".backup '$BK/simple-photos.db'" 2>/dev/null; then
        ok "DB snapshot (sqlite .backup) → $BK/simple-photos.db"
    else
        cp -a "$DB_HOST_DIR/"simple-photos.db* "$BK/" 2>/dev/null && ok "DB raw copy → $BK/"
    fi
fi
info "Backups in: $BK"

# ── Update code ──────────────────────────────────────────────────────────────
step "Update code → origin/$BRANCH"
cd "$REPO_DIR"
git config remote.origin.fetch "+refs/heads/*:refs/remotes/origin/*"
git fetch origin --quiet
git checkout "$BRANCH" --quiet 2>/dev/null || git checkout -b "$BRANCH" "origin/$BRANCH"
git reset --hard "origin/$BRANCH"
ok "HEAD: $(git log --oneline -1)"

# ── Provision assets + wire features ─────────────────────────────────────────
$SKIP_ASSETS || provision_assets
wire_features

# ── Build web bundle (mounted into the container at /app/web/dist) ───────────
step "Build web frontend"
cd "$REPO_DIR/web"
npm install --no-audit --no-fund 2>&1 | tail -2
npm run build 2>&1 | tail -3
ok "web/dist rebuilt"

# ── Build server image ───────────────────────────────────────────────────────
step "Build server image"
cd "$INSTANCE_DIR"
docker compose build
ok "image built"

# ── Stop, wipe (optional), start ─────────────────────────────────────────────
step "Recreate container"
docker compose down || true

if $DO_WIPE; then
    info "Wiping database…"
    rm -f "$DB_HOST_DIR/"simple-photos.db* 2>/dev/null || sudo rm -f "$DB_HOST_DIR/"simple-photos.db* 2>/dev/null || true
    info "Purging server-managed storage (originals preserved)…"
    safe_purge_managed_subdirs "$STORAGE_HOST_DIR" "${SAFE_MANAGED_SUBDIRS[@]}"
    if $WIPE_TAKEOUT; then
        if [[ -d "$STORAGE_HOST_DIR/Takeout" && ! -L "$STORAGE_HOST_DIR/Takeout" ]]; then
            info "Removing import sources (Takeout/) as requested…"
            rm -rf -- "$STORAGE_HOST_DIR/Takeout" 2>/dev/null || sudo rm -rf -- "$STORAGE_HOST_DIR/Takeout" 2>/dev/null || warn "could not remove Takeout/"
        fi
    fi
    ok "Fresh storage ready"
fi

docker compose up -d --force-recreate
ok "container started"

# ── Health check ─────────────────────────────────────────────────────────────
step "Verify"
printf "Waiting for server on :%s" "$HOST_PORT"
ready=false
for _ in $(seq 1 60); do
    if curl -sf "http://127.0.0.1:${HOST_PORT}/api/health" >/dev/null 2>&1; then ready=true; break; fi
    printf "."; sleep 1
done
echo
if $ready; then
    ok "Healthy."
    echo "  setup/status: $(curl -s "http://127.0.0.1:${HOST_PORT}/api/setup/status")"
    # Surface AI/geo init + APK availability from the boot logs.
    sleep 2
    docker logs "$INSTANCE" 2>&1 | grep -iE "AI engine initialized|model loaded|geo|APK" | tail -6 || true
    if curl -sfI "http://127.0.0.1:${HOST_PORT}/downloads/android" >/dev/null 2>&1; then ok "APK download endpoint OK (/downloads/android)"; else warn "APK endpoint not serving a file"; fi
else
    warn "Did not become healthy in time. Recent logs:"
    docker compose logs --tail=40
    exit 1
fi

echo
echo -e "${c_bold}Deploy complete.${c_nc}  →  http://127.0.0.1:${HOST_PORT}  (LAN: check the instance IP)"
$DO_WIPE && echo "Fresh setup: open the URL and complete the first-run wizard. AI + geo models are installed but disabled by default — enable them in Settings if you want them; the APK is on the download page."
echo "Backups kept at: $BK"
