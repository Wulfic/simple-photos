#!/usr/bin/env bash
# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  Simple Photos — Install & Setup Script                                  ║
# ║                                                                          ║
# ║  Supports both Docker and bare-metal installations with auto-port        ║
# ║  detection, CLI flags, and interactive mode.                             ║
# ║                                                                          ║
# ║  Usage:                                                                  ║
# ║    Interactive:   ./install.sh                                           ║
# ║    CLI (native):  ./install.sh --mode native --port 8080                 ║
# ║    CLI (docker):  ./install.sh --mode docker --port 8080                 ║
# ║  CLI Flags:                                                              ║
# ║    --mode <native|docker>  Installation mode                             ║
# ║    --port <number>         Starting port (auto-increments if busy)       ║
# ║    --name <string>         Instance name (for Docker containers)         ║
# ║    --uninstall <native|docker>  Remove an existing installation          ║
# ║                                                                          ║
# ║  NOTE: photo storage location is now configured in the first-run setup   ║
# ║  wizard (web UI). The installer scaffolds a default directory only.      ║
# ║    --letsencrypt-domain <fqdn>   Pre-seed Let's Encrypt domain in       ║
# ║                                  config.toml [tls.letsencrypt]          ║
# ║    --letsencrypt-email <addr>    Pre-seed Let's Encrypt contact email   ║
# ║    --letsencrypt-staging         Use the LE staging directory (testing) ║
# ║    --letsencrypt-agree-tos       Confirm acceptance of the LE Subscriber║
# ║                                  Agreement (https://letsencrypt.org/    ║
# ║                                  repository/) — required to provision  ║
# ║    --local-ca                    Generate a self-signed local CA after  ║
# ║                                  install (LAN-only HTTPS, no third-     ║
# ║                                  party CA, no public DNS).  See         ║
# ║                                  Settings → SSL → "Self-signed local    ║
# ║                                  CA" in the web UI for the download &   ║
# ║                                  per-platform install scripts.          ║
# ║    --no-build-android      Skip Android APK build prompt                ║
# ║    --no-start              Don't start the server after install          ║
# ║    --yes                   Auto-accept all prompts                       ║
# ║    --help                  Show this help                                ║
# ╚═══════════════════════════════════════════════════════════════════════════╝

set -euo pipefail

# ── Colors ────────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ── Helpers ───────────────────────────────────────────────────────────────────
info()    { echo -e "${BLUE}ℹ ${NC}$1"; }
success() { echo -e "${GREEN}✓ ${NC}$1"; }
warn()    { echo -e "${YELLOW}⚠ ${NC}$1"; }
error()   { echo -e "${RED}✗ ${NC}$1"; }
step()    { echo -e "\n${BOLD}${CYAN}━━━ $1 ━━━${NC}\n"; }

# ── Resolve project root ─────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ══════════════════════════════════════════════════════════════════════════════
# AI model + GeoNames download helpers (formerly scripts/fetch_ai_models.sh
# and scripts/fetch_geo_data.sh)
# ══════════════════════════════════════════════════════════════════════════════

# Download a single file with curl or wget, using a .part temp file so a
# partial download never leaves a corrupt artifact behind.
_download_file() {
    local out="$1" url="$2"
    if [[ -s "$out" ]]; then
        info "[dl] $(basename "$out") already present — skipping"
        return 0
    fi
    info "[dl] Downloading $(basename "$out")…"
    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 -o "$out.part" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$out.part" "$url"
    else
        error "Neither curl nor wget found on PATH."
        return 1
    fi
    if [[ ! -s "$out.part" ]]; then
        error "Download produced empty file: $out.part"
        rm -f "$out.part"
        return 1
    fi
    mv "$out.part" "$out"
    info "[dl]   → $(du -h "$out" | cut -f1)"
}

# Pinned, models-only mirror release (issue #5b). The two large buffalo_l face
# models are hosted here on github.com — NOT on HuggingFace's Xet CDN
# (cas-bridge.xethub.co), which some install networks can't resolve. Same URL
# for every version, so it works for tagged and local/unstamped builds alike.
MODEL_MIRROR_BASE="https://github.com/Wulfic/simple-photos/releases/download/assets-models"

# Download a model from the github.com mirror first, falling back to its
# upstream source (HuggingFace) only if the mirror is unreachable.
_download_model() {
    local out="$1" name="$2" fallback="$3"
    if [[ -s "$out" ]]; then
        info "[dl] $name already present — skipping"
        return 0
    fi
    if _download_file "$out" "$MODEL_MIRROR_BASE/$name"; then
        return 0
    fi
    warn "[dl] mirror for $name unavailable — falling back to upstream source"
    _download_file "$out" "$fallback"
}

# Download all ONNX models needed for AI face/object recognition.
# Models: SCRFD face detector, ArcFace embeddings, UltraFace fallback,
#         MobileNetV2 object classifier (all Apache-2.0 / MIT-licensed).
download_ai_models() {
    local target="${1:-$SCRIPT_DIR/server/models}"
    mkdir -p "$target"

    # buffalo_l models: mirror-first (issue #5b). ultraface + mobilenet already
    # come from github.com (not Xet-affected), so they fetch directly.
    _download_model "$target/det_10g.onnx" "det_10g.onnx" \
        "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx"
    _download_model "$target/w600k_r50.onnx" "w600k_r50.onnx" \
        "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx"
    _download_file "$target/ultraface-RFB-320.onnx" \
        "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx"
    _download_file "$target/mobilenetv2-12.onnx" \
        "https://github.com/onnx/models/raw/refs/heads/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx"

    info "[ai] Models present in $target:"
    ls -lh "$target" | tail -n +2
}

# Download the GeoNames cities500 dataset for offline reverse geocoding.
# License: CC BY 4.0 — attribute geonames.org.
download_geo_data() {
    local target="${1:-$SCRIPT_DIR/server/data/cities500.txt}"
    local tmpdir
    tmpdir="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf '$tmpdir'" RETURN

    mkdir -p "$(dirname "$target")"
    info "[geo] Downloading GeoNames cities500 dataset…"
    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 -o "$tmpdir/cities500.zip" \
            "https://download.geonames.org/export/dump/cities500.zip"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$tmpdir/cities500.zip" \
            "https://download.geonames.org/export/dump/cities500.zip"
    else
        error "Neither curl nor wget found on PATH."
        return 1
    fi

    if command -v unzip >/dev/null 2>&1; then
        unzip -p "$tmpdir/cities500.zip" cities500.txt > "$target"
    else
        error "unzip is not installed."
        return 1
    fi

    local lines
    lines="$(wc -l <"$target" | tr -d ' ')"
    info "[geo] Done — $lines cities written to $target"

    # Companion file: admin1CodesASCII.txt promotes the 2-char ADM1 code
    # (e.g. "CA") to a full state/region name ("California").  ~10 KB,
    # plain text, optional but recommended.  Failure here is non-fatal —
    # the geocoder gracefully falls back to the raw code.
    local admin1_target
    admin1_target="$(dirname "$target")/admin1CodesASCII.txt"
    if [[ ! -s "$admin1_target" ]]; then
        info "[geo] Downloading admin1CodesASCII.txt (state/region names)…"
        if command -v curl >/dev/null 2>&1; then
            curl -fL --retry 3 -o "$admin1_target" \
                "https://download.geonames.org/export/dump/admin1CodesASCII.txt" \
                || warn "[geo] admin1 download failed — state names will fall back to 2-char codes"
        elif command -v wget >/dev/null 2>&1; then
            wget -q -O "$admin1_target" \
                "https://download.geonames.org/export/dump/admin1CodesASCII.txt" \
                || warn "[geo] admin1 download failed — state names will fall back to 2-char codes"
        fi
    fi
}

