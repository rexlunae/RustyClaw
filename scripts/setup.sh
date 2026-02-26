#!/usr/bin/env bash
# ── RustyClaw Full Setup ─────────────────────────────────────────────────────
#
# Installs RustyClaw and optionally supporting tools:
#   • Rust toolchain (1.85+)
#   • RustyClaw (from local workspace or crates.io)
#   • uv (Python environment manager)
#   • Ollama (local model server)
#   • Node.js + npm (for exo dashboard)
#   • Exo (distributed AI cluster)
#
# Usage:
#   ./scripts/setup.sh              # interactive mode — choose what to install
#   ./scripts/setup.sh --all        # install everything (no prompts)
#   ./scripts/setup.sh --skip exo   # skip exo
#   ./scripts/setup.sh --only rust rustyclaw  # only Rust + RustyClaw
#   ./scripts/setup.sh --help
#
# Can also be piped (non-interactive installs core only):
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
ALL_COMPONENTS="rust rustyclaw uv ollama node exo"
CORE_COMPONENTS="rust rustyclaw"
OPTIONAL_COMPONENTS="uv ollama node exo"
SKIP=""
ONLY=""
EXO_DIR="${EXO_DIR:-$HOME/exo}"
RUSTYCLAW_FEATURES=""
FROM_SOURCE=false
FORCE=false
INSTALL_ALL=false
INTERACTIVE=true

# Detect if we're in a pipe (non-interactive)
if [[ ! -t 0 ]]; then
    INTERACTIVE=false
fi

print_help() {
    cat <<'EOF'
🦀🦞 RustyClaw Full Setup

Usage: ./scripts/setup.sh [OPTIONS]

Options:
  --all                     Install all components (no prompts)
  --skip <component...>     Skip listed components
  --only <component...>     Install only listed components
  --exo-dir <path>          Where to clone exo (default: ~/exo)
  --features <features>     Extra cargo features for RustyClaw (e.g. "rustyclaw-core/matrix")
  --from-source             Build RustyClaw from local workspace instead of crates.io
  --force                   Overwrite existing RustyClaw binaries
  --help                    Show this help

Components: rust, rustyclaw, uv, ollama, node, exo

Examples:
  ./scripts/setup.sh                          # interactive — choose components
  ./scripts/setup.sh --all                    # install everything
  ./scripts/setup.sh --skip exo ollama        # skip exo and ollama
  ./scripts/setup.sh --only rust rustyclaw    # just Rust + RustyClaw
  ./scripts/setup.sh --from-source            # build from local checkout
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --help|-h) print_help ;;
        --all|-a)
            INSTALL_ALL=true
            INTERACTIVE=false
            shift
            ;;
        --skip)
            shift
            while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                SKIP="$SKIP $1"; shift
            done
            INTERACTIVE=false
            ;;
        --only)
            shift
            while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                ONLY="$ONLY $1"; shift
            done
            INTERACTIVE=false
            ;;
        --exo-dir)   EXO_DIR="$2"; shift 2 ;;
        --features)  RUSTYCLAW_FEATURES="$2"; shift 2 ;;
        --from-source) FROM_SOURCE=true; shift ;;
        --force|-f) FORCE=true; shift ;;
        *) warn "Unknown option: $1"; shift ;;
    esac
done

# ── Detect platform ─────────────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin) PLATFORM="macos" ;;
    Linux)  PLATFORM="linux" ;;
    *)      err "Unsupported OS: $OS"; exit 1 ;;
esac

has() { command -v "$1" &>/dev/null; }

# ── Detect installed components ─────────────────────────────────────────────
# Using simple variables instead of associative arrays for bash 3.x compatibility
STATUS_rust="missing"; VERSION_rust=""
STATUS_rustyclaw="missing"; VERSION_rustyclaw=""
STATUS_uv="missing"; VERSION_uv=""
STATUS_ollama="missing"; VERSION_ollama=""
STATUS_node="missing"; VERSION_node=""
STATUS_exo="missing"; VERSION_exo=""

# Selection state (1=selected, 0=not selected)
SEL_rust=1; SEL_rustyclaw=1  # Core: selected by default
SEL_uv=0; SEL_ollama=0; SEL_node=0; SEL_exo=0  # Optional: not selected

