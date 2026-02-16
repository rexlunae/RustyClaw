#!/usr/bin/env bash
#
# RustyClaw Tailscale Setup Script
# Automates Tailscale installation and configuration for remote gateway access
#

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Functions
print_header() {
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}  RustyClaw 🦞 Tailscale Setup${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

check_root() {
    if [[ $EUID -eq 0 ]]; then
        print_error "This script should not be run as root"
        print_info "Run as normal user (sudo will be requested when needed)"
        exit 1
    fi
}

detect_os() {
    if [[ -f /etc/os-release ]]; then
        . /etc/os-release
        OS=$ID
        OS_VERSION=$VERSION_ID
    elif [[ "$(uname)" == "Darwin" ]]; then
        OS="macos"
        OS_VERSION=$(sw_vers -productVersion)
    else
        print_error "Unsupported operating system"
        exit 1
    fi
}

check_tailscale() {
    if command -v tailscale &> /dev/null; then
        TAILSCALE_INSTALLED=true
        TAILSCALE_VERSION=$(tailscale version | head -1 | cut -d' ' -f2)
        print_success "Tailscale already installed (version $TAILSCALE_VERSION)"
        return 0
    else
        TAILSCALE_INSTALLED=false
        return 1
    fi
}

install_tailscale() {
    print_info "Installing Tailscale..."

    case $OS in
        ubuntu|debian|pop|linuxmint)
            curl -fsSL https://tailscale.com/install.sh | sh
            ;;
        fedora|centos|rhel)
            curl -fsSL https://tailscale.com/install.sh | sh
            ;;
        arch|manjaro)
            sudo pacman -S --noconfirm tailscale
            sudo systemctl enable --now tailscaled
            ;;
        macos)
            if command -v brew &> /dev/null; then
                brew install tailscale
            else
                print_error "Homebrew not found. Install from https://tailscale.com/download"
                exit 1
            fi
            ;;
        *)
            print_error "Automatic installation not supported for $OS"
            print_info "Please install from: https://tailscale.com/download"
            exit 1
            ;;
    esac

    if check_tailscale; then
        print_success "Tailscale installed successfully"
    else
        print_error "Failed to install Tailscale"
        exit 1
    fi
}

configure_tailscale() {
    local hostname="${1:-rustyclaw-gateway}"

    print_info "Configuring Tailscale..."

    # Check if already logged in
    if sudo tailscale status &> /dev/null; then
        print_success "Already authenticated with Tailscale"
        return 0
    fi

    # Authenticate
    print_info "Authenticating with Tailscale..."
    echo "  A browser window will open for authentication"

    if [[ "$hostname" == "rustyclaw-gateway" ]]; then
        sudo tailscale up --hostname="$hostname"
    else
        sudo tailscale up --hostname="$hostname"
    fi

    if sudo tailscale status &> /dev/null; then
        print_success "Tailscale authenticated successfully"
    else
        print_error "Failed to authenticate Tailscale"
        exit 1
    fi
}

get_tailscale_info() {
    TAILSCALE_IP=$(tailscale ip -4 2>/dev/null || echo "unknown")
    TAILSCALE_HOSTNAME=$(tailscale status --json 2>/dev/null | grep -o '"HostName":"[^"]*"' | cut -d'"' -f4 || echo "unknown")
    TAILSCALE_STATUS=$(sudo tailscale status 2>/dev/null | head -1 || echo "unknown")
}