# ══════════════════════════════════════════════════════════════════════════════
# Default values
# ══════════════════════════════════════════════════════════════════════════════
MODE=""
PORT=""
INSTANCE_NAME=""
# Default scaffold path; the running server's first-run setup wizard
# is the canonical place to choose the photo storage root.
STORAGE_PATH=""
UNINSTALL=""
LE_DOMAIN=""
LE_EMAIL=""
LE_STAGING=false
LE_AGREE_TOS=false
LOCAL_CA=false
NO_BUILD_ANDROID=false
NO_START=false
AUTO_YES=false
DEFAULT_PORT=8080
DOCKER_CMD="docker"

# ══════════════════════════════════════════════════════════════════════════════
# Parse CLI arguments
# ══════════════════════════════════════════════════════════════════════════════
show_help() {
    sed -n '2,30p' "$0" | sed 's/^# ║\s*/  /' | sed 's/\s*║$//' | sed 's/^# ╔.*$//' | sed 's/^# ╚.*$//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode)           MODE="$2"; shift 2 ;;
        --port)           PORT="$2"; shift 2 ;;
        --name)           INSTANCE_NAME="$2"; shift 2 ;;
        --uninstall)      UNINSTALL="$2"; shift 2 ;;
        --letsencrypt-domain)    LE_DOMAIN="$2"; shift 2 ;;
        --letsencrypt-email)     LE_EMAIL="$2"; shift 2 ;;
        --letsencrypt-staging)   LE_STAGING=true; shift ;;
        --letsencrypt-agree-tos) LE_AGREE_TOS=true; shift ;;
        --local-ca)       LOCAL_CA=true; shift ;;
        --no-build-android) NO_BUILD_ANDROID=true; shift ;;
        --no-start)       NO_START=true; shift ;;
        --yes|-y)         AUTO_YES=true; shift ;;
        --help|-h)        show_help ;;
        *)                error "Unknown option: $1"; echo "Use --help for usage."; exit 1 ;;
    esac
done

# ══════════════════════════════════════════════════════════════════════════════
# Utility functions
# ══════════════════════════════════════════════════════════════════════════════

port_in_use() {
    local port="$1"
    if command -v ss &>/dev/null; then
        ss -tlnH 2>/dev/null | grep -q ":${port} " && return 0
    elif command -v netstat &>/dev/null; then
        netstat -tlnp 2>/dev/null | grep -q ":${port} " && return 0
    elif command -v lsof &>/dev/null; then
        lsof -i ":${port}" -sTCP:LISTEN &>/dev/null && return 0
    fi
    # Fallback: attempt connection
    (echo >/dev/tcp/localhost/"$port") 2>/dev/null && return 0
    return 1
}

find_available_port() {
    local port="${1:-$DEFAULT_PORT}"
    local max=100
    local i=0
    while port_in_use "$port" && [ $i -lt $max ]; do
        echo -e "${YELLOW}⚠ ${NC}Port $port is in use, trying $((port + 1))..." >&2
        port=$((port + 1))
        i=$((i + 1))
    done
    if [ $i -ge $max ]; then
        echo -e "${RED}✗ ${NC}No available port found after $max attempts (starting from $1)" >&2
        exit 1
    fi
    echo "$port"
}

prompt_yn() {
    local question="$1"
    local default="${2:-Y}"
    if $AUTO_YES; then
        [[ "$default" == "Y" ]] && return 0 || return 1
    fi
    local hint="[Y/n]"
    [[ "$default" == "N" ]] && hint="[y/N]"
    read -p "  $question $hint " -n 1 -r REPLY
    echo ""
    REPLY=${REPLY:-$default}
    [[ $REPLY =~ ^[Yy]$ ]]
}

prompt_text() {
    local question="$1"
    local default="${2:-}"
    if $AUTO_YES && [ -n "$default" ]; then
        echo "$default"
        return
    fi
    if [ -n "$default" ]; then
        read -p "  $question [$default]: " REPLY
        echo "${REPLY:-$default}"
    else
        read -p "  $question: " REPLY
        echo "$REPLY"
    fi
}

generate_key() {
    openssl rand -hex 32 2>/dev/null || head -c 64 /dev/urandom | od -An -tx1 | tr -d ' \n' | head -c 64
}

# ── Android SDK installer ─────────────────────────────────────────────────────
install_android_sdk() {
    local sdk_root="${ANDROID_HOME:-$HOME/android-sdk}"
    info "Installing Android SDK command-line tools to $sdk_root..."

    # Ensure unzip is available
    if ! command -v unzip &>/dev/null; then
        if command -v apt-get &>/dev/null; then sudo apt-get install -y -qq unzip
        elif command -v dnf &>/dev/null; then sudo dnf install -y unzip
        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm unzip
        else error "unzip is required but could not be installed."; return 1; fi
    fi

    local cmdline_url="https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip"
    local tmp_zip
    tmp_zip=$(mktemp /tmp/android-cmdtools-XXXXXX.zip)
    curl -fsSL "$cmdline_url" -o "$tmp_zip"

    mkdir -p "$sdk_root/cmdline-tools"
    unzip -q "$tmp_zip" -d "$sdk_root/cmdline-tools"
    # The zip extracts to cmdline-tools/cmdline-tools — rename to cmdline-tools/latest
    if [[ -d "$sdk_root/cmdline-tools/cmdline-tools" ]]; then
        mv "$sdk_root/cmdline-tools/cmdline-tools" "$sdk_root/cmdline-tools/latest"
    fi
    rm -f "$tmp_zip"

    export ANDROID_HOME="$sdk_root"
    export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$PATH"

    # Accept all licenses non-interactively
    yes | sdkmanager --licenses >/dev/null 2>&1 || true

    # Install the required SDK components for compileSdk=34 / buildTools=34.0.0
    sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0"

    # Persist ANDROID_HOME to ~/.bashrc so future sessions pick it up
    local profile_export="export ANDROID_HOME=\"$sdk_root\""
    if ! grep -qF "ANDROID_HOME" "$HOME/.bashrc" 2>/dev/null; then
        echo "$profile_export" >> "$HOME/.bashrc"
        echo 'export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$PATH"' >> "$HOME/.bashrc"
    fi

    success "Android SDK installed at $sdk_root"
}

# ── Install CUDA runtime libraries for ONNX CUDA Execution Provider ──────────
# The ONNX Runtime CUDA EP requires several CUDA 12 runtime libraries at
# runtime. These are separate from the kernel driver — Ubuntu's multiverse
# repo ships them individually. Called whenever NVIDIA driver is confirmed
# working on an apt-based system.
_install_cuda_runtime_libs() {
    command -v apt-get &>/dev/null || return 0
    # Check if already installed
    if ldconfig -p 2>/dev/null | grep -q "libcublasLt.so.12"; then
        success "CUDA runtime libraries already installed (libcublasLt.so.12 found)"
        return 0
    fi

    info "Installing CUDA runtime libraries for AI inference (ONNX CUDA EP)..."
    # Enable multiverse (houses the CUDA 12 runtime packages) — idempotent.
    sudo add-apt-repository -y multiverse 2>/dev/null || true
    sudo apt-get update -qq 2>/dev/null || true

    # Core libraries required by ORT CUDA EP (CUDA 12):
    #   libcublasLt.so.12, libcublas.so.12, libcurand.so.10,
    #   libcufft.so.11, libcudart.so.12, libcudnn.so.9
    local pkgs=(libcublaslt12 libcublas12 libcurand10 libcufft11 libcudart12)

    # nvidia-cudnn is available from Ubuntu multiverse as an install-script
    # package; add it if nvidia-cuda-dev is present (required dep).
    if apt-cache show libcuda1 &>/dev/null 2>&1 || dpkg -l libcuda1 &>/dev/null 2>&1; then
        pkgs+=(nvidia-cudnn)
    else
        pkgs+=(nvidia-cudnn)  # try anyway — apt will skip if deps unresolvable
    fi

    # shellcheck disable=SC2068
    if sudo apt-get install -y -qq ${pkgs[@]} 2>/dev/null; then
        success "CUDA runtime libraries installed: ${pkgs[*]}"
        # Refresh linker cache so the new .so files are visible immediately.
        sudo ldconfig 2>/dev/null || true
    else
        # Partial install — try without nvidia-cudnn (cuDNN needs extra deps)
        local base_pkgs=(libcublaslt12 libcublas12 libcurand10 libcufft11 libcudart12)
        # shellcheck disable=SC2068
        sudo apt-get install -y -qq ${base_pkgs[@]} 2>/dev/null && {
            success "CUDA base libraries installed (cuDNN skipped)"
            sudo ldconfig 2>/dev/null || true
        } || {
            warn "Could not install CUDA runtime libraries — ONNX AI inference will run on CPU."
            warn "Manual fix:  sudo apt install libcublaslt12 libcublas12 libcurand10 libcufft11 libcudart12 nvidia-cudnn"
        }
    fi
}

