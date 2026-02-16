# WebAuthn/Passkey Authentication

RustyClaw supports modern passwordless authentication using WebAuthn passkeys alongside traditional TOTP 2FA.

## Overview

WebAuthn (Web Authentication) enables passwordless authentication using:
- **Security keys** (YubiKey, Titan Key, etc.)
- **Platform authenticators** (TouchID, Face ID, Windows Hello)
- **Cross-device authentication** (QR code flow)

This provides:
- ✅ **Phishing-resistant** authentication (credentials bound to domain)
- ✅ **No shared secrets** (public key cryptography)
- ✅ **User-friendly** experience (biometric unlock)
- ✅ **Multi-device** support (multiple passkeys per user)

## Requirements

WebAuthn **requires TLS/WSS** for security:
```bash
# WebAuthn will NOT work with plain ws://
rustyclaw gateway start --tls-self-signed  # Development
rustyclaw gateway start --tls-cert cert.pem --tls-key key.pem  # Production
```

## Configuration

### Enable WebAuthn

Add to `~/.rustyclaw/config.toml`:

```toml
[webauthn]
enabled = true
rp_id = "localhost"  # Must match the domain
rp_origin = "https://localhost:8443"  # Full URL with protocol
```

### Configuration Fields

| Field | Description | Example |
|-------|-------------|---------|
| `enabled` | Enable WebAuthn authentication | `true` / `false` |
| `rp_id` | Relying Party ID (domain) | `"localhost"`, `"example.com"` |
| `rp_origin` | Relying Party origin (full URL) | `"https://localhost:8443"` |

### Important Notes

1. **rp_id must match the domain**:
   - For localhost: `rp_id = "localhost"`
   - For production: `rp_id = "example.com"` (no subdomain)

2. **rp_origin must use https://**:
   - Development: `"https://localhost:8443"`
   - Production: `"https://example.com"`

3. **Credentials are domain-bound**:
   - A passkey registered on `localhost` won't work on `example.com`
   - Moving to production requires re-registering passkeys

## Registration Flow

### 1. Client Requests Registration

```json
{
  "type": "webauthn_register_start",
  "user_id": "user@example.com",
  "user_name": "User Name"
}
```

### 2. Server Returns Challenge

```json
{
  "type": "webauthn_challenge",
  "challenge": {
    "publicKey": {
      "challenge": "base64url_encoded_challenge",
      "rp": {
        "name": "RustyClaw",
        "id": "localhost"
      },
      "user": {
        "id": "base64url_user_id",
        "name": "user@example.com",
        "displayName": "User Name"
      },
      "pubKeyCredParams": [...],
      "timeout": 60000,
      "excludeCredentials": [...],
      "authenticatorSelection": {
        "authenticatorAttachment": "platform",
        "requireResidentKey": true,
        "userVerification": "required"
      }
    }
  }
}
```

### 3. Client Performs WebAuthn Ceremony

Browser/client calls:
```javascript
const credential = await navigator.credentials.create({
  publicKey: challenge.publicKey
});
```

User interaction:
- **Platform authenticator**: TouchID, Face ID, Windows Hello
- **Security key**: Insert and touch YubiKey
- **Cross-device**: Scan QR code with phone

### 4. Client Sends Response

```json
{
  "type": "webauthn_register_finish",
  "user_id": "user@example.com",
  "credential": {
    "id": "credential_id",
    "rawId": "base64url_raw_id",
    "response": {
      "clientDataJSON": "base64url_data",
      "attestationObject": "base64url_attestation"
    },
    "type": "public-key"
  }
}
```

### 5. Server Verifies and Stores

Gateway:
1. Verifies attestation object
2. Validates challenge match
3. Checks origin and RP ID
4. Stores credential in vault

Response:
```json
{
  "type": "webauthn_register_success",
  "credential_id": "abc123..."
}
```

## Authentication Flow

### 1. Client Requests Authentication

```json
{
  "type": "webauthn_auth_start",
  "user_id": "user@example.com"
}
```

### 2. Server Returns Challenge

```json
{
  "type": "webauthn_challenge",
  "challenge": {
    "publicKey": {
      "challenge": "base64url_encoded_challenge",
      "timeout": 60000,
      "rpId": "localhost",
      "allowCredentials": [
        {
          "id": "base64url_cred_id",
          "type": "public-key",
          "transports": ["usb", "nfc", "ble", "internal"]
        }
      ],
      "userVerification": "required"
    }
  }
}
```

### 3. Client Performs WebAuthn Ceremony

```javascript
const assertion = await navigator.credentials.get({
  publicKey: challenge.publicKey
});
```

### 4. Client Sends Response

```json
{
  "type": "webauthn_auth_finish",
  "user_id": "user@example.com",
  "assertion": {
    "id": "credential_id",
    "rawId": "base64url_raw_id",
    "response": {
      "clientDataJSON": "base64url_data",
      "authenticatorData": "base64url_auth_data",
      "signature": "base64url_signature",
      "userHandle": "base64url_user_handle"
    },
    "type": "public-key"
  }
}
```

### 5. Server Verifies Authentication

Gateway:
1. Verifies signature using stored public key
2. Validates challenge match
3. Checks origin and RP ID
4. Updates credential last_used timestamp

Response:
```json
{
  "type": "auth_success"
}
```

## Credential Management

### Storing Credentials

Passkey credentials are stored in the RustyClaw secrets vault:

```rust
pub struct StoredPasskey {
    pub credential: Passkey,
    pub user_name: String,
    pub created_at: i64,  // Unix timestamp
    pub last_used: Option<i64>,
}
```

