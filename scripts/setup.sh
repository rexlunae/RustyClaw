#!/usr/bin/env bash
# ── RustyClaw Full Setup ─────────────────────────────────────────────────────
#
# Installs RustyClaw and all supporting tools from source:
#   • Rust toolchain (1.85+)
#   • RustyClaw (from local workspace or crates.io)
#   • uv (Python environment manager)
#   • Ollama (local model server)
#   • Node.js + npm (for exo dashboard)
#   • Exo (distributed AI cluster)
#
# Usage:
#   ./scripts/setup.sh              # install everything
#   ./scripts/setup.sh --skip exo   # skip exo
#   ./scripts/setup.sh --only rust rustyclaw  # only Rust + RustyClaw
#   ./scripts/setup.sh --help
#
# Can also be piped:
#   curl -fsSL https://raw.githubusercontent.com/rexlunae/RustyClaw/main/scripts/setup.sh | bash
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

# ── Colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

info()    { echo -e "${BLUE}[INFO]${NC}  $1"; }
success() { echo -e "${GREEN}[  OK]${NC}  $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $1"; }
err()     { echo -e "${RED}[FAIL]${NC}  $1"; }
step()    { echo -e "\n${CYAN}${BOLD}── $1 ──${NC}"; }

# ── Argument parsing ─────────────────────────────────────────────────────────
ALL_COMPONENTS=(rust rustyclaw uv ollama node exo)
SKIP=()
ONLY=()
EXO_DIR="${EXO_DIR:-$HOME/exo}"
RUSTYCLAW_FEATURES=""
FROM_SOURCE=false
FORCE=false

print_help() {
    cat <<'EOF'
🦀🦞 RustyClaw Full Setup

Usage: ./scripts/setup.sh [OPTIONS]

Options:
  --skip <component...>     Skip listed components
  --only <component...>     Install only listed components
  --exo-dir <path>          Where to clone exo (default: ~/exo)
  --features <features>     Extra cargo features for RustyClaw (e.g. "rustyclaw-core/matrix")
  --from-source             Build RustyClaw from local workspace instead of crates.io
  --force                   Overwrite existing RustyClaw binaries
  --help                    Show this help

Components: rust, rustyclaw, uv, ollama, node, exo

Examples:
  ./scripts/setup.sh                          # install everything
  ./scripts/setup.sh --skip exo ollama        # skip exo and ollama
  ./scripts/setup.sh --only rust rustyclaw    # just Rust + RustyClaw
  ./scripts/setup.sh --from-source            # build from local checkout
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --help|-h) print_help ;;
        --skip)
            shift
            while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                SKIP+=("$1"); shift
            done
            ;;
        --only)
            shift
            while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                ONLY+=("$1"); shift
            done
            ;;
        --exo-dir)   EXO_DIR="$2"; shift 2 ;;
        --features)  RUSTYCLAW_FEATURES="$2"; shift 2 ;;
        --from-source) FROM_SOURCE=true; shift ;;
        --force|-f) FORCE=true; shift ;;
        *) warn "Unknown option: $1"; shift ;;
    esac
done