# ── NVIDIA GPU setup (optional, Linux only) ───────────────────────────────────
# Detects an NVIDIA GPU and ensures the kernel module matching the *running*
# kernel is installed, so NVENC transcoding and CUDA-accelerated AI inference
# actually work. Safe to call on systems without NVIDIA hardware (it just exits).
setup_gpu() {
    # Linux only — skip on macOS / WSL where the host already manages drivers.
    [[ "$(uname -s)" != "Linux" ]] && return 0
    # Skip inside containers (driver belongs to the host)
    [[ -f /.dockerenv ]] && return 0

    # Detect NVIDIA hardware via lspci (fast, no driver needed)
    if ! command -v lspci &>/dev/null; then
        return 0
    fi
    if ! lspci 2>/dev/null | grep -iE "vga|3d|display" | grep -qi nvidia; then
        info "No NVIDIA GPU detected — skipping GPU driver setup."
        info "  (Transcoding and AI inference will run on CPU.)"
        return 0
    fi

    success "NVIDIA GPU detected"

    # If nvidia-smi already works, still ensure CUDA runtime libs are present.
    if command -v nvidia-smi &>/dev/null && nvidia-smi -L &>/dev/null; then
        success "NVIDIA driver already loaded:"
        nvidia-smi -L | sed 's/^/    /'
        _install_cuda_runtime_libs
        return 0
    fi

    warn "NVIDIA hardware present but driver is not loaded."

    # Only attempt automated install on Debian/Ubuntu — other distros have
    # very different packaging stories.
    if ! command -v apt-get &>/dev/null; then
        warn "Automatic NVIDIA driver install is only supported on Debian/Ubuntu."
        warn "Please install the NVIDIA driver manually and re-run."
        return 0
    fi

    if ! prompt_yn "Install NVIDIA driver + kernel modules for GPU acceleration?"; then
        warn "Skipping GPU driver install. Transcoding and AI will run on CPU."
        return 0
    fi

    info "Installing dkms, build tools, and NVIDIA driver..."
    sudo apt-get update -qq
    # dkms + headers are required for any source-built nvidia kernel module
    sudo apt-get install -y -qq dkms "linux-headers-$(uname -r)" || true

    # Pick the latest available driver metapackage (open-kernel preferred for
    # Turing+ GPUs like RTX 20-series and newer).
    local driver_pkg
    driver_pkg=$(apt-cache search --names-only '^nvidia-driver-[0-9]+-open$' 2>/dev/null \
        | awk '{print $1}' | sort -V | tail -1)
    if [[ -z "$driver_pkg" ]]; then
        driver_pkg=$(apt-cache search --names-only '^nvidia-driver-[0-9]+$' 2>/dev/null \
            | awk '{print $1}' | sort -V | tail -1)
    fi

    if [[ -z "$driver_pkg" ]]; then
        warn "No NVIDIA driver package found in apt — enable the 'restricted' / 'non-free' repositories and re-run."
        return 1
    fi

    info "Installing $driver_pkg..."
    sudo apt-get install -y -qq "$driver_pkg" || {
        warn "Driver metapackage install failed."
        return 1
    }

    # Ensure prebuilt kernel modules for the *running* kernel are installed.
    # Ubuntu ships these as linux-modules-nvidia-<ver>-<flavour>-<kernel>.
    local ver
    ver=$(echo "$driver_pkg" | grep -oE '[0-9]+' | head -1)
    if [[ -n "$ver" ]]; then
        local flavour=""
        [[ "$driver_pkg" == *-open ]] && flavour="-open"
        local kver
        kver=$(uname -r)
        sudo apt-get install -y -qq \
            "linux-modules-nvidia-${ver}${flavour}-${kver}" \
            "linux-modules-nvidia-${ver}${flavour}-generic-hwe-24.04" 2>/dev/null || true
    fi

    # Try to load now without rebooting
    sudo modprobe nvidia 2>/dev/null || true
    if command -v nvidia-smi &>/dev/null && nvidia-smi -L &>/dev/null; then
        success "NVIDIA driver loaded successfully:"
        nvidia-smi -L | sed 's/^/    /'
        _install_cuda_runtime_libs
    else
        warn "Driver installed but module is not yet loaded."
        warn "A reboot is required before GPU transcoding and AI will work."
    fi
}

# ══════════════════════════════════════════════════════════════════════════════
# Uninstall mode
# ══════════════════════════════════════════════════════════════════════════════
if [ -n "${UNINSTALL:-}" ]; then
    case "$UNINSTALL" in
        native)
            step "Uninstalling Simple Photos (native)"
            if command -v systemctl &>/dev/null; then
                if systemctl is-active --quiet simple-photos.service 2>/dev/null; then
                    info "Stopping simple-photos service..."
                    sudo systemctl stop simple-photos.service 2>/dev/null || true
                fi
                if systemctl is-enabled --quiet simple-photos.service 2>/dev/null; then
                    sudo systemctl disable simple-photos.service 2>/dev/null || true
                fi
                if [ -f /etc/systemd/system/simple-photos.service ]; then
                    sudo rm -f /etc/systemd/system/simple-photos.service
                    sudo systemctl daemon-reload
                    success "Systemd service removed"
                fi
            fi
            if [ -f /etc/sudoers.d/simple-photos-cifs ]; then
                sudo rm -f /etc/sudoers.d/simple-photos-cifs
                success "Sudoers rule removed"
            fi
            if [ -f "$SCRIPT_DIR/server/target/release/simple-photos-server" ] || \
               [ -f "$SCRIPT_DIR/server/config.toml" ]; then
                if prompt_yn "Remove built server binary and config.toml?" "Y"; then
                    rm -f "$SCRIPT_DIR/server/target/release/simple-photos-server"
                    rm -f "$SCRIPT_DIR/server/config.toml"
                    success "Server binary and config removed"
                fi
            fi
            echo ""
            success "Native uninstall complete."
            warn "Photo storage data was NOT removed: $SCRIPT_DIR/server/data/storage"
            info "Remove it manually if no longer needed."
            exit 0
            ;;
        docker)
            step "Uninstalling Simple Photos (docker)"
            INSTANCES_DIR="$SCRIPT_DIR/docker-instances"
            if [ ! -d "$INSTANCES_DIR" ] || [ -z "$(ls -A "$INSTANCES_DIR" 2>/dev/null)" ]; then
                warn "No docker-instances directory found."
                exit 0
            fi
            if [ -n "$INSTANCE_NAME" ]; then
                TARGETS=("$INSTANCES_DIR/$INSTANCE_NAME")
            else
                mapfile -t TARGETS < <(find "$INSTANCES_DIR" -mindepth 1 -maxdepth 1 -type d)
            fi
            for inst_dir in "${TARGETS[@]}"; do
                [ -d "$inst_dir" ] || continue
                inst_name=$(basename "$inst_dir")
                if [ -f "$inst_dir/docker-compose.yml" ]; then
                    info "Stopping container: $inst_name"
                    (cd "$inst_dir" && $DOCKER_CMD compose down 2>/dev/null) || true
                fi
                if prompt_yn "Remove instance directory: docker-instances/$inst_name?" "Y"; then
                    rm -rf "$inst_dir"
                    success "Removed: docker-instances/$inst_name"
                fi
            done
            echo ""
            success "Docker uninstall complete."
            warn "Photo storage data outside the default scaffold path was NOT removed; remove any custom storage roots configured in the setup wizard manually."
            info "Remove it manually if no longer needed."
            exit 0
            ;;
        *)
            error "Unknown uninstall mode: '$UNINSTALL' (use 'native' or 'docker')"
            exit 1
            ;;
    esac
