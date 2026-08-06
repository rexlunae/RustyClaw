#!/usr/bin/env bash
# ── RustyClaw Full Setup ─────────────────────────────────────────────────────
#
# Installs RustyClaw and optionally supporting tools:
#   • Rust toolchain (1.85+)
#   • RustyClaw (from local workspace or crates.io), including a desktop
#     launcher entry with the app icon (.desktop on Linux, .app on macOS)
#   • uv (Python environment manager)
#   • Ollama (local model server)
#   • Node.js + npm (for exo dashboard)
#   • Exo (distributed AI cluster)
#   • llama.cpp (llama-server for local GGUF inference)
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
ALL_COMPONENTS="rust rustyclaw uv ollama node exo llamacpp"
CORE_COMPONENTS="rust rustyclaw"
OPTIONAL_COMPONENTS="uv ollama node exo llamacpp"
SKIP=""
ONLY=""
EXO_DIR="${EXO_DIR:-$HOME/exo}"
RUSTYCLAW_FEATURES=""

# ── Clients ──────────────────────────────────────────────────────────────────
# The four binaries this repo installs. `cli` is the entry point the others are
# reached through (`rustyclaw tui`, `rustyclaw desktop`) and `gateway` is what
# it spawns to do the work, so those two are the default pair; the rest is
# taste. A headless box wants no desktop client, and building one costs a
# WebKitGTK toolchain it has no other use for.
ALL_CLIENTS="cli gateway tui desktop"
DEFAULT_CLIENTS="cli gateway tui desktop"
CLIENTS_ARG=""

# ── Optional features ────────────────────────────────────────────────────────
# Cargo features that are off unless asked for, plus `semantic-memory`, which
# the gateway turns on for itself. `web-tools` and `image-gen` are not here:
# they are default features of `rustyclaw-core`, and the crates that depend on
# it do not pass `default-features = false`, so no flag on this command line
# can turn them off. Listing them as choices would be a lie.
OPTIONAL_FEATURES="semantic-memory mcp browser matrix whatsapp signal-cli qr freenet"
WITH_FEATURES=""
WITHOUT_FEATURES=""
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
  --clients <client...>     Which RustyClaw binaries to build (default: all)
  --with <feature...>       Turn optional features on
  --without <feature...>    Turn optional features off
  --list-features           Show the clients and features you can choose from
  --features <features>     Extra raw cargo features, passed through verbatim
                            (e.g. "rustyclaw-core/matrix"). For anything the
                            --with list does not cover.
  --from-source             Build RustyClaw from local workspace instead of crates.io
  --force                   Overwrite existing RustyClaw binaries
  --help                    Show this help

Components: rust, rustyclaw, uv, ollama, node, exo
Clients:    cli, gateway, tui, desktop
Features:   semantic-memory (on), mcp, browser, matrix, whatsapp, signal-cli,
            qr, freenet          — see --list-features for what each one is

Examples:
  ./scripts/setup.sh                          # interactive — choose components
  ./scripts/setup.sh --all                    # install everything
  ./scripts/setup.sh --skip exo ollama        # skip exo and ollama
  ./scripts/setup.sh --only rust rustyclaw    # just Rust + RustyClaw
  ./scripts/setup.sh --from-source            # build from local checkout
  ./scripts/setup.sh --clients cli gateway    # headless box: no GUI clients
  ./scripts/setup.sh --with mcp matrix        # add MCP and Matrix support
  ./scripts/setup.sh --without semantic-memory  # skip the ONNX runtime dep
EOF
    exit 0
}

# ── Client and feature descriptions ─────────────────────────────────────────
client_desc() {
    case "$1" in
        cli)     echo "rustyclaw — the command-line entry point (spawns the rest)" ;;
        gateway) echo "rustyclaw-gateway — the daemon that runs the agent" ;;
        tui)     echo "rustyclaw-tui — terminal UI client" ;;
        desktop) echo "rustyclaw-desktop — GUI client (needs WebKitGTK + libxdo)" ;;
        *)       echo "" ;;
    esac
}