# Determine which components to install
should_install() {
    local comp="$1"
    if [[ ${#ONLY[@]} -gt 0 ]]; then
        printf '%s\n' "${ONLY[@]}" | grep -qx "$comp"
        return $?
    fi
    if [[ ${#SKIP[@]} -gt 0 ]]; then
        if printf '%s\n' "${SKIP[@]}" | grep -qx "$comp" 2>/dev/null; then
            return 1
        fi
    fi
    return 0
}

# ── Detect platform ─────────────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin) PLATFORM="macos" ;;
    Linux)  PLATFORM="linux" ;;
    *)      err "Unsupported OS: $OS"; exit 1 ;;
esac

has() { command -v "$1" &>/dev/null; }

# Simple shell runner
sh_run() { bash -c "$*"; }

echo ""
echo -e "${BOLD}🦀🦞 RustyClaw Full Setup${NC}"
echo -e "${DIM}   OS: $OS ($ARCH)${NC}"
echo ""

INSTALLED=()
SKIPPED=()
FAILED=()

# ─────────────────────────────────────────────────────────────────────────────
# 1. Rust toolchain
# ─────────────────────────────────────────────────────────────────────────────
if should_install rust; then
    step "Rust toolchain"

    if has rustc; then
        RUST_VER=$(rustc --version | cut -d' ' -f2)
        RUST_MAJOR=$(echo "$RUST_VER" | cut -d'.' -f1)
        RUST_MINOR=$(echo "$RUST_VER" | cut -d'.' -f2)

        if [[ "$RUST_MAJOR" -ge 1 && "$RUST_MINOR" -ge 85 ]]; then
            success "Rust $RUST_VER (>= 1.85 ✓)"
            INSTALLED+=("rust")
        else
            warn "Rust $RUST_VER found but 1.85+ required — updating..."
            rustup update stable
            success "Rust updated to $(rustc --version | cut -d' ' -f2)"
            INSTALLED+=("rust")
        fi
    else
        info "Installing Rust via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
        success "Rust $(rustc --version | cut -d' ' -f2) installed"
        INSTALLED+=("rust")
    fi
else
    SKIPPED+=("rust")
fi

# ─────────────────────────────────────────────────────────────────────────────
# 2. OS build dependencies
# ─────────────────────────────────────────────────────────────────────────────
if should_install rustyclaw; then
    step "Build dependencies"

    case "$PLATFORM" in
        macos)
            if has brew; then
                # OpenSSL is vendored, but pkg-config is nice to have
                brew list pkg-config &>/dev/null || brew install pkg-config
                success "macOS build deps ready"
            else
                warn "Homebrew not found — installing..."
                /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
                brew install pkg-config
                success "Homebrew + build deps installed"
            fi
            ;;
        linux)
            if has apt-get; then
                info "Installing Debian/Ubuntu build deps..."
                sudo apt-get update -qq
                sudo apt-get install -y -qq build-essential pkg-config libssl-dev 2>/dev/null || true
                success "Debian/Ubuntu build deps ready"
            elif has dnf; then
                info "Installing Fedora/RHEL build deps..."
                sudo dnf install -y -q gcc pkg-config openssl-devel 2>/dev/null || true
                success "Fedora/RHEL build deps ready"
            elif has pacman; then
                info "Installing Arch build deps..."
                sudo pacman -Sy --noconfirm --needed base-devel openssl 2>/dev/null || true
                success "Arch build deps ready"
            elif has apk; then
                info "Installing Alpine build deps..."
                sudo apk add --no-cache build-base openssl-dev pkgconfig 2>/dev/null || true
                success "Alpine build deps ready"
            else
                warn "Unknown distro — you may need to install gcc, pkg-config, libssl-dev manually"
            fi
            ;;
    esac
fi

# ─────────────────────────────────────────────────────────────────────────────
# 3. RustyClaw
# ─────────────────────────────────────────────────────────────────────────────
if should_install rustyclaw; then
    step "RustyClaw"

    # Detect if we're inside the repo checkout
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    REPO_ROOT="$(cd "$SCRIPT_DIR/.." 2>/dev/null && pwd)" || REPO_ROOT=""
    IN_REPO=false
    if [[ -f "$REPO_ROOT/Cargo.toml" ]] && grep -q '\[workspace\]' "$REPO_ROOT/Cargo.toml" 2>/dev/null; then
        IN_REPO=true
    fi

    if [[ "$FROM_SOURCE" == true || "$IN_REPO" == true ]]; then
        if [[ "$IN_REPO" == true ]]; then
            info "Building from local workspace: $REPO_ROOT"
            INSTALL_PATH="$REPO_ROOT/crates/rustyclaw-cli"
        else
            info "Cloning RustyClaw..."
            git clone https://github.com/rexlunae/RustyClaw.git /tmp/rustyclaw-build
            INSTALL_PATH="/tmp/rustyclaw-build/crates/rustyclaw-cli"
        fi

        FORCE_FLAG=""
        [[ "$FORCE" == true ]] && FORCE_FLAG="--force"

        if [[ -n "$RUSTYCLAW_FEATURES" ]]; then
            cargo install --path "$INSTALL_PATH" --features "$RUSTYCLAW_FEATURES" $FORCE_FLAG
        else
            cargo install --path "$INSTALL_PATH" $FORCE_FLAG
        fi
    else
        info "Installing from crates.io..."
        FORCE_FLAG=""
        [[ "$FORCE" == true ]] && FORCE_FLAG="--force"

        if [[ -n "$RUSTYCLAW_FEATURES" ]]; then
            cargo install rustyclaw --features "$RUSTYCLAW_FEATURES" $FORCE_FLAG
        else
            cargo install rustyclaw $FORCE_FLAG
        fi
    fi

    if has rustyclaw; then
        success "RustyClaw $(rustyclaw --version 2>/dev/null || echo 'installed')"
        INSTALLED+=("rustyclaw")
    else
        err "RustyClaw binary not found in PATH after install"
        FAILED+=("rustyclaw")
    fi
else
    SKIPPED+=("rustyclaw")
fi

# ─────────────────────────────────────────────────────────────────────────────
# 4. uv (Python environment manager)
# ─────────────────────────────────────────────────────────────────────────────
if should_install uv; then
    step "uv (Python manager)"

    if has uv; then
        success "uv already installed ($(uv --version 2>/dev/null || echo 'found'))"
        INSTALLED+=("uv")
    else
        info "Installing uv..."
        if curl -LsSf https://astral.sh/uv/install.sh | sh 2>&1; then
            # Add to PATH for this session
            export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
            if has uv; then
                success "uv $(uv --version 2>/dev/null) installed"
                INSTALLED+=("uv")
            else
                err "uv installed but not found in PATH"
                FAILED+=("uv")
            fi
        else
            err "Failed to install uv"
            FAILED+=("uv")
        fi
    fi
else
    SKIPPED+=("uv")
fi

# ─────────────────────────────────────────────────────────────────────────────
# 5. Ollama (local model server)
# ─────────────────────────────────────────────────────────────────────────────
if should_install ollama; then
    step "Ollama (local models)"

    if has ollama; then
        success "Ollama already installed ($(ollama --version 2>/dev/null || echo 'found'))"
        INSTALLED+=("ollama")
    else
        info "Installing Ollama..."
        case "$PLATFORM" in
            macos)
                if has brew; then
                    brew install ollama 2>&1 && success "Ollama installed via Homebrew" && INSTALLED+=("ollama") \
                        || { err "Homebrew install failed"; FAILED+=("ollama"); }
                else
                    err "Homebrew required on macOS — install Homebrew first"
                    FAILED+=("ollama")
                fi
                ;;
            linux)
                if curl -fsSL https://ollama.com/install.sh | sh 2>&1; then
                    success "Ollama installed"
                    INSTALLED+=("ollama")
                else
                    err "Ollama install script failed"
                    FAILED+=("ollama")
                fi
                ;;
        esac
    fi