Vault key format:
- `WEBAUTHN_PASSKEY_{credential_id}` — Individual passkey
- `WEBAUTHN_USER_{user_id}` — User's passkey list

### Listing Passkeys

```bash
# Via CLI (future feature)
rustyclaw secrets list | grep WEBAUTHN

# Via gateway control message
{
  "type": "secrets_list",
  "filter": "WEBAUTHN_*"
}
```

### Revoking Passkeys

```bash
# Via CLI
rustyclaw secrets delete WEBAUTHN_PASSKEY_abc123

# Via gateway
{
  "type": "secrets_delete",
  "key": "WEBAUTHN_PASSKEY_abc123"
}
```

## TOTP Fallback

WebAuthn does not replace TOTP — both can coexist:

```toml
[auth]
totp_enabled = true  # Traditional TOTP 2FA

[webauthn]
enabled = true  # Modern passkeys
```

### Authentication Priority

When both are enabled:
1. Gateway offers both methods in `auth_challenge`
2. Client chooses preferred method
3. Both methods provide equal security

### Use Cases

- **TOTP**: Fallback when passkey unavailable, CLI access
- **WebAuthn**: Primary for browser/GUI, better UX

## Security Considerations

### Phishing Resistance

WebAuthn credentials are **bound to the domain**:
- ✅ Passkey registered on `example.com` won't work on `evil.com`
- ✅ Attacker cannot phish credentials (no shared secrets)
- ✅ MITM attacks are ineffective (signature includes origin)

### Device Loss

If user loses their device:
- ✅ **Multiple passkeys supported** — register backup authenticator
- ✅ **TOTP fallback** — use traditional 2FA
- ✅ **Revocation** — delete lost device's passkey from vault

### Cross-Device Flow

For users without platform authenticator:
1. Server generates QR code with challenge
2. User scans with phone (platform authenticator)
3. Phone completes WebAuthn ceremony
4. Result transmitted back to browser

(Implementation: future enhancement)

## Platform Support

### Authenticators

| Authenticator | Status | Notes |
|---------------|--------|-------|
| **YubiKey** | ✅ Supported | USB/NFC security keys |
| **Titan Key** | ✅ Supported | Google security keys |
| **TouchID** | ✅ Supported | macOS platform authenticator |
| **Face ID** | ✅ Supported | iOS platform authenticator |
| **Windows Hello** | ✅ Supported | Windows platform authenticator |
| **Android Fingerprint** | ✅ Supported | Android platform authenticator |

### Browsers

| Browser | Status | Notes |
|---------|--------|-------|
| **Chrome** | ✅ Full support | WebAuthn Level 2 |
| **Firefox** | ✅ Full support | WebAuthn Level 2 |
| **Safari** | ✅ Full support | WebAuthn Level 2 |
| **Edge** | ✅ Full support | WebAuthn Level 2 |

### Operating Systems

| OS | Platform Authenticator | External Keys |
|----|------------------------|---------------|
| **macOS** | TouchID | ✅ USB/NFC |
| **Windows** | Windows Hello | ✅ USB/NFC |
| **Linux** | ❌ No platform auth | ✅ USB/NFC |
| **iOS** | Face ID / TouchID | ✅ NFC (iPhone XS+) |
| **Android** | Fingerprint / Face | ✅ NFC |

## Troubleshooting

### Registration Fails

**Problem**: Passkey registration returns error

**Solutions**:
1. Verify TLS is enabled: `rustyclaw gateway start --tls-self-signed`
2. Check `rp_origin` matches gateway URL
3. Ensure browser supports WebAuthn
4. Try different authenticator

### Authentication Fails

**Problem**: Passkey authentication fails

**Solutions**:
1. Verify credential registered for this domain
2. Check `rp_id` matches registration
3. Ensure authenticator is available
4. Try TOTP fallback

### Wrong Domain

**Problem**: "Credential not found for this domain"

**Cause**: Moving from localhost to production or vice versa

**Solution**: Re-register passkeys on new domain (credentials are domain-bound)

### Browser Doesn't Prompt

**Problem**: WebAuthn ceremony doesn't start

**Solutions**:
1. Check browser console for errors
2. Verify HTTPS (WebAuthn requires secure context)
3. Ensure `PublicKeyCredential` API available
4. Try different browser

## Best Practices

### For Users

1. **Register multiple passkeys**:
   - Primary: Platform authenticator (TouchID, Windows Hello)
   - Backup: USB security key (YubiKey)
   - Fallback: Keep TOTP enabled

2. **Store backup codes**:
   - Keep TOTP recovery codes
   - Store in password manager or secure location

3. **Test backup methods**:
   - Verify TOTP works before disabling
   - Test security key before relying solely on platform auth

### For Administrators

1. **Require TLS in production**:
   ```toml
   [tls]
   enabled = true
   cert_path = "/path/to/cert.pem"
   key_path = "/path/to/key.pem"
   ```

2. **Set correct RP ID**:
   - Use root domain (not subdomain)
   - Example: `example.com`, not `auth.example.com`

3. **Enable both TOTP and WebAuthn**:
   - Provides flexibility for users
   - TOTP works for CLI access

4. **Monitor credentials**:
   - Track last_used timestamps
   - Revoke old/unused passkeys

## Related

- [TLS/WSS Configuration](./HOT_RELOAD.md#tlswss-configuration)
- [Secrets Vault](../README.md#secrets-vault)
- [Configuration](../README.md#configuration)

## References

- [WebAuthn Specification](https://www.w3.org/TR/webauthn-2/)
- [FIDO Alliance](https://fidoalliance.org/)
- [webauthn-rs Documentation](https://docs.rs/webauthn-rs/)