feature_desc() {
    case "$1" in
        semantic-memory) echo "vector memory recall (heavy: ONNX Runtime; no 32-bit ARM)" ;;
        mcp)             echo "Model Context Protocol servers" ;;
        browser)         echo "headless browser automation (pulls in chromiumoxide)" ;;
        matrix)          echo "Matrix messenger" ;;
        whatsapp)        echo "WhatsApp messenger" ;;
        signal-cli)      echo "Signal messenger via signal-cli" ;;
        qr)              echo "QR codes for device pairing" ;;
        freenet)         echo "Freenet tools (freenet, river, atlas)" ;;
        *)               echo "" ;;
    esac
}

print_feature_list() {
    cat <<EOF
🦀🦞 RustyClaw build options

Clients (--clients), all built by default:
EOF
    for c in $ALL_CLIENTS; do
        printf '  %-10s %s\n' "$c" "$(client_desc "$c")"
    done
    cat <<EOF

Optional features (--with / --without):
EOF
    for f in $OPTIONAL_FEATURES; do
        local mark="  "
        [[ "$f" == "semantic-memory" ]] && mark="* "
        printf '  %s%-16s %s\n' "$mark" "$f" "$(feature_desc "$f")"
    done
    cat <<'EOF'

  * on by default

Always compiled in: web-tools and image-gen. They are default features of
rustyclaw-core and the crates depending on it do not opt out, so nothing on
this command line can remove them.

Features apply to the CLI and the gateway. The TUI and desktop clients define
no features of their own. semantic-memory is a gateway-only feature — it is
the only component that runs it.
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
        --clients)
            shift
            while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                CLIENTS_ARG="$CLIENTS_ARG $1"; shift
            done
            ;;
        --with)
            shift
            while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                WITH_FEATURES="$WITH_FEATURES $1"; shift
            done
            ;;
        --without)
            shift
            while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                WITHOUT_FEATURES="$WITHOUT_FEATURES $1"; shift
            done
            ;;
        --list-features) print_feature_list ;;
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

# ── Desktop launcher integration ────────────────────────────────────────────
# Register the desktop client with the OS application launcher, with the app
# icon: a .desktop entry + hicolor icon on Linux, a ~/Applications/RustyClaw.app
# bundle on macOS. The icon is extracted from the installed binary itself
# (`rustyclaw-desktop --dump-icon`), so this works for both source and
# crates.io installs. Best-effort: failures warn but never abort setup.
install_desktop_launcher() {
    has rustyclaw-desktop || return 0
    local desktop_bin
    desktop_bin="$(command -v rustyclaw-desktop)"

    case "$PLATFORM" in
        linux)
            info "Registering RustyClaw in the applications menu..."
            local icon_dir="$HOME/.local/share/icons/hicolor/256x256/apps"
            local apps_dir="$HOME/.local/share/applications"
            mkdir -p "$icon_dir" "$apps_dir"

            if ! rustyclaw-desktop --dump-icon "$icon_dir/rustyclaw-desktop.png" 2>/dev/null; then
                warn "Could not export the app icon (rustyclaw-desktop too old?) — the menu entry will use a generic icon"
            fi

            # StartupWMClass matches the binary name, which GTK uses as the
            # WM_CLASS/app-id — this is what lets GNOME/KDE attach the icon
            # to the *running* window, not just the menu entry.
            cat > "$apps_dir/rustyclaw-desktop.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=RustyClaw
