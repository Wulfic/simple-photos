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
# ║    CLI (backup):  ./install.sh --mode docker --role backup --port 8081   ║
# ║                                                                          ║
# ║  CLI Flags:                                                              ║
# ║    --mode <native|docker>  Installation mode                             ║
# ║    --port <number>         Starting port (auto-increments if busy)       ║
# ║    --role <primary|backup> Server role (default: primary)                ║
# ║    --name <string>         Instance name (for Docker containers)         ║
# ║    --storage <path>        Path to photo storage directory               ║
# ║    --admin-user <string>   Admin username (skip interactive prompt)      ║
# ║    --admin-pass <string>   Admin password (skip interactive prompt)      ║
# ║    --backup-api-key <key>  Backup API key for backup servers             ║
# ║    --primary-url <url>     Primary server URL (for backup pairing)       ║
# ║    --no-build-android      Skip Android APK build prompt                 ║
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
# Default values
# ══════════════════════════════════════════════════════════════════════════════
MODE=""
PORT=""
ROLE=""
INSTANCE_NAME=""
STORAGE_PATH=""
ADMIN_USER=""
ADMIN_PASS=""
BACKUP_API_KEY=""
PRIMARY_URL=""
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
        --role)           ROLE="$2"; shift 2 ;;
        --name)           INSTANCE_NAME="$2"; shift 2 ;;
        --storage)        STORAGE_PATH="$2"; shift 2 ;;
        --admin-user)     ADMIN_USER="$2"; shift 2 ;;
        --admin-pass)     ADMIN_PASS="$2"; shift 2 ;;
        --backup-api-key) BACKUP_API_KEY="$2"; shift 2 ;;
        --primary-url)    PRIMARY_URL="$2"; shift 2 ;;
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
        warn "Port $port is in use, trying $((port + 1))..."
        port=$((port + 1))
        i=$((i + 1))
    done
    if [ $i -ge $max ]; then
        error "No available port found after $max attempts (starting from $1)"
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
# Step 2: Server role
# ══════════════════════════════════════════════════════════════════════════════
if [ -z "$ROLE" ]; then
    if ! $AUTO_YES; then
        echo ""
        echo -e "  ${BOLD}Server role:${NC}"
        echo -e "  ${CYAN}1)${NC} Primary — main server for uploading & managing photos"
        echo -e "  ${CYAN}2)${NC} Backup  — backup server that syncs from a primary"
        echo ""
        read -p "  Choose [1/2]: " ROLE_CHOICE
        case "${ROLE_CHOICE:-1}" in
            1) ROLE="primary" ;;
            2) ROLE="backup" ;;
            *) ROLE="primary" ;;
        esac
    else
        ROLE="primary"
    fi
fi
success "Server role: $ROLE"

# ══════════════════════════════════════════════════════════════════════════════
# Step 3: Check & install dependencies
# ══════════════════════════════════════════════════════════════════════════════
step "Step 2/7: Checking dependencies"

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

# ── Native dependency check ──────────────────────────────────────────────────
if [[ "$MODE" == "native" ]]; then
    MISSING_DEPS=()

    if command -v cargo &>/dev/null; then
        success "Rust $(rustc --version 2>/dev/null | awk '{print $2}') found"
    else
        warn "Rust not found"; MISSING_DEPS+=("rust")
    fi

    if command -v node &>/dev/null; then
        success "Node.js $(node --version) found"
    else
        warn "Node.js not found"; MISSING_DEPS+=("node")
    fi

    if command -v npm &>/dev/null; then
        success "npm $(npm --version) found"
    elif [[ ! " ${MISSING_DEPS[*]:-} " =~ " node " ]]; then
        warn "npm not found"; MISSING_DEPS+=("npm")
    fi

    if command -v javac &>/dev/null; then
        success "Java JDK $(javac -version 2>&1 | awk '{print $2}') found"
    else
        warn "Java JDK not found (optional — needed for Android builds)"
        MISSING_DEPS+=("java")
    fi

    if command -v ffmpeg &>/dev/null; then
        success "FFmpeg $(ffmpeg -version 2>/dev/null | head -1 | awk '{print $3}') found"
    else
        warn "FFmpeg not found (needed for video/GIF thumbnails)"
        MISSING_DEPS+=("ffmpeg")
    fi

    if [ ${#MISSING_DEPS[@]} -gt 0 ]; then
        info "Missing: ${MISSING_DEPS[*]}"
        if prompt_yn "Install missing dependencies?"; then
            for dep in "${MISSING_DEPS[@]}"; do
                case "$dep" in
                    rust)
                        info "Installing Rust via rustup..."
                        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
                        # shellcheck disable=SC1091
                        source "$HOME/.cargo/env" 2>/dev/null || true
                        export PATH="$HOME/.cargo/bin:$PATH"
                        command -v cargo &>/dev/null && success "Rust installed" || { error "Rust install failed"; exit 1; }
                        ;;
                    node|npm)
                        info "Installing Node.js..."
                        if command -v apt-get &>/dev/null; then
                            curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash - 2>/dev/null || {
                                sudo apt-get update -qq; sudo apt-get install -y -qq nodejs npm; }
                            sudo apt-get install -y -qq nodejs
                        elif command -v dnf &>/dev/null; then sudo dnf install -y nodejs npm
                        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm nodejs npm
                        elif command -v brew &>/dev/null; then brew install node
                        else error "Cannot auto-install Node.js"; exit 1; fi
                        command -v node &>/dev/null && success "Node.js installed" || { error "Node.js install failed"; exit 1; }
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
                        else warn "Cannot auto-install Java. Android builds unavailable."; fi
                        ;;
                    ffmpeg)
                        info "Installing FFmpeg..."
                        if command -v apt-get &>/dev/null; then
                            sudo apt-get update -qq
                            sudo apt-get install -y -qq ffmpeg
                        elif command -v dnf &>/dev/null; then sudo dnf install -y ffmpeg
                        elif command -v pacman &>/dev/null; then sudo pacman -S --noconfirm ffmpeg
                        elif command -v brew &>/dev/null; then brew install ffmpeg
                        else warn "Cannot auto-install FFmpeg. Video thumbnails will use placeholders."; fi
                        command -v ffmpeg &>/dev/null && success "FFmpeg installed" || warn "FFmpeg install failed — video thumbnails will use placeholders"
                        ;;
                esac
            done
        else
            for dep in "${MISSING_DEPS[@]}"; do
                [[ "$dep" == "rust" || "$dep" == "node" || "$dep" == "npm" ]] && { error "Rust and Node.js are required."; exit 1; }
            done
        fi
    fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 4: Port configuration (auto-increment)