else
    SKIPPED+=("ollama")
fi

# ─────────────────────────────────────────────────────────────────────────────
# 6. Node.js + npm
# ─────────────────────────────────────────────────────────────────────────────
if should_install node; then
    step "Node.js + npm"

    # Source nvm/fnm if present
    export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
    # shellcheck disable=SC1091
    [[ -s "$NVM_DIR/nvm.sh" ]] && . "$NVM_DIR/nvm.sh" 2>/dev/null || true
    has fnm && eval "$(fnm env 2>/dev/null)" || true

    if has node && has npm; then
        success "Node $(node --version) + npm $(npm --version) already installed"
        INSTALLED+=("node")
    else
        info "Installing Node.js..."
        case "$PLATFORM" in
            macos)
                if has brew; then
                    brew install node 2>&1 && success "Node.js installed via Homebrew" && INSTALLED+=("node") \
                        || { err "Homebrew install failed"; FAILED+=("node"); }
                else
                    err "Homebrew required on macOS"
                    FAILED+=("node")
                fi
                ;;
            linux)
                if has apt-get; then
                    # Try NodeSource LTS
                    if curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash - 2>&1 \
                        && sudo apt-get install -y nodejs 2>&1; then
                        success "Node.js installed via NodeSource"
                        INSTALLED+=("node")
                    else
                        err "Node.js install failed"
                        FAILED+=("node")
                    fi
                elif has dnf; then
                    sudo dnf install -y nodejs npm 2>&1 && success "Node.js installed" && INSTALLED+=("node") \
                        || { err "Node.js install failed"; FAILED+=("node"); }
                elif has pacman; then
                    sudo pacman -Sy --noconfirm nodejs npm 2>&1 && success "Node.js installed" && INSTALLED+=("node") \
                        || { err "Node.js install failed"; FAILED+=("node"); }
                else
                    warn "Installing Node.js via nvm..."
                    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash
                    export NVM_DIR="$HOME/.nvm"
                    # shellcheck disable=SC1091
                    . "$NVM_DIR/nvm.sh"
                    nvm install --lts
                    success "Node.js installed via nvm"
                    INSTALLED+=("node")
                fi
                ;;
        esac
    fi