detect_components() {
    if has rustc; then
        STATUS_rust="installed"
        VERSION_rust="$(rustc --version 2>/dev/null | cut -d' ' -f2)"
    fi
    if has rustyclaw; then
        STATUS_rustyclaw="installed"
        VERSION_rustyclaw="$(rustyclaw --version 2>/dev/null || echo 'unknown')"
    fi
    if has uv; then
        STATUS_uv="installed"
        VERSION_uv="$(uv --version 2>/dev/null | head -1)"
    fi
    if has ollama; then
        STATUS_ollama="installed"
        VERSION_ollama="$(ollama --version 2>/dev/null | head -1 || echo 'found')"
    fi
    if has node && has npm; then
        STATUS_node="installed"
        VERSION_node="node $(node --version 2>/dev/null), npm $(npm --version 2>/dev/null)"
    fi
    if [[ -d "$EXO_DIR" && -f "$EXO_DIR/setup.py" ]]; then
        STATUS_exo="installed"
        VERSION_exo="at $EXO_DIR"
    fi
}

detect_components

# ── Interactive selection ───────────────────────────────────────────────────
get_status() {
    local comp="$1"
    eval "echo \$STATUS_$comp"
}

get_version() {
    local comp="$1"
    eval "echo \$VERSION_$comp"
}

get_selected() {
    local comp="$1"
    eval "echo \$SEL_$comp"
}

set_selected() {
    local comp="$1"
    local val="$2"
    eval "SEL_$comp=$val"
}

toggle_selected() {
    local comp="$1"
    local current=$(get_selected "$comp")
    if [[ "$current" == "1" ]]; then
        set_selected "$comp" 0
    else
        set_selected "$comp" 1
    fi
}

show_menu() {
    clear
    echo -e "${BOLD}🦀🦞 RustyClaw Setup${NC}"
    echo -e "${DIM}   OS: $OS ($ARCH)${NC}"
    echo ""
    echo -e "${BOLD}Select components to install:${NC}"
    echo -e "${DIM}(Use number keys to toggle, Enter to proceed, q to quit)${NC}"
    echo ""
    
    local i=1
    for comp in $ALL_COMPONENTS; do
        local status=$(get_status "$comp")
        local version=$(get_version "$comp")
        local selected=$(get_selected "$comp")
        
        # Checkbox
        local check="[ ]"
        [[ "$selected" == "1" ]] && check="[${GREEN}✓${NC}]"
        
        # Status indicator
        local status_str=""
        if [[ "$status" == "installed" ]]; then
            status_str="${GREEN}(installed: $version)${NC}"
        else
            status_str="${DIM}(not installed)${NC}"
        fi
        
        # Component description
        local desc=""
        case "$comp" in
            rust)      desc="Rust toolchain (required)" ;;
            rustyclaw) desc="RustyClaw CLI + TUI" ;;
            uv)        desc="Python environment manager (for exo)" ;;
            ollama)    desc="Local model server" ;;
            node)      desc="Node.js + npm (for exo dashboard)" ;;
            exo)       desc="Distributed AI cluster" ;;
        esac
        
        echo -e "  ${BOLD}$i)${NC} $check ${CYAN}$comp${NC} - $desc"
        echo -e "         $status_str"
        i=$((i + 1))
    done
    
    echo ""
    echo -e "  ${BOLD}a)${NC} Select all"
    echo -e "  ${BOLD}n)${NC} Select none (core only)"
    echo -e "  ${BOLD}Enter)${NC} Proceed with selection"
    echo -e "  ${BOLD}q)${NC} Quit"
    echo ""
}

if [[ "$INTERACTIVE" == true ]]; then
    while true; do
        show_menu
        read -rsn1 key
        
        case "$key" in
            1) toggle_selected rust ;;
            2) toggle_selected rustyclaw ;;
            3) toggle_selected uv ;;
            4) toggle_selected ollama ;;
            5) toggle_selected node ;;
            6) toggle_selected exo ;;
            a|A)
                for comp in $ALL_COMPONENTS; do
                    set_selected "$comp" 1
                done
                ;;
            n|N)
                for comp in $ALL_COMPONENTS; do
                    set_selected "$comp" 0
                done
                for comp in $CORE_COMPONENTS; do
                    set_selected "$comp" 1
                done
                ;;
            q|Q)
                echo "Cancelled."
                exit 0
                ;;
            "")
                # Enter pressed — proceed
                break
                ;;
        esac
    done
    clear