fi

# ══════════════════════════════════════════════════════════════════════════════
# Banner
# ══════════════════════════════════════════════════════════════════════════════
echo -e "${BOLD}"
echo "  ╔══════════════════════════════════════════════╗"
echo "  ║         📸  Simple Photos Installer          ║"
echo "  ║    Self-hosted E2E encrypted photo library   ║"
echo "  ╚══════════════════════════════════════════════╝"
echo -e "${NC}"

# ══════════════════════════════════════════════════════════════════════════════
# Step 1: Installation mode
# ══════════════════════════════════════════════════════════════════════════════
step "Step 1/7: Installation mode"

if [ -z "$MODE" ]; then
    echo -e "  ${BOLD}How would you like to install Simple Photos?${NC}"
    echo ""
    echo -e "  ${CYAN}1)${NC} Native  — build from source (requires Rust & Node.js)"
    echo -e "  ${CYAN}2)${NC} Docker  — containerized (requires Docker)"
    echo ""
    read -p "  Choose [1/2]: " MODE_CHOICE
    case "${MODE_CHOICE:-1}" in
        1) MODE="native" ;;
        2) MODE="docker" ;;
        *) MODE="native" ;;
    esac
fi

if [[ "$MODE" != "native" && "$MODE" != "docker" ]]; then
    error "Invalid mode: $MODE (must be 'native' or 'docker')"
    exit 1
fi
success "Installation mode: $MODE"

# ══════════════════════════════════════════════════════════════════════════════
# Step 2: Check & install dependencies
# ══════════════════════════════════════════════════════════════════════════════
step "Step 2/7: Checking dependencies"

# Warm up sudo credentials once so the session is cached for the entire script.
# This avoids repeated password prompts between long-running steps (e.g. builds).
if sudo -n true 2>/dev/null; then
    : # already have cached sudo, nothing to do
else
    info "Some steps require sudo. Please enter your password once now."
    sudo -v || { error "sudo authentication failed."; exit 1; }
    # Keep sudo alive in the background for the duration of the script
    ( while true; do sudo -n true; sleep 50; kill -0 "$$" 2>/dev/null || exit; done ) &
    SUDO_KEEPALIVE_PID=$!
    trap 'kill "$SUDO_KEEPALIVE_PID" 2>/dev/null || true' EXIT
fi

# ── Docker installation helper ────────────────────────────────────────────────
install_docker() {
    info "Installing Docker..."
    if command -v apt-get &>/dev/null; then
        sudo apt-get update -qq
        sudo apt-get install -y -qq ca-certificates curl gnupg lsb-release

        sudo install -m 0755 -d /etc/apt/keyrings
        if [ ! -f /etc/apt/keyrings/docker.asc ]; then
            local distro_id
            distro_id=$(. /etc/os-release && echo "$ID")
            curl -fsSL "https://download.docker.com/linux/${distro_id}/gpg" | \
                sudo tee /etc/apt/keyrings/docker.asc > /dev/null
            sudo chmod a+r /etc/apt/keyrings/docker.asc
        fi

        local codename arch
        codename=$(. /etc/os-release && echo "$VERSION_CODENAME")
        arch=$(dpkg --print-architecture)
        echo "deb [arch=${arch} signed-by=/etc/apt/keyrings/docker.asc] \
https://download.docker.com/linux/$(. /etc/os-release && echo "$ID") \
${codename} stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null

        sudo apt-get update -qq
        sudo apt-get install -y -qq docker-ce docker-ce-cli containerd.io \
            docker-buildx-plugin docker-compose-plugin

    elif command -v dnf &>/dev/null; then
        sudo dnf -y install dnf-plugins-core
        sudo dnf config-manager --add-repo \
            https://download.docker.com/linux/fedora/docker-ce.repo
        sudo dnf install -y docker-ce docker-ce-cli containerd.io \
            docker-buildx-plugin docker-compose-plugin

    elif command -v pacman &>/dev/null; then
        sudo pacman -S --noconfirm docker docker-compose docker-buildx

    else
        error "Cannot auto-install Docker. Please install manually:"
        error "  https://docs.docker.com/engine/install/"
        exit 1
    fi

    sudo systemctl start docker 2>/dev/null || true
    sudo systemctl enable docker 2>/dev/null || true

    # Add user to docker group so sudo isn't needed
    if ! groups "$USER" 2>/dev/null | grep -q '\bdocker\b'; then
        sudo usermod -aG docker "$USER" 2>/dev/null || true
        warn "Added $USER to docker group. You may need to log out/in for non-sudo access."
    fi

    if command -v docker &>/dev/null; then
        success "Docker installed: $(docker --version)"
    else
        error "Docker installation failed."
        exit 1
    fi
}

# ── Docker dependency check ──────────────────────────────────────────────────
if [[ "$MODE" == "docker" ]]; then
    if command -v docker &>/dev/null; then
        success "Docker $(docker --version 2>/dev/null | awk '{print $3}' | tr -d ',') found"
    else
        warn "Docker not found"
        if prompt_yn "Install Docker?"; then
            install_docker
        else
            error "Docker is required for Docker mode."
            exit 1
        fi
    fi

    # Check docker compose
    if docker compose version &>/dev/null 2>&1; then
        success "Docker Compose $(docker compose version --short 2>/dev/null) found"
    elif command -v docker-compose &>/dev/null; then
        success "Docker Compose (standalone) found"
    else
        warn "Docker Compose not found, attempting install..."
        if command -v apt-get &>/dev/null; then
            sudo apt-get install -y -qq docker-compose-plugin 2>/dev/null || true
        fi
    fi

    # Ensure daemon is running
    if ! docker info &>/dev/null 2>&1; then
        warn "Docker daemon not running. Starting..."
        sudo systemctl start docker 2>/dev/null || true
        sleep 2
        if ! docker info &>/dev/null 2>&1; then
            if sudo docker info &>/dev/null 2>&1; then
                DOCKER_CMD="sudo docker"
                warn "Docker requires sudo — using sudo for Docker commands."
            else
                error "Cannot reach Docker daemon. Please start Docker."
                exit 1
            fi
        fi
    fi
    success "Docker daemon is running"
fi

# ── Shared host-build dependency check (all modes) ───────────────────────────
# npm and Java are needed on the host regardless of install mode because
# reset-server.sh always runs the web frontend build (npm) and the Android
# APK build (Java/Gradle) directly on the host machine.
SHARED_MISSING_DEPS=()

if command -v npm &>/dev/null; then
    success "npm $(npm --version) found"
else
    warn "npm not found (required to build the web frontend)"
    SHARED_MISSING_DEPS+=("npm")
fi

if command -v javac &>/dev/null; then
    success "Java JDK $(javac -version 2>&1 | awk '{print $2}') found"
else
    warn "Java JDK not found (required for Android APK builds)"
    SHARED_MISSING_DEPS+=("java")
fi

