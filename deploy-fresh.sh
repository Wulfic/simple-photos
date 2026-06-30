#!/usr/bin/env bash
# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  Simple Photos — Fresh Deploy (Docker instance)                           ║
# ║                                                                           ║
# ║  Pulls the latest code for a branch, rebuilds the web bundle + server     ║
# ║  image, and (optionally) wipes the database + server-managed storage so   ║
# ║  the instance comes back up as a brand-new, first-run setup — WITHOUT     ║
# ║  deleting your original photos / import sources.                          ║
# ║                                                                           ║
# ║  Designed to run *inside the host that owns the Docker instance*          ║
# ║  (e.g. LXC 132 `lxc-photos`), from anywhere — paths are resolved from     ║
# ║  the compose file, not assumed.                                           ║
# ║                                                                           ║
# ║  Usage:                                                                   ║
# ║    ./deploy-fresh.sh                      # branch=dev, fresh wipe, keep originals
# ║    ./deploy-fresh.sh --branch main                                        ║
# ║    ./deploy-fresh.sh --no-wipe            # update+rebuild only, keep data ║
# ║    ./deploy-fresh.sh --instance simple-photos                             ║
# ║    ./deploy-fresh.sh --yes                # don't prompt before wiping     ║
# ║                                                                           ║
# ║  SAFETY: the only data ever deleted is the DB and a fixed allow-list of   ║
# ║  server-managed subdirectories (blobs, thumbnails, caches…). Originals    ║
# ║  and import sources (e.g. Takeout/) are NEVER touched. Every deletion is  ║
# ║  routed through guard-rails that refuse drive roots, system paths,        ║
# ║  shallow paths, and symlinks. Do not bypass them.                         ║
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
AUTO_YES=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --branch)        BRANCH="$2"; shift 2 ;;
        --instance)      INSTANCE="$2"; shift 2 ;;
        --no-wipe)       DO_WIPE=false; shift ;;
        --wipe-takeout)  WIPE_TAKEOUT=true; shift ;;
        --yes|-y)        AUTO_YES=true; shift ;;
        -h|--help)       sed -n '2,33p' "$0" | sed 's/^# ║\?\s\?//; s/\s*║$//'; exit 0 ;;
        *) echo "Unknown option: $1 (try --help)" >&2; exit 1 ;;
    esac
done

INSTANCE_DIR="$REPO_DIR/docker-instances/$INSTANCE"
COMPOSE="$INSTANCE_DIR/docker-compose.yml"
CONFIG="$INSTANCE_DIR/config.toml"

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
# e.g. host_path_for ":/data/storage"  →  /mnt/vault/Dev_Stuff/TEMP
host_path_for() {
    local container_target="$1"
    grep -E "^[[:space:]]*-[[:space:]]*[^#]+:${container_target}(:|$)" "$COMPOSE" \
        | head -1 \
        | sed -E "s|^[[:space:]]*-[[:space:]]*||; s|:${container_target}(:.*)?$||" \
        | tr -d '"'
}

# ── Pre-flight ───────────────────────────────────────────────────────────────
step "Pre-flight"
command -v docker >/dev/null || abort "docker not found on PATH."
command -v git    >/dev/null || abort "git not found on PATH."
command -v npm    >/dev/null || abort "npm not found on PATH (needed to build web/dist)."
[[ -f "$COMPOSE" ]] || abort "Compose file not found: $COMPOSE"

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
# Save any uncommitted working-tree changes as a patch so a hard reset is reversible.
if [[ -n "$(cd "$REPO_DIR" && git status --porcelain)" ]]; then
    (cd "$REPO_DIR" && git diff --ignore-cr-at-eol) > "$BK/uncommitted.patch" 2>/dev/null || true
    ok "Saved uncommitted changes → $BK/uncommitted.patch"
fi
# Snapshot the DB (consistent copy via sqlite .backup when available).
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
# Ensure all branches are fetchable (some clones ship a main-only refspec).
git config remote.origin.fetch "+refs/heads/*:refs/remotes/origin/*"
git fetch origin --quiet
git checkout "$BRANCH" --quiet 2>/dev/null || git checkout -b "$BRANCH" "origin/$BRANCH"
git reset --hard "origin/$BRANCH"
ok "HEAD: $(git log --oneline -1)"

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
        # Extra-guarded: only a dir literally named Takeout directly under the validated root.
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
else
    warn "Did not become healthy in time. Recent logs:"
    docker compose logs --tail=40
    exit 1
fi

echo
echo -e "${c_bold}Deploy complete.${c_nc}  →  http://127.0.0.1:${HOST_PORT}  (LAN: check the instance IP)"
$DO_WIPE && echo "Fresh setup: open the URL and complete the first-run wizard."
echo "Backups kept at: $BK"
