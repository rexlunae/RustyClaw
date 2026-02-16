# Remote Gateway Access Guide

This guide explains how to access your RustyClaw gateway from anywhere - whether you're on a different network, traveling, or accessing from mobile devices.

## Overview

RustyClaw supports multiple methods for remote access:
1. **Direct Access** - Port forwarding (simple, requires router config)
2. **Tunnel Services** - ngrok, Cloudflare Tunnel (easiest, no config)
3. **VPN** - Tailscale, WireGuard (most secure)
4. **Cloud Hosting** - AWS, GCP, DigitalOcean (always available)

---

## Method 1: Direct Access (Port Forwarding)

### Prerequisites
- Static IP or Dynamic DNS service
- Router access for port forwarding
- TLS certificates (Let's Encrypt recommended)

### Step 1: Enable TLS

Edit `~/.rustyclaw/config.toml`:

```toml
[tls]
enabled = true
self_signed = false
cert_path = "/path/to/fullchain.pem"
key_path = "/path/to/privkey.pem"
```

### Step 2: Get TLS Certificates

#### Option A: Let's Encrypt (Recommended)

```bash
# Install certbot
sudo apt-get install certbot

# Get certificate (replace your-domain.com)
sudo certbot certonly --standalone \
  -d your-domain.com \
  --agree-tos \
  --email your-email@example.com

# Certificates will be in /etc/letsencrypt/live/your-domain.com/
```

Update config:
```toml
[tls]
enabled = true
self_signed = false
cert_path = "/etc/letsencrypt/live/your-domain.com/fullchain.pem"
key_path = "/etc/letsencrypt/live/your-domain.com/privkey.pem"
```

#### Option B: Self-Signed (Development Only)

RustyClaw can generate self-signed certificates automatically:

```toml
[tls]
enabled = true
self_signed = true
# cert_path and key_path not needed - auto-generated
```

⚠️ **Warning**: Self-signed certificates require manual trust on each client device.

### Step 3: Configure Gateway Listen Address

```toml
# Listen on all interfaces (0.0.0.0) instead of localhost
# Default in config - just verify:
# gateway --listen 0.0.0.0:8080
```

Or start with:
```bash
rustyclaw gateway --listen 0.0.0.0:8080
```

### Step 4: Configure Router Port Forwarding

1. Log into your router (usually 192.168.1.1 or 192.168.0.1)
2. Find "Port Forwarding" or "Virtual Server" settings
3. Add rule:
   - **External Port**: 8080
   - **Internal IP**: Your computer's local IP (e.g., 192.168.1.100)
   - **Internal Port**: 8080
   - **Protocol**: TCP
4. Save and restart router if needed

### Step 5: Find Your Public IP

```bash
curl ifconfig.me
# Example output: 203.0.113.45
```

### Step 6: Test Remote Access

From another network:
```bash
# Test connection
curl https://203.0.113.45:8080

# Connect web-ui
open https://203.0.113.45:8080
```

### Step 7: Optional - Dynamic DNS

If your IP changes, use a Dynamic DNS service:

**Popular Services**:
- No-IP (https://www.noip.com/)
- DuckDNS (https://www.duckdns.org/) - Free
- Dynu (https://www.dynu.com/)

Setup example with DuckDNS:
```bash
# Install ddclient
sudo apt-get install ddclient

# Configure /etc/ddclient.conf
protocol=duckdns
server=www.duckdns.org
login=your-domain
password=your-duckdns-token
your-domain.duckdns.org
```

Now access via: `wss://your-domain.duckdns.org:8080`

---

## Method 2: Tunnel Services (Easiest)

No router configuration needed! Perfect for:
- Restricted networks (corporate, dorms)
- Dynamic IPs
- No router access
- Quick testing

### Option A: ngrok (Recommended for Testing)

#### Step 1: Install ngrok

```bash
# Download from https://ngrok.com/download
# Or use package manager:
brew install ngrok  # macOS
snap install ngrok   # Linux
choco install ngrok  # Windows
```

#### Step 2: Sign Up (Free Tier Available)

```bash
# Get authtoken from https://dashboard.ngrok.com/get-started/your-authtoken
ngrok config add-authtoken YOUR_TOKEN
```

#### Step 3: Start RustyClaw Gateway

```bash
rustyclaw gateway --listen 127.0.0.1:8080
```

#### Step 4: Start ngrok Tunnel

```bash
# HTTP tunnel (websocket will upgrade)
ngrok http 8080

# For WSS (secure websocket)
ngrok http --scheme=https 8080
```

Output:
```
Session Status                online
Forwarding                    https://abc123.ngrok.io -> http://localhost:8080
```

#### Step 5: Access Remotely

```bash
# Your public URL
https://abc123.ngrok.io

# Connect web-ui
open https://abc123.ngrok.io
```

**Limitations**:
- Free tier: Random URL changes on restart
- Free tier: 40 connections/minute
- Paid tier ($8/mo): Custom subdomain, more connections

### Option B: Cloudflare Tunnel (Free, Permanent URLs)

#### Step 1: Install cloudflared

```bash
# Download from https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/install-and-setup/installation/

# macOS
brew install cloudflare/cloudflare/cloudflared

# Linux
wget https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb
sudo dpkg -i cloudflared-linux-amd64.deb
```

#### Step 2: Authenticate

```bash
cloudflared tunnel login
# Opens browser to authenticate with Cloudflare
```

#### Step 3: Create Tunnel

```bash
cloudflared tunnel create rustyclaw
# Output: Created tunnel rustyclaw with id abc123...
```

#### Step 4: Configure Tunnel

Create `~/.cloudflared/config.yml`:

```yaml
tunnel: abc123-your-tunnel-id
credentials-file: /home/user/.cloudflared/abc123.json

ingress:
  - hostname: rustyclaw.yourdomain.com
    service: http://localhost:8080
  - service: http_status:404
```

#### Step 5: Add DNS Record

```bash
cloudflared tunnel route dns rustyclaw rustyclaw.yourdomain.com
```

#### Step 6: Start Services

```bash
# Terminal 1: Start RustyClaw
rustyclaw gateway --listen 127.0.0.1:8080

# Terminal 2: Start tunnel
cloudflared tunnel run rustyclaw
```

#### Step 7: Access

```
https://rustyclaw.yourdomain.com
```

**Advantages**:
- Free forever
- Permanent URL
- Automatic HTTPS
- DDoS protection

### Option C: Tailscale Funnel (Free, Secure)

See [TAILSCALE.md](./TAILSCALE.md) for full integration guide.

```bash
# Install Tailscale
curl -fsSL https://tailscale.com/install.sh | sh

# Enable funnel
tailscale funnel 8080

# Access via your-device.tail-scale.ts.net
```

---

## Method 3: VPN Access (Most Secure)

Use VPN to access your home network remotely, then connect to gateway as if local.

### Option A: Tailscale (Easiest VPN)

```bash
# Install on server
curl -fsSL https://tailscale.com/install.sh | sh
tailscale up

# Install on client devices (phone, laptop)
# Download from https://tailscale.com/download

# Access gateway via Tailscale IP
# From any device: wss://100.x.y.z:8080
```

See full guide: [TAILSCALE.md](./TAILSCALE.md)

### Option B: WireGuard (Advanced)

```bash
# Install WireGuard
sudo apt-get install wireguard

# Generate keys
wg genkey | tee privatekey | wg pubkey > publickey

# Configure /etc/wireguard/wg0.conf
[Interface]
Address = 10.0.0.1/24
ListenPort = 51820
PrivateKey = <server-private-key>

[Peer]
PublicKey = <client-public-key>
AllowedIPs = 10.0.0.2/32

# Start VPN
sudo wg-quick up wg0

# Access gateway via VPN IP
wss://10.0.0.1:8080
```

---

## Method 4: Cloud Hosting (Always Available)

Deploy RustyClaw on a cloud server for 24/7 access.

### Quick Deploy Scripts

#### DigitalOcean Droplet

```bash
# Create droplet (Ubuntu 22.04)
# SSH into droplet

# Install RustyClaw
curl https://rustyclaw.dev/install.sh | sh

# Configure for remote access
cd ~/.rustyclaw
nano config.toml

# Start as service
sudo systemctl enable rustyclaw-gateway
sudo systemctl start rustyclaw-gateway

# Access via droplet IP
wss://your-droplet-ip:8080
```

#### AWS EC2

```bash
# Launch EC2 instance (t2.micro for free tier)
# Open port 8080 in Security Group

# Install RustyClaw
curl https://rustyclaw.dev/install.sh | sh

# Configure
rustyclaw configure

# Run in background
nohup rustyclaw gateway &
```

---

## Security Best Practices

### 1. Enable 2FA (TOTP)

```bash
rustyclaw secrets setup-totp
# Scan QR code with authenticator app
```

Update config:
```toml
totp_enabled = true
```

### 2. Use Strong Vault Password

```bash
# When gateway starts, you'll be prompted for vault password
# Use a strong, unique password
```

### 3. Enable DM Pairing (For Messengers)

```toml
[pairing]
enabled = true
require_code = true
```

### 4. IP Allowlist (Optional)

Restrict access to specific IPs:

```toml
[security]
allowed_ips = ["203.0.113.45", "198.51.100.0/24"]
```

### 5. Rate Limiting

```toml
[security]
max_requests_per_minute = 60
```

### 6. Enable Audit Logging

```toml
[hooks]
enabled = true
audit_log_hook = true
audit_log_path = "~/.rustyclaw/logs/audit.log"
```

### 7. Monitor Access

```bash
# Watch logs
tail -f ~/.rustyclaw/logs/gateway.log

# Check metrics (if enabled)
curl http://localhost:9090/metrics
```

---

## Troubleshooting

### Connection Refused

**Problem**: Can't connect from remote network

**Solutions**:
1. Check firewall rules:
   ```bash
   sudo ufw allow 8080/tcp
   ```
2. Verify gateway is listening on 0.0.0.0:
   ```bash
   netstat -tlnp | grep 8080
   ```
3. Check router port forwarding is correct
4. Test from local network first

### TLS Certificate Errors

**Problem**: "Certificate not trusted" errors

**Solutions**:
1. Use Let's Encrypt instead of self-signed
2. If using self-signed, manually trust the certificate:
   - iOS: Settings → Profile → Trust
   - Android: Settings → Security → Install certificate
   - Desktop: Trust in browser/system keychain

### Slow Connection

**Problem**: High latency, slow responses

**Solutions**:
1. Use tunnel service with nearest PoP (ngrok/Cloudflare)
2. Check upload speed: `speedtest-cli`
3. Enable compression in config:
   ```toml
   [gateway]
   enable_compression = true
   ```
4. Reduce poll intervals for messengers:
   ```toml
   [[messengers]]
   poll_interval = 120  # Increase from 60s
   ```

### Timeout Issues

**Problem**: Connections timeout or drop

**Solutions**:
1. Increase timeout in config:
   ```toml
   [gateway]
   websocket_ping_interval = 30
   websocket_timeout = 300
   ```
2. Use keep-alive in tunnel service
3. Check if ISP blocks long-lived connections

---

## Monitoring & Health Checks

### Health Check Endpoint

Enable health checks in `~/.rustyclaw/config.toml`:

```toml
[health]
enabled = true
listen = "127.0.0.1:8080"  # Change to 0.0.0.0:8080 for remote access
```

**Endpoints:**

- `/health` - Simple status check (for load balancers, uptime monitors)
- `/status` - Detailed metrics (connections, messages, uptime)

```bash
# Check if gateway is running
curl http://localhost:8080/health
# Response: {"status":"ok","version":"0.1.33","uptime_secs":3600}

# Get detailed status with metrics
curl http://localhost:8080/status
# Response: {
#   "status":"ok",
#   "version":"0.1.33",
#   "uptime_secs":3600,
#   "metrics": {
#     "total_connections":42,
#     "active_connections":3,
#     "total_messages":1337
#   },
#   "timestamp":1698765432
# }
```

### Prometheus Metrics

Enable metrics for monitoring:

```toml
[metrics]
enabled = true
listen = "127.0.0.1:9090"
```

Monitor with Prometheus/Grafana or check manually:

```bash
curl http://localhost:9090/metrics
```

### Auto-Restart on Failure

#### systemd Service (Linux)

Create `/etc/systemd/system/rustyclaw-gateway.service`:

```ini
[Unit]
Description=RustyClaw Gateway
After=network.target

[Service]
Type=simple
User=your-username
WorkingDirectory=/home/your-username
ExecStart=/home/your-username/.cargo/bin/rustyclaw gateway
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

Enable:
```bash
sudo systemctl daemon-reload
sudo systemctl enable rustyclaw-gateway
sudo systemctl start rustyclaw-gateway
```

---

## Performance Tuning

### For High Traffic

```toml
[gateway]
max_concurrent_connections = 1000
worker_threads = 8

[sandbox]
mode = "none"  # Disable sandboxing for max speed (less secure)
```

### For Low Resources (Raspberry Pi)

```toml
[gateway]
max_concurrent_connections = 10
worker_threads = 2

[model]
# Use smaller, faster models
model = "claude-haiku-3-5"
```

---

## Mobile Access

### iOS

1. Access gateway via tunnel URL: `https://your-tunnel.ngrok.io`
2. Add to Home Screen for app-like experience
3. PWA will work offline after first load

### Android

1. Open Chrome
2. Visit gateway URL
3. Menu → "Install App"
4. Launches in fullscreen mode

---

## Example Configurations

### Home Network Access

```toml
# ~/.rustyclaw/config.toml
[gateway]
listen = "0.0.0.0:8080"

[tls]
enabled = false  # Local network, no TLS needed

totp_enabled = false  # Trust home network
```

### Public Internet Access

```toml
[gateway]
listen = "0.0.0.0:8080"

[tls]
enabled = true
self_signed = false
cert_path = "/etc/letsencrypt/live/yourdomain.com/fullchain.pem"
key_path = "/etc/letsencrypt/live/yourdomain.com/privkey.pem"

totp_enabled = true

[pairing]
enabled = true
require_code = true

[security]
allowed_ips = []  # Allow all, but require 2FA
```

### Development/Testing

```toml
[gateway]
listen = "127.0.0.1:8080"

[tls]
enabled = false

totp_enabled = false

# Use ngrok tunnel instead of exposing directly
```

---

## FAQ

### Q: Is remote access secure?

**A**: Yes, if configured properly:
- ✅ Use TLS/WSS (not plain HTTP)
- ✅ Enable TOTP 2FA
- ✅ Strong vault password
- ✅ DM pairing for messengers
- ✅ Monitor access logs

### Q: Can I use a free domain?

**A**: Yes! Options:
- DuckDNS (free subdomain)
- No-IP (free subdomain)
- FreeDNS (free subdomain)
- Cloudflare (free, but requires domain purchase ~$10/year)

### Q: Do I need a static IP?

**A**: No! Use:
- Dynamic DNS service (updates automatically)
- Tunnel service (ngrok, Cloudflare)
- VPN (Tailscale, WireGuard)

### Q: Can I access from multiple devices simultaneously?

**A**: Yes! The gateway supports multiple concurrent WebSocket connections. Default limit is 100, configurable in `config.toml`.

### Q: How much bandwidth does it use?

**A**: Minimal when idle (~1KB/s for heartbeats). Active usage:
- Text chat: ~10-50KB per message
- Image vision: ~500KB-2MB per image
- Typical usage: <100MB/day

---

## References

- [TLS Configuration](./TLS.md)
- [Tailscale Integration](./TAILSCALE.md)
- [Security Best Practices](./SECURITY.md)
- [Monitoring Guide](./METRICS.md)
- [ngrok Documentation](https://ngrok.com/docs)
- [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/)
- [Let's Encrypt](https://letsencrypt.org/getting-started/)