if [ ${#SHARED_MISSING_DEPS[@]} -gt 0 ]; then
    info "Missing host-build tools: ${SHARED_MISSING_DEPS[*]}"
    if prompt_yn "Install missing host-build tools?"; then
        for dep in "${SHARED_MISSING_DEPS[@]}"; do
            case "$dep" in
                npm)
                    info "Installing Node.js + npm..."
                    if command -v apt-get &>/dev/null; then
                        # Official NodeSource setup script; HTTPS-only.
                        # nosemgrep: bash.curl.security.curl-pipe-bash.curl-pipe-bash
                        curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash - 2>/dev/null || {
                            sudo apt-get update -qq; sudo apt-get install -y -qq nodejs npm; }
                        sudo apt-get install -y -qq nodejs
                    elif command -v dnf &>/dev/null; then sudo dnf install -y nodejs npm
                    elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm nodejs npm
                    elif command -v brew &>/dev/null; then brew install node
                    else error "Cannot auto-install Node.js/npm"; exit 1; fi
                    command -v npm &>/dev/null && success "npm installed" || { error "npm install failed"; exit 1; }
                    ;;
                java)
                    info "Installing Java JDK 17..."
                    if command -v apt-get &>/dev/null; then
                        sudo apt-get update -qq
                        sudo apt-get install -y -qq openjdk-17-jdk-headless 2>/dev/null || \
                            sudo apt-get install -y -qq default-jdk-headless
                    elif command -v dnf &>/dev/null; then sudo dnf install -y java-17-openjdk-devel
                    elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm jdk17-openjdk
                    elif command -v brew &>/dev/null; then brew install openjdk@17
                    else warn "Cannot auto-install Java. Android builds will be skipped."; fi
                    command -v javac &>/dev/null && success "Java JDK installed" || warn "Java install failed — Android APK builds will be skipped."
                    ;;
            esac
        done
    else
        warn "Skipping host-build tool install — web frontend and Android APK builds will be skipped in reset-server.sh."
    fi
fi

# ── Native dependency check ──────────────────────────────────────────────────
if [[ "$MODE" == "native" ]]; then
    MISSING_DEPS=()

    if command -v cargo &>/dev/null; then
        success "Rust $(rustc --version 2>/dev/null | awk '{print $2}') found"
    else
        warn "Rust not found"; MISSING_DEPS+=("rust")
    fi

    # openssl-sys (and similar crates) need libssl-dev + pkg-config at compile time
    if pkg-config --exists openssl 2>/dev/null; then
        success "libssl-dev / pkg-config found"
    else
        warn "libssl-dev or pkg-config not found (required to compile Rust openssl crates)"
        MISSING_DEPS+=("libssl-dev")
    fi

    if command -v node &>/dev/null; then
        success "Node.js $(node --version) found"
    else
        warn "Node.js not found"; MISSING_DEPS+=("node")
    fi

    if command -v ffmpeg &>/dev/null; then
        success "FFmpeg $(ffmpeg -version 2>/dev/null | head -1 | awk '{print $3}') found"
    else
        warn "FFmpeg not found (required for video thumbnails and baking edits into video/audio downloads)"
        MISSING_DEPS+=("ffmpeg")
    fi

    if command -v mount.cifs &>/dev/null; then
        success "cifs-utils (mount.cifs) found"
    else
        warn "cifs-utils not found (required for SMB/network storage mounting)"
        MISSING_DEPS+=("cifs-utils")
    fi

    if command -v smbclient &>/dev/null; then
        success "smbclient found"
    else
        warn "smbclient not found (required for the SMB connection-test step in the wizard)"
        MISSING_DEPS+=("smbclient")
    fi

    if ! $NO_BUILD_ANDROID; then
        sdk_ok=false
        if [[ -n "${ANDROID_HOME:-}" ]] && [[ -f "${ANDROID_HOME}/cmdline-tools/latest/bin/sdkmanager" ]]; then
            export PATH="${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools:${PATH}"
            sdk_ok=true
        elif command -v sdkmanager &>/dev/null; then
            sdk_ok=true
        fi
        if $sdk_ok; then
            success "Android SDK found"
        else
            warn "Android SDK not found (required for Android APK build)"
            MISSING_DEPS+=("android-sdk")
        fi
    fi

    if [ ${#MISSING_DEPS[@]} -gt 0 ]; then
        info "Missing: ${MISSING_DEPS[*]}"
        if prompt_yn "Install missing dependencies?"; then
            for dep in "${MISSING_DEPS[@]}"; do
                case "$dep" in
                    rust)
                        info "Installing Rust via rustup..."
                        # Official rustup installer over HTTPS+TLS1.2; verified by upstream.
                        # nosemgrep: bash.curl.security.curl-pipe-bash.curl-pipe-bash
                        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
                        # shellcheck disable=SC1091
                        source "$HOME/.cargo/env" 2>/dev/null || true
                        export PATH="$HOME/.cargo/bin:$PATH"
                        command -v cargo &>/dev/null && success "Rust installed" || { error "Rust install failed"; exit 1; }
                        # Install OpenSSL dev headers + pkg-config required by openssl-sys crate
                        info "Installing libssl-dev and pkg-config (required by Rust openssl crates)..."
                        if command -v apt-get &>/dev/null; then
                            sudo apt-get install -y -qq libssl-dev pkg-config
                        elif command -v dnf &>/dev/null; then sudo dnf install -y openssl-devel pkgconf-pkg-config
                        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm openssl pkg-config
                        elif command -v brew &>/dev/null; then brew install openssl pkg-config
                        fi
                        success "libssl-dev and pkg-config installed"
                        ;;
                    node)
                        info "Installing Node.js..."
                        if command -v apt-get &>/dev/null; then
                            # Official NodeSource setup script; HTTPS-only.
                            # nosemgrep: bash.curl.security.curl-pipe-bash.curl-pipe-bash
                            curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash - 2>/dev/null || {
                                sudo apt-get update -qq; sudo apt-get install -y -qq nodejs npm; }
                            sudo apt-get install -y -qq nodejs
                        elif command -v dnf &>/dev/null; then sudo dnf install -y nodejs npm
                        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm nodejs npm
                        elif command -v brew &>/dev/null; then brew install node
                        else error "Cannot auto-install Node.js"; exit 1; fi
                        command -v node &>/dev/null && success "Node.js installed" || { error "Node.js install failed"; exit 1; }
                        ;;
                    ffmpeg)
                        info "Installing FFmpeg..."
                        if command -v apt-get &>/dev/null; then
                            sudo apt-get update -qq
                            sudo apt-get install -y -qq ffmpeg
                        elif command -v dnf &>/dev/null; then sudo dnf install -y ffmpeg
                        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm ffmpeg
                        elif command -v brew &>/dev/null; then brew install ffmpeg
                        else
                            error "Cannot auto-install FFmpeg. Please install it manually (https://ffmpeg.org/download.html)."
                            error "FFmpeg is required for video thumbnails and baking edits into video/audio downloads."
                            exit 1
                        fi
                        command -v ffmpeg &>/dev/null && success "FFmpeg installed" || { error "FFmpeg install failed — required for video editing."; exit 1; }
                        ;;
                    cifs-utils)
                        info "Installing cifs-utils (provides mount.cifs for SMB/network storage)..."
                        if command -v apt-get &>/dev/null; then
                            sudo apt-get update -qq
                            sudo apt-get install -y -qq cifs-utils
                        elif command -v dnf &>/dev/null; then sudo dnf install -y cifs-utils
                        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm cifs-utils
                        elif command -v brew &>/dev/null; then warn "cifs-utils not available on macOS via brew — SMB mounting may not work."
                        else warn "Cannot auto-install cifs-utils. SMB/network storage mounting will not work."; fi
                        command -v mount.cifs &>/dev/null && success "cifs-utils installed" || warn "cifs-utils install failed — SMB storage mounting will not be available."
                        ;;
                    libssl-dev)
                        info "Installing libssl-dev and pkg-config (required to compile Rust openssl crates)..."
                        if command -v apt-get &>/dev/null; then
                            sudo apt-get install -y -qq libssl-dev pkg-config
                        elif command -v dnf &>/dev/null; then sudo dnf install -y openssl-devel pkgconf-pkg-config
                        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm openssl pkg-config
                        elif command -v brew &>/dev/null; then brew install openssl pkg-config
                        else error "Cannot auto-install libssl-dev. Please install it manually."; exit 1; fi
                        pkg-config --exists openssl 2>/dev/null && success "libssl-dev and pkg-config installed" || { error "libssl-dev install failed — required to build the server."; exit 1; }
                        ;;
                    smbclient)
                        info "Installing smbclient (used by the wizard's SMB connection test)..."
                        if command -v apt-get &>/dev/null; then
                            sudo apt-get update -qq
                            sudo apt-get install -y -qq smbclient
                        elif command -v dnf &>/dev/null; then sudo dnf install -y samba-client
                        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm smbclient
                        elif command -v brew &>/dev/null; then brew install samba 2>/dev/null || warn "Could not install samba via brew."
                        else warn "Cannot auto-install smbclient. The SMB 'Test connection' button will fail."; fi
                        command -v smbclient &>/dev/null && success "smbclient installed" || warn "smbclient install failed — the SMB connection-test step will not work."
                        ;;
                    android-sdk)
                        install_android_sdk || { error "Android SDK install failed."; exit 1; }
                        ;;
                esac
            done
        else
            for dep in "${MISSING_DEPS[@]}"; do
                [[ "$dep" == "rust" || "$dep" == "node" ]] && { error "Rust and Node.js are required."; exit 1; }
            done
        fi
    fi

    # Detect/install NVIDIA driver for GPU transcoding (NVENC) + CUDA AI inference.
    # No-op on systems without an NVIDIA GPU.
    setup_gpu

    # ── Optional: passwordless sudo for mount.cifs / umount ──────────────────
    # Modern cifs-utils (≥ 7.x on Ubuntu 24.04) refuses SUID-only mounts unless
    # the mount point is listed in /etc/fstab. The cleanest fix for a
    # background server that mounts ad-hoc shares is a NOPASSWD sudoers rule
    # scoped to just `mount.cifs` and `umount`. We offer it here, opt-in.
    if command -v mount.cifs &>/dev/null && [[ "$(uname)" == "Linux" ]]; then
        SP_SUDOERS_FILE="/etc/sudoers.d/simple-photos-cifs"
        SERVER_USER="$(whoami)"
        MOUNT_CIFS_BIN="$(command -v mount.cifs)"
        UMOUNT_BIN="$(command -v umount || echo /usr/bin/umount)"
        if [[ ! -f "$SP_SUDOERS_FILE" ]]; then
            echo ""
            info "SMB mounting note: \`mount.cifs\` on modern Linux requires either an"
            info "/etc/fstab entry (per-share) or a passwordless sudo rule. Installing a"
            info "scoped sudoers drop-in lets the server mount user-supplied SMB shares"
            info "from the setup wizard without prompting for a password."
            if prompt_yn "Install NOPASSWD sudoers rule for mount.cifs/umount (user: ${SERVER_USER})?" "Y"; then
                TMP_SUDOERS=$(mktemp)
                cat > "$TMP_SUDOERS" <<SUDOERS
