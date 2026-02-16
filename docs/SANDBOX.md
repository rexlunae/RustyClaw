# Sandbox Security Guide

RustyClaw implements multiple layers of sandbox isolation to protect your system from potentially harmful commands executed by the AI agent. This guide covers sandbox modes, configuration, and security best practices.

## Overview

**Why Sandbox?**
AI agents execute shell commands on your behalf. While powerful, this poses security risks:
- Accidental deletion of important files
- Exposure of sensitive credentials
- Execution of malicious code
- System-wide damage from bugs

**RustyClaw's Defense:**
- 🛡️ Multiple sandbox modes (Landlock, Bubblewrap, macOS Sandbox)
- 🚫 Path-based access control
- 🔒 Credential directory protection (automatic)
- ⚠️ Pre-execution path validation
- 🎯 Configurable deny lists

---

## Sandbox Modes

RustyClaw supports 4 sandbox modes, auto-selected based on platform:

### 1. Landlock (Linux - Recommended)

**What it is:** Kernel-level security module (Linux 5.13+) that restricts filesystem access.

**How it works:**
- Kernel-enforced restrictions (can't be bypassed)
- Applied per-process (irreversible once set)
- Denies read/execute access to specified paths
- Automatic fallback if not supported

**Strengths:**
- ✅ Strongest security (kernel-level)
- ✅ Fine-grained path control
- ✅ No external dependencies
- ✅ Lightweight (no overhead)

**Limitations:**
- ❌ Linux only (kernel 5.13+)
- ❌ Requires modern kernel
- ⚠️ Cannot be undone (per-process)

**Example denial:**
```rust
// Agent cannot:
cat ~/.rustyclaw/credentials/secrets.json  // ❌ Blocked by Landlock
cd ~/.ssh && cat id_rsa                     // ❌ Blocked by Landlock
```

---

### 2. Bubblewrap (Linux)

**What it is:** User namespace sandbox (bwrap) that creates isolated filesystem views.

**How it works:**
- Creates new mount namespace
- Exposes only approved directories
- Blocks access to everything else
- Each command runs in isolated bubble

**Strengths:**
- ✅ Very strong isolation
- ✅ Widely available on Linux
- ✅ Per-command sandboxing
- ✅ Flexible configuration

**Limitations:**
- ❌ Linux only
- ❌ Requires bwrap binary installed
- ⚠️ Some overhead per command

**Installation:**
```bash
# Ubuntu/Debian
sudo apt-get install bubblewrap

# Fedora/RHEL
sudo dnf install bubblewrap

# Arch
sudo pacman -S bubblewrap
```

**Example denial:**
```bash
# Inside bwrap bubble, only workspace + /tmp visible
ls /home/user/.ssh/        # ❌ Directory doesn't exist
cat /etc/passwd            # ❌ File not accessible
```

---

### 3. macOS Sandbox (macOS)

**What it is:** Apple's sandbox-exec with Seatbelt profiles.

**How it works:**
- Uses macOS sandbox-exec command
- Generates Seatbelt policy profiles
- Restricts file operations system-wide

**Strengths:**
- ✅ Built into macOS (no install needed)
- ✅ Apple-supported
- ✅ Integrates with macOS security

**Limitations:**
- ❌ macOS only
- ⚠️ Complex Seatbelt syntax
- ⚠️ Less flexible than Linux options

**Example denial:**
```bash
# Seatbelt profile blocks sensitive paths
sandbox-exec -p "(deny file-read* ...)" command
```

---

### 4. Path Validation (Fallback)

**What it is:** Pre-execution path checking (no kernel enforcement).

**How it works:**
- Parses command for file paths
- Checks against deny list
- Rejects command if blocked path detected

**Strengths:**
- ✅ Works everywhere (portable)
- ✅ No dependencies
- ✅ Fast (no overhead)

**Limitations:**
- ❌ **NOT kernel-enforced** (can be bypassed)
- ❌ Only catches obvious path references
- ⚠️ Parser can be fooled

**Use case:** Last resort when no other sandbox available.

---

## Configuration

### Enable Sandbox

Edit `~/.rustyclaw/config.toml`:

```toml
[sandbox]
# Sandbox mode: "auto", "landlock", "bwrap", "macos", "path", "none"
mode = "auto"  # Recommended: auto-detect best available

# Additional paths to deny (beyond credentials dir)
deny_paths = [
    "/home/user/.ssh",
    "/home/user/.gnupg",
    "/etc/ssl/private",
    "/root",
]

# Paths to explicitly allow in strict mode (optional)
allow_paths = [
    "/home/user/projects",
    "/tmp",
]
```

### Mode Selection

**Auto (Recommended):**
```toml
[sandbox]
mode = "auto"  # Picks: landlock > bwrap > macos > path
```

**Force Specific Mode:**
```toml
[sandbox]
mode = "landlock"  # Force Landlock (fails if unavailable)
```

**Disable (NOT RECOMMENDED):**
```toml
[sandbox]
mode = "none"  # ⚠️ NO PROTECTION - use only for debugging
```

### Protected Paths (Automatic)

RustyClaw automatically protects:
- `~/.rustyclaw/credentials/` - Secrets vault
- Paths in `sandbox.deny_paths` config
- SSH keys (if detected)
- GPG keys (if detected)

---

## Security Levels

### Level 1: Maximum Security (Production)

```toml
[sandbox]
mode = "auto"
deny_paths = [
    "/home/user/.ssh",
    "/home/user/.gnupg",
    "/home/user/.aws",
    "/home/user/.kube",
    "/etc/ssl",
    "/etc/ssh",
    "/root",
]
```

**Result:** Agent cannot access credentials, even if compromised.

---

### Level 2: Development Mode

```toml
[sandbox]
mode = "auto"
deny_paths = [
    "/home/user/.ssh",      # Block SSH keys
    # Allow AWS/Kube for development
]
```

**Result:** Balance between security and functionality.

---

### Level 3: Debugging Only

```toml
[sandbox]
mode = "path"  # Weakest protection
deny_paths = []  # Nothing denied (except credentials dir)
```

**Result:** Minimal protection. Use only when troubleshooting.

---

## How It Works

### Command Execution Flow

```
User Request: "Delete old logs in /var/log"
       ↓
Gateway receives command
       ↓
┌─────────────────────────────────┐
│   SANDBOX CHECK                 │
│  1. Mode detection              │
│  2. Path validation             │
│  3. Credential check            │
└─────────────────────────────────┘
       ↓
[LANDLOCK MODE]
       ↓
Apply Landlock restrictions:
  - Deny: ~/.rustyclaw/credentials/
  - Deny: ~/.ssh/
  - Deny: /root/
       ↓
Execute: rm /var/log/old.log
       ↓
✅ Success (allowed path)

═══════════════════════════════════

[BLOCKED EXAMPLE]
       ↓
User: "Show me API keys"
       ↓
Command: cat ~/.rustyclaw/credentials/secrets.json
       ↓
Landlock blocks read access
       ↓
❌ Error: Permission denied
       ↓
Agent receives error, cannot access secrets
```

### Bubblewrap Isolation

```bash
# Agent executes: cat ~/project/file.txt

# RustyClaw wraps with bwrap:
bwrap \
  --ro-bind /usr /usr \              # Read-only system files
  --ro-bind /lib /lib \              # Read-only libraries
  --bind ~/workspace ~/workspace \   # Writable workspace
  --bind /tmp /tmp \                 # Writable temp
  --dev /dev \                       # Devices
  --proc /proc \                     # Process info
  --unshare-all \                    # Isolate namespaces
  --die-with-parent \                # Clean exit
  -- \
  bash -c "cat ~/project/file.txt"

# Inside bubble:
# - Only sees: /usr, /lib, ~/workspace, /tmp, /dev, /proc
# - Cannot see: ~/.ssh, ~/.gnupg, ~/.rustyclaw/credentials
```

---

## Testing Your Sandbox

### Check Active Mode

```bash
rustyclaw doctor sandbox

# Output:
# Sandbox Status:
#   Mode: Landlock (kernel 5.15.0)
#   Protected paths: 3
#   - /home/user/.rustyclaw/credentials
#   - /home/user/.ssh
#   - /home/user/.gnupg
```

### Test Protection

**Method 1: Safe test**
```bash
# Create dummy secret
mkdir -p ~/.rustyclaw/credentials
echo "SECRET=test123" > ~/.rustyclaw/credentials/test.txt

# Try to read (should fail)
rustyclaw command "cat ~/.rustyclaw/credentials/test.txt"

# Expected:
# Error: Permission denied (blocked by Landlock/bwrap)
```

**Method 2: Verify deny paths**
```bash
# Add test deny path
echo '[sandbox]
deny_paths = ["/tmp/blocked"]' >> ~/.rustyclaw/config.toml

# Create test file
mkdir /tmp/blocked
echo "sensitive" > /tmp/blocked/data.txt

# Try to access
rustyclaw command "cat /tmp/blocked/data.txt"

# Expected: Blocked by sandbox
```

### Sandbox Capabilities Detection

```bash
# Check what's available on your system
rustyclaw doctor capabilities

# Example output:
# Sandbox Capabilities:
#   ✓ Landlock (kernel 5.15.0)
#   ✓ Bubblewrap (bwrap 0.5.0)
#   ✗ macOS Sandbox (not on macOS)
#   Best mode: Landlock
```

---

## Elevated Mode

Sometimes you need to run privileged commands (sudo). RustyClaw supports per-session elevated mode:

### Enable Elevated Mode

**In TUI:**
```
/elevated on
```

**In config:**
```toml
[sandbox]
allow_elevated = true  # Allow sudo commands
```

### Security Implications

⚠️ **Warning:** Elevated mode bypasses sandbox for sudo commands!

```bash
# Elevated mode OFF (default):
sudo rm -rf /  # ❌ Blocked by sandbox

# Elevated mode ON:
sudo rm -rf /  # ⚠️ ALLOWED (use with caution!)
```

**Best practice:**
- Keep elevated mode OFF by default
- Enable only when needed
- Disable immediately after use
- Use `/elevated on` in TUI (per-session)

---

## Common Scenarios

### Scenario 1: Agent Needs to Read SSH Config

**Problem:**
```bash
Agent: "Check SSH config"
Command: cat ~/.ssh/config
Result: ❌ Permission denied
```

**Solution A (Recommended):** Use tools that don't need direct access
```bash
# Instead of reading file:
Agent: "Show SSH hosts"
Command: ssh -G host | grep hostname
Result: ✅ Works (doesn't need file access)
```

**Solution B:** Temporarily allow path
```toml
[sandbox]
allow_paths = ["/home/user/.ssh/config"]  # Read-only access
```

---

### Scenario 2: Development Needs AWS Credentials

**Problem:**
```bash
Agent: "Deploy to AWS"
Command: aws s3 cp ... (needs ~/.aws/credentials)
Result: ❌ Blocked
```

**Solution:** Don't deny AWS paths in development
```toml
[sandbox]
deny_paths = [
    "/home/user/.ssh",
    # "/home/user/.aws",  # Commented out for dev work
]
```

**Better:** Use environment variables instead
```bash
export AWS_ACCESS_KEY_ID=...
export AWS_SECRET_ACCESS_KEY=...
# Sandbox doesn't block env vars
```

---

### Scenario 3: CI/CD Pipeline Needs Access

**Problem:** Sandbox blocks deployment scripts.

**Solution:** Use `mode = "path"` for CI
```toml
# ci-config.toml
[sandbox]
mode = "path"  # Weaker but works in containers
deny_paths = []  # CI already sandboxed by Docker
```

---

## Security Best Practices

### 1. Always Use Auto Mode
```toml
[sandbox]
mode = "auto"  # Let RustyClaw pick the strongest available
```

### 2. Deny Sensitive Directories
```toml
[sandbox]
deny_paths = [
    "~/.ssh",        # SSH keys
    "~/.gnupg",      # GPG keys
    "~/.aws",        # AWS credentials
    "~/.kube",       # Kubernetes config
    "~/.docker",     # Docker credentials
    "/etc/ssl",      # SSL certificates
]
```

### 3. Use Workspace Directory
```bash
# Configure workspace
cd ~/projects/work
rustyclaw chat

# Agent operates in ~/projects/work
# Cannot access parent directories
```

### 4. Enable TOTP 2FA
```toml
totp_enabled = true  # Require 2FA for gateway access
```

**Defense in depth:** Even if sandbox is bypassed, 2FA protects access.

### 5. Review Credentials Regularly
```bash
rustyclaw secrets list

# Remove unused secrets
rustyclaw secrets delete OLD_API_KEY
```

### 6. Monitor Agent Activity
```bash
# Enable audit logging
[hooks]
audit_log_hook = true
audit_log_path = "~/.rustyclaw/logs/audit.log"

# Review commands
tail -f ~/.rustyclaw/logs/audit.log
```

---

## Troubleshooting

### "Landlock not supported"

**Problem:**
```
[sandbox] Landlock not supported (kernel < 5.13)
```

**Solution:**
```bash
# Check kernel version
uname -r

# If < 5.13: Use bwrap instead
[sandbox]
mode = "bwrap"
```

**Or upgrade kernel:**
```bash
# Ubuntu
sudo apt-get update && sudo apt-get dist-upgrade
```

---

### "bwrap: not found"

**Problem:**
```
[sandbox] Bubblewrap not available
```

**Solution:**
```bash
# Install bubblewrap
sudo apt-get install bubblewrap  # Ubuntu/Debian
sudo dnf install bubblewrap      # Fedora/RHEL
sudo pacman -S bubblewrap        # Arch
```

---

### "Permission denied" but path should be allowed

**Problem:**
```bash
Command: cat ~/project/file.txt
Error: Permission denied
```

**Debug:**
```bash
# 1. Check sandbox mode
rustyclaw doctor sandbox

# 2. Check deny_paths
grep -A 10 '\[sandbox\]' ~/.rustyclaw/config.toml

# 3. Check if parent dir is denied
ls -la ~/  # Is ~/project denied?

# 4. Try with weaker mode
[sandbox]
mode = "path"  # Temporarily for debugging
```

---

### Agent can access blocked paths

**Problem:** Sandbox not working?

**Verify:**
```bash
# 1. Check mode
[sandbox]
mode = "auto"  # Should auto-detect

# 2. Verify capabilities
rustyclaw doctor capabilities

# 3. Test manually
bwrap --help  # Should show help
cat /sys/kernel/security/landlock/abi  # Should show number

# 4. Check logs
grep sandbox ~/.rustyclaw/logs/gateway.log
```

**If still not working:**
```bash
# Force strongest mode
[sandbox]
mode = "landlock"  # Will error if not available
```

---

## Advanced Configuration

### Custom Sandbox Policy

```toml
[sandbox]
mode = "bwrap"

# Fine-grained control
deny_paths = [
    "/home/user/.ssh",
    "/home/user/secrets",
]

allow_paths = [
    "/home/user/projects/public",
    "/tmp",
]

# Deny execution of binaries in tmp
deny_exec = ["/tmp"]
```

### Per-Tool Sandbox Override

```rust
// In tool implementation
pub fn execute_command(cmd: &str) -> Result<String> {
    let policy = SandboxPolicy {
        deny_read: vec![
            PathBuf::from("/home/user/.ssh"),
            PathBuf::from("/etc/passwd"),
        ],
        deny_exec: vec![
            PathBuf::from("/tmp"),
        ],
        ..Default::default()
    };

    let mode = SandboxMode::Auto;
    let output = run_sandboxed(cmd, &policy, mode)?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

---

## Comparison: Sandbox Modes

| Feature | Landlock | Bubblewrap | macOS Sandbox | Path Validation |
|---------|----------|------------|---------------|-----------------|
| **Platform** | Linux 5.13+ | Linux | macOS | All |
| **Kernel Enforced** | ✅ Yes | ✅ Yes | ✅ Yes | ❌ No |
| **Bypass Proof** | ✅ Yes | ✅ Yes | ✅ Yes | ❌ No |
| **Installation** | Built-in | Package | Built-in | Built-in |
| **Overhead** | None | Low | Low | None |
| **Flexibility** | High | High | Medium | Low |
| **Security** | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐ |
| **Recommended** | ✅ Yes | ✅ Yes | ✅ Yes | ⚠️ Last Resort |

**Winner (Linux):** Landlock (kernel 5.13+) or Bubblewrap
**Winner (macOS):** macOS Sandbox
**Winner (Windows/Other):** Path Validation (⚠️ weak)

---

## FAQ

### Q: Can I disable sandbox for testing?

**A:** Yes, but NOT recommended:
```toml
[sandbox]
mode = "none"  # ⚠️ NO PROTECTION
```

**Better:** Use weak mode:
```toml
[sandbox]
mode = "path"  # Some protection, easier debugging
```

---

### Q: Does sandbox affect performance?

**A:**
- Landlock: **No overhead** (kernel-level)
- Bubblewrap: **~50ms per command** (namespace creation)
- macOS Sandbox: **~20ms per command**
- Path Validation: **~1ms per command**

For typical usage: **negligible impact**.

---

### Q: Can agent break out of sandbox?

**A:**
- Landlock/bwrap/macOS: **NO** (kernel enforced)
- Path Validation: **YES** (easily bypassed)

Always use kernel-enforced sandbox in production.

---

### Q: What if I need sudo access?

**A:** Use elevated mode:
```bash
/elevated on  # In TUI
# Or in config:
[sandbox]
allow_elevated = true
```

⚠️ **Warning:** This bypasses sandbox for sudo commands!

---

### Q: Does sandbox protect against all threats?

**A:** No, sandbox is **one layer** of defense:

**What it protects:**
- ✅ File access (credentials, SSH keys)
- ✅ Directory traversal
- ✅ Accidental data loss

**What it doesn't protect:**
- ❌ Network attacks (use firewall)
- ❌ Memory corruption (use safe languages)
- ❌ Logic bugs (use code review)

**Defense in depth:** Use sandbox + TOTP 2FA + SSRF protection + prompt guards.

---

## Resources

- [Landlock Documentation](https://landlock.io/)
- [Bubblewrap GitHub](https://github.com/containers/bubblewrap)
- [macOS Sandbox Guide](https://developer.apple.com/documentation/security/app_sandbox)
- [RustyClaw Security Guide](./SECURITY.md)
- [RustyClaw Configuration](./CONFIGURATION.md)

---

## Summary

**RustyClaw Sandbox Provides:**
- ✅ Multiple sandbox modes (auto-selected)
- ✅ Kernel-enforced restrictions (Landlock/bwrap/macOS)
- ✅ Automatic credential protection
- ✅ Configurable deny/allow lists
- ✅ Path validation fallback
- ✅ Testing and verification tools

**Recommended Setup:**
```toml
[sandbox]
mode = "auto"  # Auto-detect best mode
deny_paths = [
    "~/.ssh",
    "~/.gnupg",
    "~/.aws",
]

[totp]
enabled = true  # Add 2FA for defense in depth
```

**Stay safe! 🛡️🦞**
