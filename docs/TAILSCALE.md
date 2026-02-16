# Tailscale Integration Guide

Tailscale provides the easiest and most secure way to access your RustyClaw gateway remotely. It creates a private mesh VPN network that just works - no port forwarding, no firewall rules, no complex configuration.

## Why Tailscale for RustyClaw?

**Benefits:**
- ✅ Zero configuration networking - works everywhere
- ✅ End-to-end encrypted - military-grade security
- ✅ Automatic NAT traversal - works behind any firewall
- ✅ Free for personal use - up to 100 devices
- ✅ Cross-platform - Linux, Mac, Windows, iOS, Android
- ✅ MagicDNS - use friendly names instead of IPs
- ✅ Fast - direct peer-to-peer connections when possible

**Use cases:**
- Access gateway from phone while traveling
- Connect from coffee shop, hotel, or anywhere
- Share access with team members (controlled ACLs)
- Access multiple RustyClaw instances across locations
- Bypass restrictive corporate/school firewalls

---

## Quick Start (5 minutes)

### 1. Install Tailscale on Gateway Server

**Linux (Ubuntu/Debian):**
```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

**macOS:**
```bash
brew install tailscale
sudo tailscale up
```

**Windows:**
Download from [tailscale.com/download](https://tailscale.com/download) and run installer.

### 2. Start RustyClaw Gateway

```bash
rustyclaw gateway --listen 0.0.0.0:8080
```

> **Note:** Use `0.0.0.0` to listen on all interfaces (including Tailscale)

### 3. Get Your Tailscale IP

```bash
tailscale ip -4
# Example output: 100.101.102.103
```

### 4. Connect from Client Device

**Install Tailscale on client:**
- **Phone:** Download Tailscale app from App Store / Play Store
- **Laptop:** Install from [tailscale.com/download](https://tailscale.com/download)
- **Another server:** `curl -fsSL https://tailscale.com/install.sh | sh`

**Connect to gateway:**
```bash
# Using Tailscale IP
rustyclaw connect --url ws://100.101.102.103:8080

# Or with MagicDNS (if enabled)
rustyclaw connect --url ws://your-server:8080
```

**For TUI from mobile (via SSH):**
```bash
ssh user@100.101.102.103
rustyclaw chat
```

---

## Configuration

### Enable MagicDNS (Recommended)

MagicDNS assigns friendly names to your devices:

1. Go to [https://login.tailscale.com/admin/dns](https://login.tailscale.com/admin/dns)
2. Enable **MagicDNS**
3. Your devices get names like: `server.tail-scale.ts.net`

Now connect using:
```bash
rustyclaw connect --url ws://server:8080
```

### Configure RustyClaw for Tailscale

Edit `~/.rustyclaw/config.toml`:

```toml
# Optional: Bind to specific Tailscale interface
[gateway]
listen = "100.101.102.103:8080"  # Your Tailscale IP

# Or bind to all interfaces (includes Tailscale)
# listen = "0.0.0.0:8080"

# Enable TLS for extra security (optional with Tailscale)
[tls]
enabled = false  # Tailscale already encrypts everything
```

---

## Advanced Features

### Tailscale Funnel (Public HTTPS Access)

Tailscale Funnel lets you expose your gateway to the internet with automatic HTTPS:

```bash
# Enable funnel for port 8080
tailscale funnel 8080

# Your gateway is now available at:
# https://your-device.tail-scale.ts.net:8080
```

**Use cases:**
- Share temporary access with someone without Tailscale
- Access from devices where you can't install Tailscale
- Webhook endpoints for external services

**Security note:** Funnel bypasses Tailscale authentication - ensure TOTP 2FA is enabled!

```toml
# In config.toml - REQUIRED when using Funnel
totp_enabled = true
```

### Access Control Lists (ACLs)

Control who can access your gateway:

1. Go to [https://login.tailscale.com/admin/acls](https://login.tailscale.com/admin/acls)
2. Edit ACLs (HuJSON format):

```json
{
  "acls": [
    {
      // Allow yourself full access
      "action": "accept",
      "src": ["your-email@example.com"],
      "dst": ["your-server:*"]
    },
    {
      // Allow team member access to gateway only
      "action": "accept",
      "src": ["teammate@example.com"],
      "dst": ["your-server:8080"]
    }
  ]
}
```

### Subnet Routing

Access your entire home network through Tailscale:

**On gateway server:**
```bash
# Enable IP forwarding
echo 'net.ipv4.ip_forward = 1' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p

# Advertise subnet
sudo tailscale up --advertise-routes=192.168.1.0/24

# Approve in admin console:
# https://login.tailscale.com/admin/machines
```

Now you can access any device on your home network (192.168.1.x) from anywhere!

### Exit Node (Optional)

Make all your internet traffic go through the gateway server:

```bash
# On server: advertise as exit node
sudo tailscale up --advertise-exit-node

# On client: use server as exit node
tailscale up --exit-node=your-server
```

---

## Mobile Access

### iOS/Android App

1. Install Tailscale from App Store / Play Store
2. Sign in with your account
3. Connect to your Tailscale network
4. Use SSH client or web browser:

**Option A: SSH to gateway**
```
Host: 100.101.102.103 (or server.tail-scale.ts.net)
Port: 22
Username: your-username
```

Run `rustyclaw chat` after connecting.

**Option B: Use web-ui (if enabled)**
```
http://100.101.102.103:8080/web-ui/
```

### Shortcuts / Automation

**iOS Shortcut example:**
```
SSH into server
Run Command: rustyclaw chat
```

**Android Termux:**
```bash
pkg install openssh
ssh user@server.tail-scale.ts.net -t "rustyclaw chat"
```

---

## Tailscale + RustyClaw Best Practices

### 1. Use MagicDNS
Enable MagicDNS so you can use friendly names instead of IPs:
- `ws://gateway:8080` instead of `ws://100.101.102.103:8080`

### 2. Set Device Name
```bash
sudo tailscale up --hostname=rustyclaw-gateway
```

### 3. Enable Key Expiry Warnings
Go to [Settings](https://login.tailscale.com/admin/settings/keys) and enable email notifications before keys expire.

### 4. Use Tags for Organization
Tag your devices in the admin console:
- `tag:gateway` - RustyClaw gateways
- `tag:client` - Client devices
- `tag:prod` - Production instances
- `tag:dev` - Development instances

### 5. Monitor Connections
```bash
# Check connected peers
tailscale status

# Show detailed peer info
tailscale status --peers
```

### 6. Security Hardening
```toml
# config.toml - Defense in depth
totp_enabled = true  # Always use 2FA

[tls]
enabled = false  # Not needed with Tailscale (already encrypted)

[pairing]
enabled = true  # Require pairing for messenger DMs
```

---

## Troubleshooting

### Can't Connect to Gateway

**Check Tailscale status:**
```bash
tailscale status
# Ensure both devices show as online
```

**Verify gateway is listening:**
```bash
# On server
ss -tlnp | grep 8080
# Should show rustyclaw listening
```

**Test connectivity:**
```bash
# From client device
curl http://100.101.102.103:8080/health
```

### Connection Works But Gateway Not Responding

**Check firewall (if enabled):**
```bash
# Allow on local firewall
sudo ufw allow from 100.64.0.0/10 to any port 8080
```

**Verify gateway is running:**
```bash
# On server
ps aux | grep rustyclaw
journalctl -u rustyclaw-gateway -f
```

### Slow Performance

**Check connection type:**
```bash
tailscale status
# Look for "direct" connections - faster than "relay"
```

**Enable derp.tailscale.com:**
Ensure [https://login.tailscale.com/admin/dns](https://login.tailscale.com/admin/dns) has DERP enabled.

**Use nearest DERP region:**
```bash
tailscale netcheck
# Shows latency to DERP regions
```

### Device Not Showing in Admin Console

**Re-authenticate:**
```bash
sudo tailscale up --force-reauth
```

**Check login status:**
```bash
tailscale status
# Should show "Logged in as: your-email@example.com"
```

### IP Changes After Reboot

Tailscale IPs are stable but can change rarely. Use MagicDNS names instead:
```bash
# Don't hardcode IPs
ws://100.101.102.103:8080  # ❌ Bad

# Use MagicDNS names
ws://gateway:8080  # ✅ Good
```

---

## Multi-Gateway Setup

Running multiple RustyClaw instances? Here's how to organize them:

### Setup
```bash
# Gateway 1 (Personal)
tailscale up --hostname=rustyclaw-personal
rustyclaw gateway --listen 0.0.0.0:8080

# Gateway 2 (Work)
tailscale up --hostname=rustyclaw-work
rustyclaw gateway --listen 0.0.0.0:8081

# Gateway 3 (Development)
tailscale up --hostname=rustyclaw-dev
rustyclaw gateway --listen 0.0.0.0:8082
```

### Connect
```bash
# Personal
rustyclaw connect --url ws://rustyclaw-personal:8080

# Work
rustyclaw connect --url ws://rustyclaw-work:8081

# Dev
rustyclaw connect --url ws://rustyclaw-dev:8082
```

### ACL Example
```json
{
  "acls": [
    {
      "action": "accept",
      "src": ["tag:personal-clients"],
      "dst": ["tag:rustyclaw-personal:8080"]
    },
    {
      "action": "accept",
      "src": ["tag:work-clients"],
      "dst": ["tag:rustyclaw-work:8081"]
    }
  ]
}
```

---

## Comparison: Tailscale vs Other Methods

| Feature | Tailscale | Port Forwarding | ngrok | VPN (WireGuard) |
|---------|-----------|-----------------|-------|-----------------|
| Setup Time | 5 min | 30+ min | 10 min | 60+ min |
| Security | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ |
| Works Anywhere | Yes | No | Yes | Yes |
| Free Tier | 100 devices | Yes | Limited | Yes |
| Mobile Support | Excellent | N/A | Good | Good |
| Complexity | Very Easy | Hard | Easy | Hard |
| Speed | Fast | Fast | Medium | Fast |
| Reliability | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐⭐ |

**Winner:** Tailscale offers the best balance of ease-of-use, security, and features.

---

## Automation Scripts

### Auto-start Script (systemd)

Create `/etc/systemd/system/rustyclaw-tailscale.service`:

```ini
[Unit]
Description=RustyClaw Gateway (Tailscale)
After=network-online.target tailscaled.service
Wants=network-online.target tailscaled.service

[Service]
Type=simple
User=%i
ExecStartPre=/bin/sleep 5
ExecStart=/home/%i/.cargo/bin/rustyclaw gateway --listen 0.0.0.0:8080
Restart=always
RestartSec=10
Environment="PATH=/home/%i/.cargo/bin:/usr/local/bin:/usr/bin:/bin"

[Install]
WantedBy=multi-user.target
```

Enable:
```bash
sudo systemctl enable --now rustyclaw-tailscale@$USER
```

### Health Check Script

Save as `~/scripts/check-gateway.sh`:

```bash
#!/bin/bash

TAILSCALE_IP=$(tailscale ip -4)
HEALTH_URL="http://${TAILSCALE_IP}:8080/health"

if ! curl -sf "$HEALTH_URL" > /dev/null; then
    echo "Gateway unhealthy, restarting..."
    sudo systemctl restart rustyclaw-tailscale@$USER

    # Send notification (optional)
    curl -X POST https://ntfy.sh/your-topic \
        -d "RustyClaw gateway restarted"
fi
```

Run every 5 minutes:
```bash
crontab -e
# Add:
*/5 * * * * /home/username/scripts/check-gateway.sh
```

---

## FAQ

### Q: Do I need TLS if I'm using Tailscale?

**A:** No! Tailscale already encrypts all traffic with WireGuard. You can disable TLS in RustyClaw config to simplify setup:
```toml
[tls]
enabled = false
```

### Q: Can I use both Tailscale and public access (ngrok)?

**A:** Yes! Run two gateway instances:
```bash
# Local Tailscale access
rustyclaw gateway --listen 100.101.102.103:8080

# Public ngrok access (with TLS + 2FA)
rustyclaw gateway --listen 127.0.0.1:8081
ngrok http 8081
```

### Q: What's the bandwidth usage?

**A:** Minimal:
- Idle: ~1KB/s (keepalives)
- Active chat: ~10-50KB per message
- Typical daily usage: <50MB

### Q: Can I share access with non-Tailscale users?

**A:** Yes, use **Tailscale Funnel**:
```bash
tailscale funnel 8080
```

Gives public HTTPS URL: `https://your-device.tail-scale.ts.net`

**Important:** Enable TOTP 2FA when using Funnel!

### Q: Does this work in China / behind corporate firewalls?

**A:** Usually yes! Tailscale uses multiple techniques to traverse firewalls:
- Direct connections (when possible)
- STUN/TURN relay servers
- DERP relay servers (last resort)

Success rate: >99% in practice.

### Q: How do I revoke access?

**A:** Three options:
1. **Remove device:** [https://login.tailscale.com/admin/machines](https://login.tailscale.com/admin/machines) → Delete
2. **Disable key:** Admin console → Keys → Disable
3. **Update ACLs:** Remove from ACL rules

Changes take effect immediately.

### Q: Can I use my own DERP relay servers?

**A:** Yes! See [Tailscale DERP documentation](https://tailscale.com/kb/1118/custom-derp-servers).

Good for:
- Corporate compliance (all traffic through your servers)
- Reduced latency in specific regions
- Maximum privacy (self-hosted relay)

---

## Resources

- [Tailscale Official Docs](https://tailscale.com/kb/)
- [Tailscale Blog](https://tailscale.com/blog/)
- [WireGuard Protocol](https://www.wireguard.com/)
- [RustyClaw Security Guide](./SECURITY.md)
- [RustyClaw Remote Access](./REMOTE_ACCESS.md)

---

## Support

**Tailscale issues:**
- Email: support@tailscale.com
- Forum: [https://forum.tailscale.com](https://forum.tailscale.com)

**RustyClaw + Tailscale integration:**
- GitHub Issues: [https://github.com/your-repo/rustyclaw/issues](https://github.com/your-repo/rustyclaw/issues)
- Discussions: [https://github.com/your-repo/rustyclaw/discussions](https://github.com/your-repo/rustyclaw/discussions)

---

## Next Steps

1. ✅ Install Tailscale on gateway server
2. ✅ Start RustyClaw gateway
3. ✅ Install Tailscale on client devices
4. ✅ Connect and test
5. ⭐ Enable MagicDNS for friendly names
6. ⭐ Configure ACLs for access control
7. ⭐ Set up health monitoring
8. ⭐ Configure auto-start (systemd)

**Happy remote RustyClaw usage! 🦞✨**