# ══════════════════════════════════════════════════════════════════════════════
step "Step 3/7: Port configuration"

if [ -z "$PORT" ]; then
    PORT=$(prompt_text "Server port" "$DEFAULT_PORT")
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
        DEFAULT_NAME="simple-photos-${ROLE}-${PORT}"
    else
        DEFAULT_NAME="simple-photos"
    fi
    INSTANCE_NAME=$(prompt_text "Instance name" "$DEFAULT_NAME")
fi
success "Instance: $INSTANCE_NAME"

if [ -z "$STORAGE_PATH" ]; then
    STORAGE_PATH=$(prompt_text "Photo storage path" "$SCRIPT_DIR/server/data/storage")
fi
mkdir -p "$STORAGE_PATH" 2>/dev/null || true
success "Storage: $STORAGE_PATH"

JWT_SECRET=$(generate_key)

if [[ "$ROLE" == "backup" ]] && [ -z "$BACKUP_API_KEY" ]; then
    BACKUP_API_KEY=$(generate_key)
    info "Generated backup API key: ${BACKUP_API_KEY:0:16}..."
fi

if [[ "$ROLE" == "backup" ]] && [ -z "$PRIMARY_URL" ] && ! $AUTO_YES; then
    PRIMARY_URL=$(prompt_text "Primary server URL (e.g., http://localhost:8080)" "")
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 6: Build & Install
# ══════════════════════════════════════════════════════════════════════════════
step "Step 5/7: Building"

write_config() {
    local dest="$1"
    local cfg_port="$2"
    local cfg_storage="$3"
    local cfg_db="$4"
    local cfg_web="$5"
    local cfg_base_url="$6"

    local backup_line=""
    if [ -n "$BACKUP_API_KEY" ]; then
        backup_line="api_key = \"${BACKUP_API_KEY}\""
    fi

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
${backup_line}

[tls]
enabled = false
TOML
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
    cargo build --release 2>&1 | tail -5
    success "Server built → server/target/release/simple-photos-server"
    cd "$SCRIPT_DIR"

    # ── Config ────────────────────────────────────────────────────────────
    write_config \
        "$SCRIPT_DIR/server/config.toml" \
        "$PORT" \
        "$STORAGE_PATH" \
        "./data/db/simple-photos.db" \
        "../web/dist" \
        "http://localhost:${PORT}"
    success "Config → server/config.toml"

    mkdir -p "$SCRIPT_DIR/server/data/db" "$SCRIPT_DIR/server/data/storage"
    mkdir -p "$SCRIPT_DIR/downloads"
    success "Data directories ready"

    # ── Systemd service for auto-start on boot ────────────────────────────
    if command -v systemctl &>/dev/null; then
        info "Setting up systemd service for auto-start on boot..."
        SERVICE_FILE="/etc/systemd/system/simple-photos.service"
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
Environment=RUST_LOG=info

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
    success "Config → docker-instances/${INSTANCE_NAME}/config.toml"

    # ── docker-compose.yml ────────────────────────────────────────────────
    cat > "$INSTANCE_DIR/docker-compose.yml" << YAML
services:
  server:
    build:
      context: ${SCRIPT_DIR}/server
      dockerfile: Dockerfile
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
        if prompt_yn "Build the Android APK?" "N"; then
            info "Building Android APK..."
            cd "$SCRIPT_DIR/android"
            [ -f "gradlew" ] && chmod +x gradlew && \
                (./gradlew assembleDebug 2>&1 | tail -5 && success "APK built" || warn "APK build failed")
            cd "$SCRIPT_DIR"
        else
            info "Skipping Android build."
        fi
    else
        info "Java JDK or android/ not found — skipping APK build."
    fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 7: Summary & Launch
# ══════════════════════════════════════════════════════════════════════════════
step "Step 7/7: Ready!"

echo -e "${GREEN}${BOLD}  ✅ Simple Photos is installed and ready!${NC}"
echo ""
echo -e "  ${BOLD}Mode:${NC}     $MODE"
echo -e "  ${BOLD}Role:${NC}     $ROLE"
echo -e "  ${BOLD}Port:${NC}     $PORT"
echo -e "  ${BOLD}Name:${NC}     $INSTANCE_NAME"
echo -e "  ${BOLD}Storage:${NC}  $STORAGE_PATH"
echo ""

if [ -n "$BACKUP_API_KEY" ]; then
    echo -e "  ${BOLD}Backup API Key:${NC} $BACKUP_API_KEY"
    echo -e "  ${YELLOW}  (Save this — needed to register this server as a backup target)${NC}"
    echo ""
fi

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