GenericName=AI Agent Client
Comment=Desktop client for the RustyClaw AI agent
Exec=$desktop_bin
Terminal=false
Icon=rustyclaw-desktop
Categories=Development;Utility;Network;
Keywords=AI;agent;chat;LLM;assistant;
StartupWMClass=rustyclaw-desktop
StartupNotify=true
EOF
            has update-desktop-database && update-desktop-database "$apps_dir" 2>/dev/null || true
            has gtk-update-icon-cache && gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
            success "Applications-menu entry installed (RustyClaw)"
            ;;
        macos)
            info "Installing RustyClaw.app to ~/Applications..."
            local app_dir="$HOME/Applications/RustyClaw.app"
            local app_version
            app_version="$(rustyclaw-desktop --version 2>/dev/null | awk '{print $2}')"
            mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"

            # Copy (not symlink) so Launch Services treats it as a real
            # bundled app; re-running setup after an upgrade refreshes it.
            cp -f "$desktop_bin" "$app_dir/Contents/MacOS/rustyclaw-desktop"

            if ! rustyclaw-desktop --dump-icon "$app_dir/Contents/Resources/icon.icns" 2>/dev/null; then
                warn "Could not export the app icon (rustyclaw-desktop too old?) — the Dock will use a generic icon"
            fi

            cat > "$app_dir/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleName</key>
    <string>RustyClaw</string>
    <key>CFBundleDisplayName</key>
    <string>RustyClaw</string>
    <key>CFBundleIdentifier</key>
    <string>io.rustyclaw.desktop</string>
    <key>CFBundleExecutable</key>
    <string>rustyclaw-desktop</string>
    <key>CFBundleIconFile</key>
    <string>icon</string>
    <key>CFBundleShortVersionString</key>
    <string>${app_version:-0.0.0}</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF
            # Nudge Launch Services to pick up the (new) bundle.
            touch "$app_dir"
            local lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
            [[ -x "$lsregister" ]] && "$lsregister" -f "$app_dir" 2>/dev/null || true
            success "RustyClaw.app installed (Launchpad/Spotlight → RustyClaw)"
            ;;
    esac
}

# ── Detect installed components ─────────────────────────────────────────────
# Using simple variables instead of associative arrays for bash 3.x compatibility
STATUS_rust="missing"; VERSION_rust=""
STATUS_rustyclaw="missing"; VERSION_rustyclaw=""
STATUS_uv="missing"; VERSION_uv=""
STATUS_ollama="missing"; VERSION_ollama=""
STATUS_node="missing"; VERSION_node=""
STATUS_exo="missing"; VERSION_exo=""
STATUS_llamacpp="missing"; VERSION_llamacpp=""

# Selection state (1=selected, 0=not selected)
SEL_rust=1; SEL_rustyclaw=1  # Core: selected by default
SEL_uv=0; SEL_ollama=0; SEL_node=0; SEL_exo=0; SEL_llamacpp=0  # Optional: not selected

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
    if has llama-server; then
        STATUS_llamacpp="installed"
        VERSION_llamacpp="$(llama-server --version 2>/dev/null || echo 'found')"
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

# ── Client / feature selection state ────────────────────────────────────────
# Feature names carry hyphens and variable names cannot, so every name is
# folded to underscores before it becomes part of one.
var_key() { echo "${1//-/_}"; }

get_client()    { eval "echo \${SEL_CLIENT_$(var_key "$1"):-0}"; }
set_client()    { eval "SEL_CLIENT_$(var_key "$1")=$2"; }
toggle_client() {
    if [[ "$(get_client "$1")" == "1" ]]; then set_client "$1" 0; else set_client "$1" 1; fi
}
want_client()   { [[ "$(get_client "$1")" == "1" ]]; }

get_feat()    { eval "echo \${SEL_FEAT_$(var_key "$1"):-0}"; }
set_feat()    { eval "SEL_FEAT_$(var_key "$1")=$2"; }
toggle_feat() {
    if [[ "$(get_feat "$1")" == "1" ]]; then set_feat "$1" 0; else set_feat "$1" 1; fi
}
want_feat()   { [[ "$(get_feat "$1")" == "1" ]]; }

