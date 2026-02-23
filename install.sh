#!/usr/bin/env bash
# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  Simple Photos — Install & Setup Script                                  ║
# ║                                                                          ║
# ║  This script handles the complete setup:                                 ║
# ║    1. Checks / installs system dependencies (Rust, Node.js, Java JDK)   ║
# ║    2. Builds the web frontend (React + Vite)                             ║
# ║    3. Builds the Rust server                                             ║
# ║    4. Generates config.toml with a secure JWT secret                     ║
# ║    5. Creates data directories                                           ║
# ║    6. Optionally builds the Android APK                                  ║
# ║    7. Starts the server                                                  ║
# ║                                                                          ║
# ║  Usage:                                                                  ║
# ║    chmod +x install.sh && ./install.sh                                   ║
# ║                                                                          ║
# ║  On first run, visit http://localhost:3000 for the setup wizard.         ║
# ╚═══════════════════════════════════════════════════════════════════════════╝

set -euo pipefail

# ── Colors ────────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# ── Helpers ───────────────────────────────────────────────────────────────────
info()    { echo -e "${BLUE}ℹ ${NC}$1"; }
success() { echo -e "${GREEN}✓ ${NC}$1"; }
warn()    { echo -e "${YELLOW}⚠ ${NC}$1"; }
error()   { echo -e "${RED}✗ ${NC}$1"; }
step()    { echo -e "\n${BOLD}${CYAN}━━━ $1 ━━━${NC}\n"; }

# ── Resolve project root (where this script lives) ───────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "${BOLD}"
echo "  ╔══════════════════════════════════════════════╗"
echo "  ║         📸  Simple Photos Installer          ║"
echo "  ║    Self-hosted E2E encrypted photo library   ║"
echo "  ╚══════════════════════════════════════════════╝"
echo -e "${NC}"

# ══════════════════════════════════════════════════════════════════════════════
# Step 1: Check system dependencies
# ══════════════════════════════════════════════════════════════════════════════
step "Step 1/6: Checking dependencies"

MISSING_DEPS=()

# ── Rust ──────────────────────────────────────────────────────────────────────
if command -v cargo &>/dev/null; then
    RUST_VERSION=$(rustc --version 2>/dev/null | awk '{print $2}')
    success "Rust $RUST_VERSION found"
else
    warn "Rust not found"
    MISSING_DEPS+=("rust")
fi

# ── Node.js ───────────────────────────────────────────────────────────────────
if command -v node &>/dev/null; then
    NODE_VERSION=$(node --version 2>/dev/null)
    success "Node.js $NODE_VERSION found"
else
    warn "Node.js not found"
    MISSING_DEPS+=("node")
fi

# ── npm ───────────────────────────────────────────────────────────────────────
if command -v npm &>/dev/null; then
    NPM_VERSION=$(npm --version 2>/dev/null)
    success "npm $NPM_VERSION found"
elif [[ ! " ${MISSING_DEPS[*]:-} " =~ " node " ]]; then
    warn "npm not found"
    MISSING_DEPS+=("npm")
fi

# ── Java JDK (for Android builds) ────────────────────────────────────────────
if command -v javac &>/dev/null; then
    JAVA_VERSION=$(javac -version 2>&1 | awk '{print $2}')
    success "Java JDK $JAVA_VERSION found"
elif command -v java &>/dev/null; then
    JAVA_VERSION=$(java -version 2>&1 | head -1 | awk -F '"' '{print $2}')
    warn "Java runtime found ($JAVA_VERSION) but JDK (javac) not found"
    MISSING_DEPS+=("java")
else
    warn "Java JDK not found (needed for Android app builds)"
    MISSING_DEPS+=("java")
fi