fi

# ── Determine what to install ───────────────────────────────────────────────
should_install() {
    local comp="$1"
    
    # If interactive, use the selection state
    if [[ "$INTERACTIVE" == true ]]; then
        [[ "$(get_selected "$comp")" == "1" ]]
        return $?
    fi
    
    # If --only was specified
    if [[ -n "$ONLY" ]]; then
        echo "$ONLY" | grep -qw "$comp"
        return $?
    fi
    
    # If --all was specified
    if [[ "$INSTALL_ALL" == true ]]; then
        # Check skip list
        if [[ -n "$SKIP" ]]; then
            if echo "$SKIP" | grep -qw "$comp"; then
                return 1
            fi
        fi
        return 0
    fi
    
    # Non-interactive default: only core components
    if echo "$CORE_COMPONENTS" | grep -qw "$comp"; then
        # Check skip list
        if [[ -n "$SKIP" ]] && echo "$SKIP" | grep -qw "$comp"; then
            return 1
        fi
        return 0
    fi
    
    return 1
}

echo ""
echo -e "${BOLD}🦀🦞 RustyClaw Full Setup${NC}"
echo -e "${DIM}   OS: $OS ($ARCH)${NC}"
echo ""

INSTALLED=""
SKIPPED=""
FAILED=""

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
            INSTALLED="$INSTALLED rust"
        else
            warn "Rust $RUST_VER found but 1.85+ required — updating..."
            rustup update stable
            success "Rust updated to $(rustc --version | cut -d' ' -f2)"
            INSTALLED="$INSTALLED rust"
        fi
    else
        info "Installing Rust via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
        success "Rust $(rustc --version | cut -d' ' -f2) installed"
        INSTALLED="$INSTALLED rust"
    fi
else
    SKIPPED="$SKIPPED rust"
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
        INSTALLED="$INSTALLED rustyclaw"
    else
        err "RustyClaw binary not found in PATH after install"
        FAILED="$FAILED rustyclaw"
    fi
else
    SKIPPED="$SKIPPED rustyclaw"
fi

# ─────────────────────────────────────────────────────────────────────────────
# 4. uv (Python environment manager)
# ─────────────────────────────────────────────────────────────────────────────
if should_install uv; then
    step "uv (Python manager)"

    if has uv; then
        success "uv already installed ($(uv --version 2>/dev/null || echo 'found'))"
        INSTALLED="$INSTALLED uv"
    else
        info "Installing uv..."
        if curl -LsSf https://astral.sh/uv/install.sh | sh 2>&1; then
            # Add to PATH for this session
            export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
            if has uv; then
                success "uv $(uv --version 2>/dev/null) installed"
                INSTALLED="$INSTALLED uv"
            else
                err "uv installed but not found in PATH"
                FAILED="$FAILED uv"
            fi
        else
            err "Failed to install uv"
            FAILED="$FAILED uv"
        fi
    fi
else
    SKIPPED="$SKIPPED uv"
fi

# ─────────────────────────────────────────────────────────────────────────────
# 5. Ollama (local model server)
# ─────────────────────────────────────────────────────────────────────────────
if should_install ollama; then
    step "Ollama (local models)"

    if has ollama; then
        success "Ollama already installed ($(ollama --version 2>/dev/null || echo 'found'))"
        INSTALLED="$INSTALLED ollama"
    else
        info "Installing Ollama..."
        case "$PLATFORM" in
            macos)
                if has brew; then
                    brew install ollama 2>&1 && success "Ollama installed via Homebrew" && INSTALLED="$INSTALLED ollama" \
                        || { err "Homebrew install failed"; FAILED="$FAILED ollama"; }
                else
                    err "Homebrew required on macOS — install Homebrew first"
                    FAILED="$FAILED ollama"
                fi
                ;;
            linux)
                if curl -fsSL https://ollama.com/install.sh | sh 2>&1; then
                    success "Ollama installed"
                    INSTALLED="$INSTALLED ollama"
                else
                    err "Ollama install script failed"
                    FAILED="$FAILED ollama"
                fi
                ;;
        esac
    fi