# Installed by Simple Photos installer.
# Allows the server user to (un)mount CIFS/SMB shares without a password.
# Scope is intentionally limited to these two binaries.
${SERVER_USER} ALL=(root) NOPASSWD: ${MOUNT_CIFS_BIN}
${SERVER_USER} ALL=(root) NOPASSWD: ${UMOUNT_BIN}
SUDOERS
                if sudo visudo -cf "$TMP_SUDOERS" >/dev/null 2>&1; then
                    sudo install -m 0440 -o root -g root "$TMP_SUDOERS" "$SP_SUDOERS_FILE" \
                        && success "Sudoers rule installed: $SP_SUDOERS_FILE" \
                        || warn "Could not install $SP_SUDOERS_FILE — run the installer with sudo to retry."
                else
                    warn "Generated sudoers file failed visudo validation — skipping."
                fi
                rm -f "$TMP_SUDOERS"
            else
                info "Skipped. You can re-run the installer or hand-craft an /etc/fstab entry later."
            fi
        else
            success "Sudoers rule already present: $SP_SUDOERS_FILE"
        fi
    fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 4: Port configuration (auto-increment)
# ══════════════════════════════════════════════════════════════════════════════
step "Step 3/7: Port configuration"

if [ -z "$PORT" ]; then
    PORT="$DEFAULT_PORT"
fi

FINAL_PORT=$(find_available_port "$PORT")
if [ "$FINAL_PORT" != "$PORT" ]; then
    info "Port $PORT was busy → using port $FINAL_PORT"
fi
PORT="$FINAL_PORT"
success "Server will run on port $PORT"

# ══════════════════════════════════════════════════════════════════════════════
# Step 5: Configuration
# ══════════════════════════════════════════════════════════════════════════════
step "Step 4/7: Configuration"

if [ -z "$INSTANCE_NAME" ]; then
    if [[ "$MODE" == "docker" ]]; then
        DEFAULT_NAME="simple-photos-${PORT}"
    else
        DEFAULT_NAME="simple-photos"
    fi
    INSTANCE_NAME=$(prompt_text "Instance name" "$DEFAULT_NAME")
fi
success "Instance: $INSTANCE_NAME"

if [ -z "$STORAGE_PATH" ]; then
    if [[ "$MODE" == "docker" ]]; then
        STORAGE_PATH="$SCRIPT_DIR/docker-instances/${INSTANCE_NAME}/data/storage"
    else
        STORAGE_PATH="$SCRIPT_DIR/server/data/storage"
    fi
fi
mkdir -p "$STORAGE_PATH" 2>/dev/null || true
success "Storage: $STORAGE_PATH"

JWT_SECRET=$(generate_key)

# ══════════════════════════════════════════════════════════════════════════════
# Step 5: Build & Install
# ══════════════════════════════════════════════════════════════════════════════
step "Step 5/7: Building"

write_config() {
    local dest="$1"
    local cfg_port="$2"
    local cfg_storage="$3"
    local cfg_db="$4"
    local cfg_web="$5"
    local cfg_base_url="$6"

    cat > "$dest" << TOML
[server]
host = "0.0.0.0"
port = ${cfg_port}
base_url = "${cfg_base_url}"

[database]
path = "${cfg_db}"
max_connections = 5

[storage]
root = "${cfg_storage}"
default_quota_bytes = 10737418240
max_blob_size_bytes = 5368709120

[auth]
jwt_secret = "${JWT_SECRET}"
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 12

[web]
static_root = "${cfg_web}"

[backup]

[tls]
enabled = false
TOML
}

# Append a [tls.letsencrypt] stanza to a freshly written config.toml when
# the user supplied --letsencrypt-* flags.  This pre-seeds the wizard's
# Let's Encrypt form (visible at first boot in Setup → SSL) so the operator
# only has to click "Issue certificate".  Provisioning itself is performed
# by the running server, which has the privileged CA-account material in
# memory — never by the installer.
maybe_write_letsencrypt_stanza() {
    local dest="$1"
    [[ -z "$LE_DOMAIN" || -z "$LE_EMAIL" ]] && return 0
    if ! $LE_AGREE_TOS; then
        warn "Let's Encrypt flags supplied without --letsencrypt-agree-tos — skipping config stub."
        warn "Re-run with --letsencrypt-agree-tos to accept https://letsencrypt.org/repository/."
        return 0
    fi
    cat >> "$dest" << TOML

[tls.letsencrypt]
domain = "${LE_DOMAIN}"
email = "${LE_EMAIL}"
staging = ${LE_STAGING}
challenge_port = 80
TOML
    success "Pre-seeded [tls.letsencrypt] for ${LE_DOMAIN} (complete in setup wizard)."
}