# ── Install missing dependencies ─────────────────────────────────────────────
if [ ${#MISSING_DEPS[@]} -gt 0 ]; then
    echo ""
    info "Missing dependencies: ${MISSING_DEPS[*]}"
    echo ""
    read -p "  Install missing dependencies? [Y/n] " -n 1 -r REPLY
    echo ""
    REPLY=${REPLY:-Y}

    if [[ $REPLY =~ ^[Yy]$ ]]; then
        for dep in "${MISSING_DEPS[@]}"; do
            case "$dep" in
                rust)
                    info "Installing Rust via rustup..."
                    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
                    source "$HOME/.cargo/env" 2>/dev/null || true
                    export PATH="$HOME/.cargo/bin:$PATH"
                    if command -v cargo &>/dev/null; then
                        success "Rust installed: $(rustc --version)"
                    else
                        error "Rust installation failed. Please install manually: https://rustup.rs"
                        exit 1
                    fi
                    ;;
                node|npm)
                    info "Installing Node.js..."
                    if command -v apt-get &>/dev/null; then
                        if ! command -v node &>/dev/null; then
                            curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash - 2>/dev/null || {
                                sudo apt-get update -qq
                                sudo apt-get install -y -qq nodejs npm
                            }
                            sudo apt-get install -y -qq nodejs
                        fi
                    elif command -v dnf &>/dev/null; then
                        sudo dnf install -y nodejs npm
                    elif command -v pacman &>/dev/null; then
                        sudo pacman -S --noconfirm nodejs npm
                    elif command -v brew &>/dev/null; then
                        brew install node
                    else
                        error "Cannot auto-install Node.js. Please install it manually: https://nodejs.org"
                        exit 1
                    fi
                    if command -v node &>/dev/null; then
                        success "Node.js installed: $(node --version)"
                    else
                        error "Node.js installation failed. Please install manually: https://nodejs.org"
                        exit 1
                    fi
                    ;;
                java)
                    info "Installing Java JDK 17..."
                    if command -v apt-get &>/dev/null; then
                        sudo apt-get update -qq
                        sudo apt-get install -y -qq openjdk-17-jdk-headless 2>/dev/null || \
                            sudo apt-get install -y -qq default-jdk-headless
                    elif command -v dnf &>/dev/null; then
                        sudo dnf install -y java-17-openjdk-devel 2>/dev/null || \
                            sudo dnf install -y java-latest-openjdk-devel
                    elif command -v pacman &>/dev/null; then
                        sudo pacman -S --noconfirm jdk17-openjdk 2>/dev/null || \
                            sudo pacman -S --noconfirm jdk-openjdk
                    elif command -v brew &>/dev/null; then
                        brew install openjdk@17
                        sudo ln -sfn "$(brew --prefix)/opt/openjdk@17/libexec/openjdk.jdk" \
                            /Library/Java/JavaVirtualMachines/openjdk-17.jdk 2>/dev/null || true
                    else
                        error "Cannot auto-install Java JDK. Please install manually."
                        error "  Ubuntu/Debian:  sudo apt install openjdk-17-jdk-headless"
                        error "  Fedora:         sudo dnf install java-17-openjdk-devel"
                        error "  Arch:           sudo pacman -S jdk17-openjdk"
                        error "  macOS:          brew install openjdk@17"
                        exit 1
                    fi
                    if command -v javac &>/dev/null; then
                        success "Java JDK installed: $(javac -version 2>&1)"
                    else
                        warn "Java JDK installed but 'javac' not in PATH."
                        warn "You may need to set JAVA_HOME or add it to your PATH."
                    fi
                    ;;
            esac
        done
    else
        # Allow skipping Java — it's only needed for Android builds
        HAS_CRITICAL_MISSING=false
        for dep in "${MISSING_DEPS[@]}"; do
            if [[ "$dep" == "rust" ]] || [[ "$dep" == "node" ]] || [[ "$dep" == "npm" ]]; then
                HAS_CRITICAL_MISSING=true
            fi
        done
        if $HAS_CRITICAL_MISSING; then
            error "Cannot proceed without Rust and Node.js."
            exit 1
        else
            warn "Skipping optional dependencies. Android app builds may not work."
        fi
    fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 2: Build web frontend
# ══════════════════════════════════════════════════════════════════════════════
step "Step 2/6: Building web frontend"

cd "$SCRIPT_DIR/web"

info "Installing npm packages..."
npm install --silent 2>&1 | tail -1
success "npm packages installed"

info "Building React app..."
npm run build 2>&1 | tail -3
success "Web frontend built → web/dist/"

cd "$SCRIPT_DIR"

# ══════════════════════════════════════════════════════════════════════════════
# Step 3: Build Rust server
# ══════════════════════════════════════════════════════════════════════════════
step "Step 3/6: Building Rust server"

cd "$SCRIPT_DIR/server"

info "Building server (release mode)... this may take a few minutes on first run."
cargo build --release 2>&1 | tail -5
success "Server built → server/target/release/simple-photos-server"

cd "$SCRIPT_DIR"

# ══════════════════════════════════════════════════════════════════════════════
# Step 4: Generate configuration
# ══════════════════════════════════════════════════════════════════════════════
step "Step 4/6: Configuring"

CONFIG_FILE="$SCRIPT_DIR/server/config.toml"