# Defaults: every client, and the same features the build has today — the
# gateway's `semantic-memory` and nothing else. Running setup with no new
# flags must install exactly what it installed before.
for _c in $ALL_CLIENTS; do set_client "$_c" 0; done
for _c in $DEFAULT_CLIENTS; do set_client "$_c" 1; done
for _f in $OPTIONAL_FEATURES; do set_feat "$_f" 0; done
set_feat semantic-memory 1

known_client()  { echo "$ALL_CLIENTS" | grep -qw -- "$1"; }
known_feature() { echo "$OPTIONAL_FEATURES" | grep -qw -- "$1"; }

# An unrecognised name is a typo, and a typo that is quietly ignored produces
# a build missing exactly what was asked for — discovered much later, after
# the compile. Refuse it now, while it costs nothing.
if [[ -n "$CLIENTS_ARG" ]]; then
    for _c in $ALL_CLIENTS; do set_client "$_c" 0; done
    for _c in $CLIENTS_ARG; do
        if [[ "$_c" == "all" ]]; then
            for _a in $ALL_CLIENTS; do set_client "$_a" 1; done
        elif known_client "$_c"; then
            set_client "$_c" 1
        else
            err "Unknown client: $_c"
            err "Known clients: $ALL_CLIENTS"
            exit 1
        fi
    done
fi

for _f in $WITH_FEATURES; do
    if [[ "$_f" == "all" ]]; then
        for _a in $OPTIONAL_FEATURES; do set_feat "$_a" 1; done
    elif known_feature "$_f"; then
        set_feat "$_f" 1
    else
        err "Unknown feature: $_f"
        err "Known features: $OPTIONAL_FEATURES"
        err "For a feature this script does not list, use --features to pass it through."
        exit 1
    fi
done

for _f in $WITHOUT_FEATURES; do
    if [[ "$_f" == "all" ]]; then
        for _a in $OPTIONAL_FEATURES; do set_feat "$_a" 0; done
    elif known_feature "$_f"; then
        set_feat "$_f" 0
    else
        err "Unknown feature: $_f"
        err "Known features: $OPTIONAL_FEATURES"
        exit 1
    fi
done

show_menu() {
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
            llamacpp)  desc="llama.cpp local GGUF inference" ;;
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
            7) toggle_selected llamacpp ;;
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
    echo ""
fi

# ── Interactive client + feature selection ──────────────────────────────────
# Only worth asking when RustyClaw itself is being built; these choices are
# what goes into the cargo invocation and nothing else uses them.
show_build_menu() {
    echo -e "${BOLD}🦀🦞 RustyClaw build options${NC}"
    echo ""
    echo -e "${BOLD}Clients to build:${NC}"
    local i=1
    for c in $ALL_CLIENTS; do
        local check="[ ]"
        [[ "$(get_client "$c")" == "1" ]] && check="[${GREEN}✓${NC}]"
        echo -e "  ${BOLD}$i)${NC} $check ${CYAN}$c${NC} - $(client_desc "$c")"
        i=$((i + 1))
    done

    echo ""
    echo -e "${BOLD}Optional features:${NC}"
    echo -e "${DIM}(web-tools and image-gen are always on — they are crate defaults)${NC}"
    local letters="asdfghjk"
    local n=0
    for f in $OPTIONAL_FEATURES; do
        local check="[ ]"
        [[ "$(get_feat "$f")" == "1" ]] && check="[${GREEN}✓${NC}]"
        echo -e "  ${BOLD}${letters:$n:1})${NC} $check ${CYAN}$f${NC} - $(feature_desc "$f")"
        n=$((n + 1))
    done

    echo ""
    echo -e "  ${BOLD}Enter)${NC} Build with this selection"
    echo -e "  ${BOLD}q)${NC} Cancel"
    echo ""
}

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