else
    SKIPPED+=("node")
fi

# ─────────────────────────────────────────────────────────────────────────────
# 7. Exo (distributed AI cluster)
# ─────────────────────────────────────────────────────────────────────────────
if should_install exo; then
    step "Exo (distributed AI cluster)"

    # exo needs uv and node
    if ! has uv; then
        warn "uv is required for exo but not installed — skipping exo"
        FAILED+=("exo")
    elif ! has node; then
        warn "Node.js is required for exo dashboard — skipping exo"
        FAILED+=("exo")
    else
        if [[ -d "$EXO_DIR" && -f "$EXO_DIR/setup.py" ]]; then
            success "Exo repo already present at $EXO_DIR"
            # Update and rebuild dashboard
            info "Pulling latest changes..."
            (cd "$EXO_DIR" && git pull --ff-only 2>/dev/null || true)
            if [[ -d "$EXO_DIR/exo/api/chatgpt-clone" ]]; then
                info "Rebuilding exo dashboard..."
                (cd "$EXO_DIR/exo/api/chatgpt-clone" && npm install --silent && npm run build --silent) 2>&1 || true
            fi
            INSTALLED+=("exo")
        else
            info "Cloning exo to $EXO_DIR..."
            git clone https://github.com/exo-explore/exo.git "$EXO_DIR" 2>&1

            info "Installing exo Python dependencies via uv..."
            (cd "$EXO_DIR" && uv pip install -e . 2>&1) || warn "uv pip install had warnings"

            # Build the dashboard if it exists
            if [[ -d "$EXO_DIR/exo/api/chatgpt-clone" ]]; then
                info "Building exo dashboard..."
                (cd "$EXO_DIR/exo/api/chatgpt-clone" && npm install --silent && npm run build --silent) 2>&1 || \
                    warn "Dashboard build failed (non-critical)"
            fi

            success "Exo cloned and installed at $EXO_DIR"
            INSTALLED+=("exo")
        fi
    fi
else
    SKIPPED+=("exo")
fi

# ─────────────────────────────────────────────────────────────────────────────
# Summary
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BOLD}  Setup Summary${NC}"
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [[ ${#INSTALLED[@]} -gt 0 ]]; then
    echo -e "  ${GREEN}Installed:${NC} ${INSTALLED[*]}"
fi
if [[ ${#SKIPPED[@]} -gt 0 ]]; then
    echo -e "  ${DIM}Skipped:${NC}   ${SKIPPED[*]}"
fi
if [[ ${#FAILED[@]} -gt 0 ]]; then
    echo -e "  ${RED}Failed:${NC}    ${FAILED[*]}"
fi

echo ""
if [[ ${#FAILED[@]} -eq 0 ]]; then
    echo -e "  ${GREEN}${BOLD}✓ All done!${NC}"
else
    echo -e "  ${YELLOW}⚠ Some components failed — see above for details.${NC}"
fi

echo ""
echo -e "  ${BOLD}Next steps:${NC}"
echo "    1. rustyclaw onboard     # configure provider + vault"
echo "    2. rustyclaw tui         # launch the terminal UI"
if should_install ollama 2>/dev/null; then
    echo "    3. ollama serve          # start local model server"
    echo "    4. ollama pull llama3    # download a model"
fi
if should_install exo 2>/dev/null; then
    echo ""
    echo -e "  ${BOLD}Exo:${NC}"
    echo "    cd $EXO_DIR && uv run exo   # start distributed cluster"
fi
echo ""