if [[ "$MODE" == "native" ]]; then
    # ── Build web frontend ────────────────────────────────────────────────
    info "Installing npm packages..."
    cd "$SCRIPT_DIR/web"
    npm install --silent 2>&1 | tail -1
    info "Building React app..."
    npm run build 2>&1 | tail -3
    success "Web frontend built → web/dist/"
    cd "$SCRIPT_DIR"

    # ── Build Rust server ─────────────────────────────────────────────────
    info "Building server (release)... may take a few minutes on first run."
    cd "$SCRIPT_DIR/server"

    # CUDA execution provider is baked into the default build (see
    # server/Cargo.toml [features]). The ORT CUDA EP loads libcudart
    # lazily at runtime, so the same binary runs on GPU and CPU hosts —
    # `AiEngine::detect_cuda()` decides per-process whether to use it.
    info "Building with CUDA execution provider baked in (auto-falls back to CPU at runtime)"
    cargo build --release 2>&1 | tail -5
    success "Server built → server/target/release/simple-photos-server"
    cd "$SCRIPT_DIR"

    # ── AI models + GeoNames dataset ──────────────────────────────────────
    # AI face/object recognition is a core feature: without the ONNX models
    # the server runs in `degraded_mode` and the `/api/ai/*` endpoints
    # return empty results.  These models are mandatory for a "fully
    # installed" instance.  GeoNames is similar for reverse geocoding —
    # without it `geo_city` / `geo_country` are never populated for
    # photos with GPS EXIF.  Both are downloaded unconditionally.
    info "Fetching AI ONNX models → server/models/  (~200 MB, mandatory for face/object recognition)"
    if ! download_ai_models "$SCRIPT_DIR/server/models"; then
        error "AI model download failed.  Re-run install.sh once the network issue is resolved."
        exit 1
    fi
    success "AI models installed"

    info "Fetching GeoNames cities500 → server/data/cities500.txt  (~25 MB, mandatory for reverse geocoding)"
    if ! download_geo_data "$SCRIPT_DIR/server/data/cities500.txt"; then
        warn "Geo dataset download failed; reverse-geocoding will be disabled \
              until you re-run install.sh."
    else
        success "GeoNames dataset installed"
    fi

    # ── Config ────────────────────────────────────────────────────────────
    write_config \
        "$SCRIPT_DIR/server/config.toml" \
        "$PORT" \
        "$STORAGE_PATH" \
        "./data/db/simple-photos.db" \
        "../web/dist" \
        "http://localhost:${PORT}"
    maybe_write_letsencrypt_stanza "$SCRIPT_DIR/server/config.toml"
    success "Config → server/config.toml"

    mkdir -p "$SCRIPT_DIR/server/data/db" "$SCRIPT_DIR/server/data/storage"
    mkdir -p "$SCRIPT_DIR/downloads"
    success "Data directories ready"

    # ── Systemd service for auto-start on boot ────────────────────────────
    if command -v systemctl &>/dev/null; then
        info "Setting up systemd service for auto-start on boot..."
        SERVICE_FILE="/etc/systemd/system/simple-photos.service"
        # Determine LD_LIBRARY_PATH: use user-local CUDA lib dir if it exists,
        # otherwise fall back to the extracted /tmp path if present.
        CUDA_LD_PATH=""
        if [ -d "/home/$(whoami)/.local/lib/cuda" ] && \
           ls "/home/$(whoami)/.local/lib/cuda/libcudnn.so.9" &>/dev/null 2>&1; then
            CUDA_LD_PATH="/home/$(whoami)/.local/lib/cuda"
        fi

        # Build the Environment= lines for the unit
        ENV_LINES="Environment=RUST_LOG=info"
        if [ -n "$CUDA_LD_PATH" ]; then
            ENV_LINES="${ENV_LINES}
Environment=LD_LIBRARY_PATH=${CUDA_LD_PATH}"
        fi

        sudo tee "$SERVICE_FILE" > /dev/null << UNIT
[Unit]
Description=Simple Photos Server
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=$(whoami)
WorkingDirectory=${SCRIPT_DIR}/server
ExecStart=${SCRIPT_DIR}/server/target/release/simple-photos-server
Restart=on-failure
RestartSec=5
${ENV_LINES}

[Install]
WantedBy=multi-user.target
UNIT

        sudo systemctl daemon-reload
        sudo systemctl enable simple-photos.service
        success "Systemd service installed and enabled (simple-photos.service)"
        info "The server will now start automatically on boot."
    else
        warn "systemd not found — skipping auto-start setup."
        warn "You can start the server manually: cd server && ./target/release/simple-photos-server"
    fi