else
    SKIPPED="$SKIPPED ollama"
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
        INSTALLED="$INSTALLED node"
    else
        info "Installing Node.js..."
        case "$PLATFORM" in
            macos)
                if has brew; then
                    brew install node 2>&1 && success "Node.js installed via Homebrew" && INSTALLED="$INSTALLED node" \
                        || { err "Homebrew install failed"; FAILED="$FAILED node"; }
                else
                    err "Homebrew required on macOS"
                    FAILED="$FAILED node"
                fi
                ;;
            linux)
                if has apt-get; then
                    # Try NodeSource LTS
                    if curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash - 2>&1 \
                        && sudo apt-get install -y nodejs 2>&1; then
                        success "Node.js installed via NodeSource"
                        INSTALLED="$INSTALLED node"
                    else
                        err "Node.js install failed"
                        FAILED="$FAILED node"
                    fi
                elif has dnf; then
                    sudo dnf install -y nodejs npm 2>&1 && success "Node.js installed" && INSTALLED="$INSTALLED node" \
                        || { err "Node.js install failed"; FAILED="$FAILED node"; }
                elif has pacman; then
                    sudo pacman -Sy --noconfirm nodejs npm 2>&1 && success "Node.js installed" && INSTALLED="$INSTALLED node" \
                        || { err "Node.js install failed"; FAILED="$FAILED node"; }
                else
                    warn "Installing Node.js via nvm..."
                    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash
                    export NVM_DIR="$HOME/.nvm"
                    # shellcheck disable=SC1091
                    . "$NVM_DIR/nvm.sh"
                    nvm install --lts
                    success "Node.js installed via nvm"
                    INSTALLED="$INSTALLED node"
                fi
                ;;
        esac
    fi
else
    SKIPPED="$SKIPPED node"
fi

# ─────────────────────────────────────────────────────────────────────────────
# 7. Exo (distributed AI cluster)
# ─────────────────────────────────────────────────────────────────────────────
if should_install exo; then
    step "Exo (distributed AI cluster)"

    # exo needs uv and node
    if ! has uv; then
        warn "uv is required for exo but not installed — skipping exo"
        FAILED="$FAILED exo"
    elif ! has node; then
        warn "Node.js is required for exo dashboard — skipping exo"
        FAILED="$FAILED exo"
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
            INSTALLED="$INSTALLED exo"
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
            INSTALLED="$INSTALLED exo"
        fi
    fi
else
    SKIPPED="$SKIPPED exo"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Summary
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BOLD}  Setup Summary${NC}"
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# Trim leading spaces
INSTALLED=$(echo "$INSTALLED" | xargs)
SKIPPED=$(echo "$SKIPPED" | xargs)
FAILED=$(echo "$FAILED" | xargs)

if [[ -n "$INSTALLED" ]]; then
    echo -e "  ${GREEN}Installed:${NC} $INSTALLED"
fi
if [[ -n "$SKIPPED" ]]; then
    echo -e "  ${DIM}Skipped:${NC}   $SKIPPED"
fi
if [[ -n "$FAILED" ]]; then
    echo -e "  ${RED}Failed:${NC}    $FAILED"
fi

echo ""
if [[ -z "$FAILED" ]]; then
    echo -e "  ${GREEN}${BOLD}✓ All done!${NC}"
else
    echo -e "  ${YELLOW}⚠ Some components failed — see above for details.${NC}"
fi

echo ""
echo -e "  ${BOLD}Next steps:${NC}"
echo "    1. rustyclaw onboard     # configure provider + vault"
echo "    2. rustyclaw tui         # launch the terminal UI"

# Show ollama hint if it was installed or available
if should_install ollama 2>/dev/null || has ollama; then
    echo "    3. ollama serve          # start local model server"
    echo "    4. ollama pull llama3    # download a model"
fi

# Show exo hint if it was installed
if should_install exo 2>/dev/null || [[ -d "$EXO_DIR" ]]; then
    echo ""
    echo -e "  ${BOLD}Exo:${NC}"
    echo "    cd $EXO_DIR && uv run exo   # start distributed cluster"
fi
echo ""