show_connection_info() {
    echo
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}  Setup Complete! 🎉${NC}"
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo

    get_tailscale_info

    echo -e "${BLUE}Your Tailscale Information:${NC}"
    echo "  IP Address: $TAILSCALE_IP"
    if [[ "$TAILSCALE_HOSTNAME" != "unknown" ]]; then
        echo "  Hostname: $TAILSCALE_HOSTNAME"
    fi
    echo

    echo -e "${BLUE}Next Steps:${NC}"
    echo
    echo "1. Start RustyClaw gateway:"
    echo -e "   ${YELLOW}rustyclaw gateway --listen 0.0.0.0:8080${NC}"
    echo
    echo "2. Install Tailscale on your client device:"
    echo "   • Mobile: Download app from App Store / Play Store"
    echo "   • Desktop: https://tailscale.com/download"
    echo "   • Linux: curl -fsSL https://tailscale.com/install.sh | sh"
    echo
    echo "3. Connect from client device:"
    if [[ "$TAILSCALE_HOSTNAME" != "unknown" ]]; then
        echo -e "   ${YELLOW}rustyclaw connect --url ws://${TAILSCALE_HOSTNAME}:8080${NC}"
        echo -e "   ${YELLOW}# Or using IP: ws://${TAILSCALE_IP}:8080${NC}"
    else
        echo -e "   ${YELLOW}rustyclaw connect --url ws://${TAILSCALE_IP}:8080${NC}"
    fi
    echo

    echo -e "${BLUE}Optional Enhancements:${NC}"
    echo
    echo "• Enable MagicDNS for friendly names:"
    echo "  https://login.tailscale.com/admin/dns"
    echo
    echo "• Configure ACLs for access control:"
    echo "  https://login.tailscale.com/admin/acls"
    echo
    echo "• Enable Tailscale Funnel for public HTTPS:"
    echo -e "  ${YELLOW}tailscale funnel 8080${NC}"
    echo "  (Requires TOTP 2FA enabled in RustyClaw)"
    echo

    echo -e "${BLUE}Resources:${NC}"
    echo "  • Full guide: docs/TAILSCALE.md"
    echo "  • Tailscale docs: https://tailscale.com/kb/"
    echo
}

enable_systemd_service() {
    print_info "Setting up systemd service..."

    local service_name="rustyclaw-tailscale@${USER}.service"
    local service_file="/etc/systemd/system/${service_name}"

    if [[ -f "$service_file" ]]; then
        print_warning "Service already exists at $service_file"
        read -p "Overwrite? (y/N): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            return 0
        fi
    fi

    sudo tee "$service_file" > /dev/null <<EOF
[Unit]
Description=RustyClaw Gateway (Tailscale)
After=network-online.target tailscaled.service
Wants=network-online.target tailscaled.service

[Service]
Type=simple
User=$USER
ExecStartPre=/bin/sleep 5
ExecStart=$HOME/.cargo/bin/rustyclaw gateway --listen 0.0.0.0:8080
Restart=always
RestartSec=10
Environment="PATH=$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"

[Install]
WantedBy=multi-user.target
EOF

    sudo systemctl daemon-reload

    read -p "Enable and start service now? (y/N): " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        sudo systemctl enable --now "$service_name"
        print_success "Service enabled and started"

        echo
        print_info "Service commands:"
        echo "  Status:  sudo systemctl status $service_name"
        echo "  Logs:    sudo journalctl -u $service_name -f"
        echo "  Restart: sudo systemctl restart $service_name"
        echo "  Stop:    sudo systemctl stop $service_name"
    else
        print_info "Service created but not started"
        print_info "Enable with: sudo systemctl enable --now $service_name"
    fi
}

# Main script
main() {
    print_header

    check_root
    detect_os

    print_info "Detected OS: $OS $OS_VERSION"
    echo

    # Check if Tailscale is installed
    if ! check_tailscale; then
        read -p "Install Tailscale? (Y/n): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Nn]$ ]]; then
            install_tailscale
        else
            print_error "Tailscale is required. Install from: https://tailscale.com/download"
            exit 1
        fi
    fi

    # Get hostname
    echo
    read -p "Device hostname [rustyclaw-gateway]: " hostname
    hostname=${hostname:-rustyclaw-gateway}

    # Configure Tailscale
    configure_tailscale "$hostname"

    # Offer systemd service setup (Linux only)
    if [[ "$OS" != "macos" ]] && command -v systemctl &> /dev/null; then
        echo
        read -p "Create systemd service for auto-start? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            enable_systemd_service
        fi
    fi

    # Show connection info
    show_connection_info
}

# Run main function
main "$@"