if [[ "$INTERACTIVE" == true ]] && should_install rustyclaw; then
    while true; do
        show_build_menu
        read -rsn1 key

        # Features are keyed off the list itself rather than a hard-coded
        # case arm, so adding one to OPTIONAL_FEATURES needs no change here.
        matched=false
        letters="asdfghjk"
        n=0
        for f in $OPTIONAL_FEATURES; do
            if [[ "$key" == "${letters:$n:1}" ]]; then
                toggle_feat "$f"
                matched=true
                break
            fi
            n=$((n + 1))
        done
        [[ "$matched" == true ]] && continue

        case "$key" in
            1) toggle_client cli ;;
            2) toggle_client gateway ;;
            3) toggle_client tui ;;
            4) toggle_client desktop ;;
            x|X|q|Q)
                echo "Cancelled."
                exit 0
                ;;
            "")
                break
                ;;
        esac
    done
    echo ""
fi

# A `rustyclaw` component with nothing under it builds nothing and then
# reports success, which is worse than saying so.
if should_install rustyclaw; then
    SELECTED_CLIENTS=""
    for _c in $ALL_CLIENTS; do
        want_client "$_c" && SELECTED_CLIENTS="$SELECTED_CLIENTS $_c"
    done
    if [[ -z "${SELECTED_CLIENTS// /}" ]]; then
        err "No RustyClaw clients selected — nothing to build."
        err "Pick at least one of: $ALL_CLIENTS"
        exit 1
    fi
