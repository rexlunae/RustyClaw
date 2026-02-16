#!/bin/bash
# RustyClaw Install Script
# Installs prerequisites and RustyClaw on Linux/macOS
#
# Usage: curl -fsSL https://rexlunae.github.io/RustyClaw/install.sh | bash
#    or: ./install.sh [--features <features>]

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

echo ""
echo "🦀🦞 RustyClaw Installer"
echo "========================"
echo ""

# Parse arguments
FEATURES="default"
while [[ $# -gt 0 ]]; do
    case $1 in
        --features)
            FEATURES="$2"
            shift 2
            ;;
        --full)
            FEATURES="full"
            shift
            ;;
        --help)
            echo "Usage: $0 [options]"
            echo ""
            echo "Options:"
            echo "  --features <list>  Comma-separated features (default: default)"
            echo "  --full             Install with all features (matrix, browser)"
            echo "  --help             Show this help"
            echo ""
            echo "Examples:"
            echo "  $0                      # Basic install"
            echo "  $0 --features matrix    # With Matrix support"
            echo "  $0 --full               # All features"
            exit 0
            ;;
        *)
            warn "Unknown option: $1"
            shift
            ;;
    esac
done

# Detect OS
OS="$(uname -s)"
ARCH="$(uname -m)"
info "Detected: $OS ($ARCH)"

# Check for Rust
if command -v cargo &> /dev/null; then
    RUST_VERSION=$(rustc --version | cut -d' ' -f2)
    success "Rust $RUST_VERSION found"
else
    warn "Rust not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    success "Rust installed"
fi

# Check Rust version (need 1.85+)
RUST_MAJOR=$(rustc --version | cut -d' ' -f2 | cut -d'.' -f1)
RUST_MINOR=$(rustc --version | cut -d' ' -f2 | cut -d'.' -f2)
if [ "$RUST_MAJOR" -lt 1 ] || ([ "$RUST_MAJOR" -eq 1 ] && [ "$RUST_MINOR" -lt 85 ]); then
    warn "Rust 1.85+ required. Updating..."
    rustup update stable
    success "Rust updated"
fi

# Install OS-specific dependencies
case $OS in
    Linux)
        info "Installing Linux build dependencies..."
        
        if command -v apt-get &> /dev/null; then
            # Debian/Ubuntu
            sudo apt-get update -qq
            sudo apt-get install -y -qq \
                build-essential \
                pkg-config \
                libssl-dev \
                libdbus-1-dev \
                2>/dev/null || warn "Some packages may need manual install"
            success "Debian/Ubuntu dependencies installed"
            
        elif command -v dnf &> /dev/null; then
            # Fedora/RHEL
            sudo dnf install -y -q \
                gcc \
                pkg-config \
                openssl-devel \
                dbus-devel \
                2>/dev/null || warn "Some packages may need manual install"
            success "Fedora/RHEL dependencies installed"
            
        elif command -v pacman &> /dev/null; then
            # Arch
            sudo pacman -Sy --noconfirm --needed \
                base-devel \
                openssl \
                dbus \
                2>/dev/null || warn "Some packages may need manual install"
            success "Arch dependencies installed"
            
        elif command -v apk &> /dev/null; then
            # Alpine
            sudo apk add --no-cache \
                build-base \
                openssl-dev \
                dbus-dev \
                pkgconfig \
                2>/dev/null || warn "Some packages may need manual install"
            success "Alpine dependencies installed"
            
        else
            warn "Unknown Linux distro. Please install manually:"
            echo "  - build-essential / gcc"
            echo "  - pkg-config"
            echo "  - libssl-dev / openssl-devel"
            echo "  - libdbus-1-dev / dbus-devel (for keyring)"
        fi
        ;;
        
    Darwin)
        info "Installing macOS build dependencies..."
        
        if command -v brew &> /dev/null; then
            brew install openssl pkg-config 2>/dev/null || true
            success "Homebrew dependencies installed"
        else
            warn "Homebrew not found. Installing..."
            /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
            brew install openssl pkg-config
            success "Homebrew + dependencies installed"
        fi
        
        # Set OpenSSL paths for compilation
        export OPENSSL_DIR=$(brew --prefix openssl)
        export PKG_CONFIG_PATH="$OPENSSL_DIR/lib/pkgconfig"
        ;;
        
    *)
        error "Unsupported OS: $OS (use Windows install.ps1 for Windows)"
        ;;
esac

# Install RustyClaw
info "Installing RustyClaw with features: $FEATURES"

if [ "$FEATURES" = "default" ]; then
    cargo install rustyclaw
else
    cargo install rustyclaw --features "$FEATURES"
fi

success "RustyClaw installed!"

# Verify installation
if command -v rustyclaw &> /dev/null; then
    VERSION=$(rustyclaw --version 2>/dev/null || echo "unknown")
    echo ""
    success "Installation complete: $VERSION"
    echo ""
    echo "Next steps:"
    echo "  1. Run: rustyclaw onboard"
    echo "  2. Then: rustyclaw tui"
    echo ""
    echo "Documentation: https://github.com/rexlunae/RustyClaw#readme"
else
    error "Installation failed - rustyclaw not in PATH"
fi