elif [[ "$MODE" == "docker" ]]; then
    # ── Build web frontend if needed ──────────────────────────────────────
    if [ ! -d "$SCRIPT_DIR/web/dist" ]; then
        if command -v npm &>/dev/null; then
            info "Building web frontend..."
            cd "$SCRIPT_DIR/web"
            npm install --silent 2>&1 | tail -1
            npm run build 2>&1 | tail -3
            cd "$SCRIPT_DIR"
            success "Web frontend built"
        else
            warn "npm not available — ensure web/dist exists before starting."
        fi
    else
        success "Web frontend already built → web/dist/"
    fi

    # ── Instance directory ────────────────────────────────────────────────
    INSTANCE_DIR="$SCRIPT_DIR/docker-instances/${INSTANCE_NAME}"
    mkdir -p "$INSTANCE_DIR/data/db" "$INSTANCE_DIR/data/storage"

    # Detect the host's LAN IP for Docker base_url so cross-container
    # discovery and subnet scanning work correctly (localhost inside a
    # container refers to the container, not the host).
    DOCKER_BASE_HOST="localhost"
    LAN_IP=$(hostname -I 2>/dev/null | awk '{print $1}')
    if [[ -n "$LAN_IP" && "$LAN_IP" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        DOCKER_BASE_HOST="$LAN_IP"
    fi

    # Inside container, internal port is always 3000; external is $PORT
    write_config \
        "$INSTANCE_DIR/config.toml" \
        3000 \
        "/data/storage" \
        "/data/db/simple-photos.db" \
        "/app/web/dist" \
        "http://${DOCKER_BASE_HOST}:${PORT}"
    maybe_write_letsencrypt_stanza "$INSTANCE_DIR/config.toml"
    success "Config → docker-instances/${INSTANCE_NAME}/config.toml"

    # ── docker-compose.yml ────────────────────────────────────────────────
    # Detect CUDA on the docker build host so containers built here pick
    # up GPU-accelerated AI inference automatically. Same rule as native
    # mode — no opt-in flag, no surprises.
    DOCKER_CARGO_FEATURES=""
    if command -v nvidia-smi >/dev/null 2>&1 && \
       nvidia-smi --query-gpu=name --format=csv,noheader >/dev/null 2>&1; then
        success "NVIDIA GPU detected — image will be built with CUDA execution provider"
        DOCKER_CARGO_FEATURES="cuda"
    fi

    cat > "$INSTANCE_DIR/docker-compose.yml" << YAML
services:
  server:
    build:
      context: ${SCRIPT_DIR}/server
      dockerfile: Dockerfile
      args:
        CARGO_FEATURES: "${DOCKER_CARGO_FEATURES}"
    container_name: ${INSTANCE_NAME}
    restart: unless-stopped
    ports:
      - "${PORT}:3000"
    volumes:
      - ${INSTANCE_DIR}/config.toml:/app/config.toml:ro
      - ${SCRIPT_DIR}/web/dist:/app/web/dist:ro
      - ${INSTANCE_DIR}/data/db:/data/db
      - ${STORAGE_PATH}:/data/storage
    extra_hosts:
      - "host.docker.internal:host-gateway"
    environment:
      - RUST_LOG=info
    networks:
      - simple-photos-net
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3000/api/health"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s

networks:
  simple-photos-net:
    name: simple-photos-net
    external: true
YAML

    success "Docker Compose → docker-instances/${INSTANCE_NAME}/docker-compose.yml"

    # ── Shared network ────────────────────────────────────────────────────
    if ! $DOCKER_CMD network ls --format '{{.Name}}' 2>/dev/null | grep -q '^simple-photos-net$'; then
        $DOCKER_CMD network create simple-photos-net 2>/dev/null || true
        success "Created Docker network: simple-photos-net"
    else
        success "Docker network simple-photos-net exists"
    fi

    # ── Build image ───────────────────────────────────────────────────────
    info "Building Docker image... (may take a few minutes on first run)"
    cd "$INSTANCE_DIR"
    $DOCKER_CMD compose build 2>&1 | tail -10
    success "Docker image built for $INSTANCE_NAME"
    cd "$SCRIPT_DIR"
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 6.5: Android APK (optional, native only)
# ══════════════════════════════════════════════════════════════════════════════
if [[ "$MODE" == "native" ]] && ! $NO_BUILD_ANDROID; then
    step "Step 6/7: Android app (optional)"
    if command -v javac &>/dev/null && [ -d "$SCRIPT_DIR/android" ]; then
        if prompt_yn "Build the Android APK?" "Y"; then
            info "Building Android APK..."
            # Ensure ANDROID_HOME is set for gradlew
            if [[ -z "${ANDROID_HOME:-}" ]]; then
                if [[ -f "$HOME/android-sdk/cmdline-tools/latest/bin/sdkmanager" ]]; then
                    export ANDROID_HOME="$HOME/android-sdk"
                    export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$PATH"
                else
                    warn "ANDROID_HOME is not set — APK build may fail."
                fi
            fi
            cd "$SCRIPT_DIR/android"
            chmod +x gradlew 2>/dev/null || true
            # Bootstrap gradle-wrapper.jar if missing (it is gitignored)
            if [ ! -f "gradle/wrapper/gradle-wrapper.jar" ]; then
                info "Downloading gradle-wrapper.jar..."
                wrapper_jar_url="https://raw.githubusercontent.com/gradle/gradle/v8.7.0/gradle/wrapper/gradle-wrapper.jar"
                mkdir -p gradle/wrapper
                curl -fsSL "$wrapper_jar_url" -o gradle/wrapper/gradle-wrapper.jar || {
                    warn "Could not download gradle-wrapper.jar — APK build skipped."
                    cd "$SCRIPT_DIR"
                    SKIP_APK=true
                }
            fi
            if [[ "${SKIP_APK:-false}" != "true" ]]; then
                (ANDROID_HOME="${ANDROID_HOME:-}" ./gradlew assembleDebug 2>&1 | tail -10 && success "APK built → android/app/build/outputs/apk/debug/app-debug.apk" || warn "APK build failed")
            fi
            cd "$SCRIPT_DIR"
        else
            info "Skipping Android build."
        fi
    else
        info "Java JDK or android/ not found — skipping APK build."
    fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 6.9: Firewall — open the server port
# ══════════════════════════════════════════════════════════════════════════════
if command -v ufw &>/dev/null && ufw status 2>/dev/null | grep -q "^Status: active"; then
    if ufw status | grep -q "^${PORT}/tcp"; then
        success "UFW rule for port ${PORT}/tcp already exists"
    else
        info "UFW is active — opening port ${PORT}/tcp..."
        ufw allow "${PORT}/tcp" comment "Simple Photos ${INSTANCE_NAME}" \
            && success "UFW: opened port ${PORT}/tcp" \
            || warn "Could not add UFW rule for port ${PORT}. Add it manually: sudo ufw allow ${PORT}/tcp"
    fi
elif command -v firewall-cmd &>/dev/null && firewall-cmd --state 2>/dev/null | grep -q "^running"; then
    if firewall-cmd --query-port="${PORT}/tcp" --permanent &>/dev/null; then
        success "firewalld rule for port ${PORT}/tcp already exists"
    else
        info "firewalld is active — opening port ${PORT}/tcp..."
        firewall-cmd --permanent --add-port="${PORT}/tcp" \
            && firewall-cmd --reload \
            && success "firewalld: opened port ${PORT}/tcp" \
            || warn "Could not add firewalld rule for port ${PORT}. Add it manually: sudo firewall-cmd --permanent --add-port=${PORT}/tcp"
    fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 7: Summary & Launch
# ══════════════════════════════════════════════════════════════════════════════
step "Step 7/7: Ready!"

echo -e "${GREEN}${BOLD}  ✅ Simple Photos is installed and ready!${NC}"
echo ""
echo -e "  ${BOLD}Mode:${NC}     $MODE"
echo -e "  ${BOLD}Port:${NC}     $PORT"
echo -e "  ${BOLD}Name:${NC}     $INSTANCE_NAME"
echo -e "  ${BOLD}Storage:${NC}  $STORAGE_PATH"
echo ""

if [[ "$MODE" == "native" ]]; then
    if command -v systemctl &>/dev/null; then
        echo -e "  ${BOLD}Start:${NC}    sudo systemctl start simple-photos"
        echo -e "  ${BOLD}Stop:${NC}     sudo systemctl stop simple-photos"
        echo -e "  ${BOLD}Restart:${NC}  sudo systemctl restart simple-photos"
        echo -e "  ${BOLD}Logs:${NC}     sudo journalctl -u simple-photos -f"
        echo -e "  ${BOLD}Boot:${NC}     ${GREEN}Enabled (auto-starts on boot)${NC}"
    else
        echo -e "  ${BOLD}Start:${NC}  cd server && ./target/release/simple-photos-server"
    fi
    echo -e "  ${BOLD}Open:${NC}   ${CYAN}http://localhost:${PORT}${NC}"
else
    echo -e "  ${BOLD}Start:${NC}  cd docker-instances/${INSTANCE_NAME} && docker compose up -d"
    echo -e "  ${BOLD}Stop:${NC}   cd docker-instances/${INSTANCE_NAME} && docker compose down"
    echo -e "  ${BOLD}Logs:${NC}   cd docker-instances/${INSTANCE_NAME} && docker compose logs -f"
    echo -e "  ${BOLD}Open:${NC}   ${CYAN}http://localhost:${PORT}${NC}"
fi
echo ""

if ! $NO_START; then
    if prompt_yn "Start the server now?"; then
        echo ""
        if [[ "$MODE" == "native" ]]; then
            if command -v systemctl &>/dev/null; then
                info "Starting via systemd..."
                sudo systemctl start simple-photos.service
                echo -e "  ${CYAN}→ http://localhost:${PORT}${NC}"
                echo ""
                success "Server is running as a systemd service."
                info "View logs:  sudo journalctl -u simple-photos -f"
                info "Stop:       sudo systemctl stop simple-photos"
                info "Restart:    sudo systemctl restart simple-photos"
            else
                info "Starting on port $PORT..."
                echo -e "  ${CYAN}→ http://localhost:${PORT}${NC}"
                echo -e "  Press ${BOLD}Ctrl+C${NC} to stop.\n"
                cd "$SCRIPT_DIR/server"
                exec ./target/release/simple-photos-server
            fi
        else
            info "Starting container: $INSTANCE_NAME"
            cd "$INSTANCE_DIR"
            $DOCKER_CMD compose up -d
            echo -e "\n  ${CYAN}→ http://localhost:${PORT}${NC}\n"
            success "Container $INSTANCE_NAME is running"
        fi
    fi
fi

# ── Local-CA hint ─────────────────────────────────────────────────────────
# When the operator passed --local-ca we don't try to call the API from
# inside the installer (the server may not be ready yet, and admin auth is
# user-driven).  Instead we surface a clear post-install instruction so
# they know exactly where to click to generate the cert and download the
# install bundle for each device.
if $LOCAL_CA; then
    echo ""
    info "Self-signed local CA was requested (--local-ca)."
    info "Open the web UI → Settings → SSL / TLS → \"Self-signed local CA\" and"
    info "click \"Generate local CA\".  Then click \"Download CA install bundle\""
    info "and run the included script on each device:"
    info "  • Linux:    sudo ./install-linux.sh"
    info "  • Windows:  PowerShell (as admin) → .\\install-windows.ps1"
    info "  • Android:  follow install-android.txt"
    info "After installing the CA on a device, the Simple Photos URL will load"
    info "as a fully-trusted HTTPS site with no browser warnings."
fi