fi

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
                sudo apt-get install -y -qq \
                    build-essential pkg-config libssl-dev \
                    libglib2.0-dev libgtk-3-dev libwebkit2gtk-4.1-dev libxdo-dev \
                    2>/dev/null || true
                success "Debian/Ubuntu build deps ready"
            elif has dnf; then
                info "Installing Fedora/RHEL build deps..."
                sudo dnf install -y -q \
                    gcc pkg-config openssl-devel \
                    glib2-devel gtk3-devel webkit2gtk4.1-devel xdotool-devel \
                    2>/dev/null || true
                success "Fedora/RHEL build deps ready"
            elif has pacman; then
                info "Installing Arch build deps..."
                # webkit2gtk-4.1 (not webkit2gtk) is the current Arch package
                # name for WebKitGTK 4.1; it provides both webkit2gtk-4.1.pc
                # and javascriptcoregtk-4.1.pc
                sudo pacman -Sy --noconfirm --needed \
                    base-devel openssl pkgconf glib2 gtk3 webkit2gtk-4.1 xdotool \
                    2>/dev/null || true
                success "Arch build deps ready"
            elif has apk; then
                info "Installing Alpine build deps..."
                sudo apk add --no-cache \
                    build-base openssl-dev pkgconfig \
                    glib-dev gtk+3.0-dev webkit2gtk-dev xdotool-dev \
                    2>/dev/null || true
                success "Alpine build deps ready"
            else
                warn "Unknown distro — you may need gcc, pkg-config, libssl-dev, glib, gtk3, webkit2gtk, and libxdo development packages"
            fi

            if has pkg-config; then
                if pkg-config --exists "glib-2.0 >= 2.70"; then
                    success "glib-2.0 development package detected"
                else
                    warn "glib-2.0.pc not found by pkg-config; install your distro's GLib dev package and set PKG_CONFIG_PATH if needed"
                fi

                # Everything below here exists for the desktop client alone.
                # Demanding a WebKitGTK toolchain from someone who asked for
                # `--clients cli gateway` is asking them to install a browser
                # engine to run a headless daemon — and the check used to
                # abort setup outright when they would not.
                if want_client desktop; then
                    if pkg-config --exists "gdk-3.0 >= 3.22"; then
                        success "gdk-3.0 development package detected"
                    else
                        warn "gdk-3.0.pc not found; install your distro's GTK3 development package"
                    fi

                    if pkg-config --exists "webkit2gtk-4.1"; then
                        success "webkit2gtk-4.1 development package detected"
                    else
                        warn "webkit2gtk-4.1.pc not found; install your distro's WebKitGTK 4.1 development package"
                    fi

                    # The desktop client (rustyclaw-desktop) hard-requires
                    # JavaScriptCore + WebKitGTK 4.1 + libxdo; if they're
                    # missing the cargo build fails only after several minutes
                    # of compile time, so fail fast here with install
                    # instructions instead.
                    if pkg-config --exists "javascriptcoregtk-4.1" && pkg-config --exists "libxdo"; then
                        success "JavaScriptCore + libxdo development packages detected"
                    else
                        err "WebKitGTK/JavaScriptCore 4.1 and libxdo dev packages not found — rustyclaw-desktop cannot build without them"
                        err "Install them, e.g.:"
                        err "  Debian/Ubuntu: sudo apt-get install libwebkit2gtk-4.1-dev libxdo-dev"
                        err "  Fedora:        sudo dnf install webkit2gtk4.1-devel xdotool-devel"
                        err "  Arch:          sudo pacman -S webkit2gtk-4.1 xdotool"
                        err "  Alpine:        sudo apk add webkit2gtk-dev xdotool-dev"
                        err ""
                        err "Or build without the GUI client: --clients cli gateway tui"
                        exit 1
                    fi
                else
                    info "Skipping GTK/WebKit checks — no desktop client selected"
                fi
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

    FORCE_FLAG=""
    [[ "$FORCE" == true ]] && FORCE_FLAG="--force"

    # ── Compose the cargo feature flags ──────────────────────────────────
    # The features live on `rustyclaw-core`, so they are named through it.
    # That spelling resolves from any crate that depends on core, which is
    # both the CLI and the gateway, so one name serves both.
    join_features() {
        echo "$1" | tr ' ' '\n' | sed '/^$/d' | tr '\n' ',' | sed 's/,$//'
    }

    CLI_FEATURES=""
    GATEWAY_FEATURES=""
    for f in $OPTIONAL_FEATURES; do
        want_feat "$f" || continue
        case "$f" in
            # Handled entirely through the gateway's defaults below — it is
            # already one of them, so naming it here would add a flag that
            # changes nothing. The CLI never gets it: the gateway is the only
            # component that runs semantic memory, and it costs an ONNX
            # Runtime build.
            semantic-memory) ;;
            *)
                CLI_FEATURES="$CLI_FEATURES rustyclaw-core/$f"
                GATEWAY_FEATURES="$GATEWAY_FEATURES rustyclaw-core/$f"
                ;;
        esac
    done

    # Whatever `--features` was handed, verbatim and unexamined — it exists
    # for the features this script does not know about. `tui`/`desktop` are
    # CLI-only names the gateway would reject, so they stay out of its list.
    for f in $(echo "$RUSTYCLAW_FEATURES" | tr ',' ' '); do
        CLI_FEATURES="$CLI_FEATURES $f"
        case "$f" in
            tui|desktop) ;;
            *) GATEWAY_FEATURES="$GATEWAY_FEATURES $f" ;;
        esac
    done

    # `semantic-memory` and `image-gen` are the gateway's *own* default
    # features, so declining the first means declining defaults — which takes
    # the second down with it. Ask for it back by name, or turning off vector
    # memory would quietly turn off image generation too.
    GATEWAY_NO_DEFAULT=""
    if ! want_feat semantic-memory; then
        GATEWAY_NO_DEFAULT="--no-default-features"
        GATEWAY_FEATURES="$GATEWAY_FEATURES image-gen"
    fi

    CLI_FEATURES="$(join_features "$CLI_FEATURES")"
    GATEWAY_FEATURES="$(join_features "$GATEWAY_FEATURES")"

    # The CLI spawns the gateway for `gateway start/restart`. Installing one
    # without the other is legitimate — a workstation talking to a gateway on
    # another host wants exactly that — but it is worth saying out loud,
    # because the failure otherwise turns up much later as "Could not find the
    # rustyclaw-gateway binary".
    if want_client cli && ! want_client gateway; then
        warn "Installing the CLI without the gateway daemon"
        warn "  'rustyclaw gateway start' will need a gateway on this machine or another"
    fi

    info "Clients:  $(echo "$SELECTED_CLIENTS" | sed 's/^ *//')"
    info "Features: ${CLI_FEATURES:-none beyond crate defaults}"
    if [[ -n "$GATEWAY_NO_DEFAULT" ]]; then
        info "Gateway:  building without semantic memory"
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
        CRATES_DIR="${INSTALL_PATH%/rustyclaw-cli}"
        GATEWAY_PATH="$CRATES_DIR/rustyclaw-gateway"
        TUI_PATH="$CRATES_DIR/rustyclaw-tui"
        DESKTOP_PATH="$CRATES_DIR/rustyclaw-desktop"

        if want_client cli; then
            info "Installing CLI (rustyclaw)..."
            if [[ -n "$CLI_FEATURES" ]]; then
                cargo install --path "$INSTALL_PATH" --features "$CLI_FEATURES" $FORCE_FLAG
            else
                cargo install --path "$INSTALL_PATH" $FORCE_FLAG
            fi
        fi
        if want_client gateway; then
            info "Installing gateway daemon (rustyclaw-gateway)..."
            if [[ -n "$GATEWAY_FEATURES" ]]; then
                cargo install --path "$GATEWAY_PATH" $GATEWAY_NO_DEFAULT --features "$GATEWAY_FEATURES" $FORCE_FLAG
            else
                cargo install --path "$GATEWAY_PATH" $GATEWAY_NO_DEFAULT $FORCE_FLAG
            fi
        fi
        # The TUI and desktop clients define no features of their own; the
        # `rustyclaw tui`/`desktop` subcommands spawn them.
        if want_client tui; then
            info "Installing terminal UI client (rustyclaw-tui)..."
            cargo install --path "$TUI_PATH" $FORCE_FLAG
        fi
        if want_client desktop; then
            info "Installing desktop GUI client (rustyclaw-desktop)..."
            cargo install --path "$DESKTOP_PATH" $FORCE_FLAG
        fi
    else
        info "Installing from crates.io..."
        if want_client cli; then
            info "Installing CLI (rustyclaw)..."
            if [[ -n "$CLI_FEATURES" ]]; then
                cargo install rustyclaw --features "$CLI_FEATURES" $FORCE_FLAG
            else
                cargo install rustyclaw $FORCE_FLAG
            fi
        fi
        if want_client gateway; then
            info "Installing gateway daemon (rustyclaw-gateway)..."
            if [[ -n "$GATEWAY_FEATURES" ]]; then
                cargo install rustyclaw-gateway $GATEWAY_NO_DEFAULT --features "$GATEWAY_FEATURES" $FORCE_FLAG
            else
                cargo install rustyclaw-gateway $GATEWAY_NO_DEFAULT $FORCE_FLAG
            fi
        fi
        if want_client tui; then
            info "Installing terminal UI client (rustyclaw-tui)..."
            cargo install rustyclaw-tui $FORCE_FLAG
        fi
        if want_client desktop; then
            info "Installing desktop GUI client (rustyclaw-desktop)..."
            cargo install rustyclaw-desktop $FORCE_FLAG
        fi
    fi

    # Only what was asked for is checked. A client nobody selected is missing
    # on purpose, and reporting that as a problem trains people to ignore the
    # warnings that do mean something.
    if want_client cli; then
        if has rustyclaw; then
            success "RustyClaw $(rustyclaw --version 2>/dev/null || echo 'installed')"
            INSTALLED="$INSTALLED rustyclaw"
        else
            err "RustyClaw binary not found in PATH after install"
            FAILED="$FAILED rustyclaw"
        fi
    fi
    if want_client gateway; then
        if has rustyclaw-gateway; then
            success "RustyClaw gateway daemon installed"
        else
            err "rustyclaw-gateway not found in PATH after install — 'gateway start/restart' will fail until it is installed"
            FAILED="$FAILED rustyclaw-gateway"
        fi
    fi
    if want_client tui; then
        if has rustyclaw-tui; then
            success "RustyClaw terminal UI client installed"
        else
            err "rustyclaw-tui not found in PATH after install — 'rustyclaw tui' will fail until it is installed"
            FAILED="$FAILED rustyclaw-tui"
        fi
    fi
    if want_client desktop; then
        if has rustyclaw-desktop; then
            success "RustyClaw desktop client installed"
            install_desktop_launcher
        else
            err "rustyclaw-desktop not found in PATH after install — 'rustyclaw desktop' will fail (desktop also needs GTK/WebKit dev libs on Linux)"
            FAILED="$FAILED rustyclaw-desktop"
        fi
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
# 8. llama.cpp (llama-server for local GGUF inference)
# ─────────────────────────────────────────────────────────────────────────────
if should_install llamacpp; then
    step "llama.cpp (llama-server)"

    if has llama-server; then
        success "llama-server already installed ($(llama-server --version 2>/dev/null || echo 'found'))"
        INSTALLED="$INSTALLED llamacpp"
    else
        info "Installing llama.cpp (llama-server)..."
        case "$PLATFORM" in
            macos)
                if has brew; then
                    brew install llama.cpp 2>&1 && success "llama.cpp installed via Homebrew" && INSTALLED="$INSTALLED llamacpp" \
                        || { err "Homebrew install failed"; FAILED="$FAILED llamacpp"; }
                else
                    err "Homebrew required on macOS — install Homebrew first"
                    FAILED="$FAILED llamacpp"
                fi
                ;;
            linux)
                # Try to download prebuilt release from GitHub
                LLAMACPP_VER=$(curl -fsSL "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest" 2>/dev/null \
                    | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//')
                if [[ -n "$LLAMACPP_VER" ]]; then
                    ARCH=$(uname -m)
                    case "$ARCH" in
                        x86_64)  LLAMACPP_ARCH="ubuntu-x64" ;;
                        aarch64) LLAMACPP_ARCH="ubuntu-arm64" ;;
                        *)       LLAMACPP_ARCH="" ;;
                    esac
                    if [[ -n "$LLAMACPP_ARCH" ]]; then
                        LLAMACPP_URL="https://github.com/ggml-org/llama.cpp/releases/download/${LLAMACPP_VER}/llama-${LLAMACPP_VER}-bin-${LLAMACPP_ARCH}.zip"
                        LLAMACPP_TMP="/tmp/llamacpp-release.zip"
                        if curl -fsSL "$LLAMACPP_URL" -o "$LLAMACPP_TMP" 2>/dev/null; then
                            sudo unzip -o "$LLAMACPP_TMP" -d /usr/local/bin/ "*/llama-server" 2>/dev/null \
                                && sudo chmod +x /usr/local/bin/llama-server \
                                && success "llama-server installed to /usr/local/bin" \
                                && INSTALLED="$INSTALLED llamacpp" \
                                || { err "Failed to extract llama-server"; FAILED="$FAILED llamacpp"; }
                            rm -f "$LLAMACPP_TMP"
                        else
                            warn "Could not download prebuilt llama.cpp; try building from source"
                            FAILED="$FAILED llamacpp"
                        fi
                    else
                        warn "Unsupported architecture ($ARCH) for prebuilt llama.cpp"
                        FAILED="$FAILED llamacpp"
                    fi
                else
                    warn "Could not determine latest llama.cpp release"
                    FAILED="$FAILED llamacpp"
                fi
                ;;
        esac
    fi
else
    SKIPPED="$SKIPPED llamacpp"
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