# Only generate config if it doesn't exist OR if it has the placeholder secret
if [ ! -f "$CONFIG_FILE" ] || grep -q "CHANGE_ME_RANDOM_64_CHAR_HEX" "$CONFIG_FILE" 2>/dev/null; then

    # Generate a cryptographically secure JWT secret
    JWT_SECRET=$(openssl rand -hex 32 2>/dev/null || head -c 64 /dev/urandom | od -An -tx1 | tr -d ' \n' | head -c 64)

    info "Generating server configuration..."

    cat > "$CONFIG_FILE" << TOML
[server]
host = "0.0.0.0"
port = 3000
base_url = "http://localhost:3000"

[database]
path = "./data/db/simple-photos.db"
max_connections = 5

[storage]
root = "./data/storage"
default_quota_bytes = 10737418240   # 10 GB per user
max_blob_size_bytes = 5368709120    # 5 GB per upload

[auth]
jwt_secret = "${JWT_SECRET}"
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 12

[web]
static_root = "../web/dist"
TOML

    success "Config generated with secure JWT secret → server/config.toml"
else
    success "Config already exists → server/config.toml"
fi

# Create data directories
mkdir -p "$SCRIPT_DIR/server/data/db" "$SCRIPT_DIR/server/data/storage"
# Create downloads directory for APK files
mkdir -p "$SCRIPT_DIR/downloads"
success "Data directories ready"

# ══════════════════════════════════════════════════════════════════════════════
# Step 5: Build Android APK (optional)
# ══════════════════════════════════════════════════════════════════════════════
step "Step 5/6: Android app (optional)"

if command -v javac &>/dev/null && [ -d "$SCRIPT_DIR/android" ]; then
    read -p "  Build the Android APK? [y/N] " -n 1 -r BUILD_APK
    echo ""
    BUILD_APK=${BUILD_APK:-N}

    if [[ $BUILD_APK =~ ^[Yy]$ ]]; then
        info "Building Android APK..."
        cd "$SCRIPT_DIR/android"
        if [ -f "gradlew" ]; then
            chmod +x gradlew
            ./gradlew assembleDebug 2>&1 | tail -5 && \
                success "Android APK built" || \
                warn "Android build failed — you can build it later with: cd android && ./gradlew assembleDebug"
        else
            warn "No gradlew found in android/ — skipping APK build."
            warn "You can build it later once the Android project is fully set up."
        fi
        cd "$SCRIPT_DIR"
    else
        info "Skipping Android build. You can build it later with:"
        echo "    cd android && ./gradlew assembleDebug"
    fi
else
    if ! command -v javac &>/dev/null; then
        info "Java JDK not installed — skipping Android build."
        info "Install Java JDK and run: cd android && ./gradlew assembleDebug"
    else
        info "No android/ directory found — skipping APK build."
    fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# Step 6: Summary & Launch
# ══════════════════════════════════════════════════════════════════════════════
step "Step 6/6: Ready!"

echo -e "${GREEN}${BOLD}  ✅ Simple Photos is installed and ready!${NC}"
echo ""
echo -e "  ${BOLD}To start the server:${NC}"
echo -e "    cd server && ./target/release/simple-photos-server"
echo ""
echo -e "  ${BOLD}Then open:${NC}"
echo -e "    ${CYAN}http://localhost:3000${NC}"
echo ""
echo -e "  On first visit, a setup wizard will guide you through:"
echo -e "    1. Creating your account"
echo -e "    2. Setting up your encryption passphrase"
echo ""
echo -e "  ${BOLD}Features:${NC}"
echo -e "    • End-to-end encrypted photo & video storage"
echo -e "    • Google Photos metadata import"
echo -e "    • Two-factor authentication (TOTP)"
echo -e "    • Android app download from web interface"
echo -e "    • Network drive support (SMB/NFS/SSHFS)"
echo ""
echo -e "  ${BOLD}For development (hot-reload):${NC}"
echo -e "    Terminal 1: cd server && cargo run"
echo -e "    Terminal 2: cd web && npm run dev"
echo -e "    Then open: ${CYAN}http://localhost:5173${NC}"
echo ""

# ── Prompt to start ──────────────────────────────────────────────────────────
read -p "  Start the server now? [Y/n] " -n 1 -r START_REPLY
echo ""
START_REPLY=${START_REPLY:-Y}

if [[ $START_REPLY =~ ^[Yy]$ ]]; then
    echo ""
    info "Starting Simple Photos server..."
    echo -e "  ${CYAN}→ http://localhost:3000${NC}"
    echo -e "  Press ${BOLD}Ctrl+C${NC} to stop.\n"
    cd "$SCRIPT_DIR/server"
    exec ./target/release/simple-photos-server
fi
