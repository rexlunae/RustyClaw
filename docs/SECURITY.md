# RustyClaw Security Model

RustyClaw is designed with the assumption that **AI agents cannot always be trusted**. While agents are powerful assistants, they can be manipulated through prompt injection, confused by adversarial inputs, or simply make mistakes. The security model provides defense-in-depth to protect sensitive data.

## Threat Model

### What We Protect Against

| Threat | Protection | Effectiveness |
|--------|------------|---------------|
| Agent reading secrets directly | Encrypted vault + path blocking | ✅ Strong |
| Agent exfiltrating secrets via tools | Sandbox isolation | ✅ Strong |
| Agent accessing secrets without permission | Access policies | ✅ Strong |
| Malicious skill reading secrets | Skill-based access policies | ✅ Strong |
| Brute force vault attacks | Rate limiting + lockout | ✅ Strong |
| Memory disclosure after vault unlock | Secrets not held in agent context | ✅ Strong |

### What Requires Additional Hardening

| Threat | Mitigation | Notes |
|--------|------------|-------|
| Root/admin compromise | None (out of scope) | Use separate user accounts |
| Physical access to machine | Full-disk encryption | OS-level protection |
| Malicious dependencies | Cargo audit + review | Supply chain security |

## Security Layers

### Layer 1: Encrypted Secrets Vault

All secrets are stored in an AES-256 encrypted vault at `~/.rustyclaw/credentials/vault.json`. The encryption key can be:

- **Auto-generated**: Stored in `keyfile.key` (default for single-user setups)
- **Password-derived**: User-provided password via Argon2id KDF

```toml
# Enable password-based encryption
secrets_password_protected = true
```

### Layer 2: TOTP Two-Factor Authentication

Optional TOTP 2FA adds a second factor for vault access:

```bash
rustyclaw secrets totp enable
```

This generates a QR code for your authenticator app. Once enabled, the agent (and user) must provide a valid TOTP code to access secrets.

### Layer 3: Per-Credential Access Policies

Each credential can have its own access policy:

| Policy | Behavior |
|--------|----------|
| `Always` | Agent can read without prompting |
| `WithApproval` | User must approve each access (default) |
| `WithAuth` | Requires vault password + TOTP before each access |
| `SkillOnly(["git", "ssh"])` | Only accessible when running named skills |

```bash
# Set policy when storing
rustyclaw secrets store github_token --policy approval
rustyclaw secrets store prod_db_password --policy auth
rustyclaw secrets store ssh_key --policy skill-only=git,ssh
```

### Layer 4: Agent Access Control

A global kill-switch disables all agent access to secrets:

```toml
# In config.toml
agent_access = false
```

When disabled, the `secrets_list`, `secrets_get`, and `secrets_store` tools return errors. The user can still access secrets via the TUI or CLI.

### Layer 5: Path Protection

The credentials directory (`~/.rustyclaw/credentials/`) is protected at the tool level:

- `read_file` blocks reads from the credentials directory
- `write_file` blocks writes to the credentials directory
- `execute_command` blocks commands containing the credentials path
- `list_directory` skips the credentials directory

Path checking uses canonicalization to prevent symlink attacks.

### Layer 6: Sandbox Isolation

For maximum security, enable sandbox mode:

```toml
[sandbox]
mode = "auto"
```

#### Available Sandbox Modes

| Mode | Platform | How It Works |
|------|----------|--------------|
| `bwrap` | Linux | Bubblewrap user namespace; credentials dir not mounted |
| `landlock` | Linux 5.13+ | Kernel-enforced filesystem restrictions |
| `macos` | macOS | `sandbox-exec` with Seatbelt profile |
| `path` | All | Software path validation only |
| `auto` | All | Picks strongest available |
| `none` | All | No sandboxing |

#### Bubblewrap Isolation Details

When sandbox mode is `bwrap`, commands run in an isolated namespace:

- ✅ `/usr`, `/lib`, `/bin`, `/etc` mounted read-only
- ✅ Workspace directory mounted read-write
- ✅ Fresh `/tmp` (tmpfs)
- ❌ Credentials directory **not mounted** (doesn't exist in sandbox)
- ❌ Home directory (except workspace) not accessible
- ✅ Network allowed (for `web_fetch`)
- ✅ Process dies if parent exits

## Security Recommendations

### For Personal Use

```toml
# Minimal security — convenient for single-user
agent_access = true
secrets_password_protected = false
totp_enabled = false

[sandbox]
mode = "path"
```

### For Shared Machines

```toml
# Moderate security — password protects secrets
agent_access = true
secrets_password_protected = true
totp_enabled = false

[sandbox]
mode = "auto"
```

### For Production / High Security

```toml
# Maximum security — full isolation + 2FA
agent_access = false  # Require explicit user approval
secrets_password_protected = true
totp_enabled = true

[sandbox]
mode = "auto"
deny_paths = ["/etc/shadow", "/root"]
```

## Credential Types

RustyClaw supports various credential types with appropriate handling:

| Type | Storage | Notes |
|------|---------|-------|
| `ApiKey` | Encrypted string | Bearer tokens, API keys |
| `UsernamePassword` | Encrypted object | Login credentials |
| `SshKey` | Encrypted keypair | Ed25519 keys, can be generated |
| `Token` | Encrypted string | OAuth tokens, bot tokens |
| `HttpPasskey` | Encrypted object | WebAuthn credentials |
| `PaymentMethod` | Encrypted object | Card details |
| `SecureNote` | Encrypted text | Recovery codes, PINs, etc. |
| `FormAutofill` | Encrypted object | Address, contact info |

### SSH Key Generation

RustyClaw can generate Ed25519 SSH keys directly:

```bash
rustyclaw secrets generate-ssh github_ssh
```

Both private and public keys are stored encrypted. Export the public key:

```bash
rustyclaw secrets export github_ssh --public
```

## Rate Limiting & Lockout

To prevent brute force attacks:

- **Rate limit**: 1 attempt per second after 3 failures
- **Lockout**: 15-minute lockout after 10 consecutive failures
- **Backoff**: Exponential backoff on repeated failures

## Audit Logging

Access attempts are logged to `~/.rustyclaw/logs/security.log`:

```
2024-01-15T10:30:45Z INFO secrets_access name=github_token policy=WithApproval approved=true
2024-01-15T10:31:02Z WARN secrets_access name=prod_password policy=WithAuth failed=invalid_totp
2024-01-15T10:45:00Z INFO sandbox_violation path=/etc/shadow command="cat /etc/shadow"
```

## Known Limitations

### Shell Expansion Bypass (Mitigated by Sandbox)

Without sandbox isolation, an agent could theoretically bypass path checking via shell expansion:

```bash
# This would bypass string matching (but sandbox blocks it)
cat $(echo ~/.rustyclaw/cred*/vault.json)
```

**Mitigation**: Enable sandbox mode (`bwrap`, `landlock`, or `macos`). The credentials directory is not mounted in the sandbox namespace.

### Memory Inspection (Low Risk)

A sufficiently sophisticated agent could theoretically inspect `/proc/self/mem` or similar. This requires:
- Same UID as the RustyClaw process
- Knowledge of memory layout
- Ability to parse raw memory

**Mitigation**: Secrets are decrypted only when needed and not stored in the agent's context window.

## Reporting Security Issues

Please report security vulnerabilities privately:

1. **Do not** open a public GitHub issue
2. Email: security@rustyclaw.dev
3. Or use GitHub's private vulnerability reporting

We aim to respond within 48 hours and release patches promptly.

## Comparison with OpenClaw

| Feature | RustyClaw | OpenClaw |
|---------|-----------|----------|
| Built-in secrets vault | ✅ Yes | ❌ No (uses 1Password, etc.) |
| Built-in TOTP | ✅ Yes | ❌ No |
| Built-in sandbox | ✅ Yes | ❌ External only |
| Per-credential policies | ✅ Yes | ❌ No |
| Memory safety | ✅ Rust | ⚠️ Node.js |
| Path protection | ✅ Yes | ⚠️ Limited |

---

*Security is a process, not a product. If you find issues, please report them responsibly.*
