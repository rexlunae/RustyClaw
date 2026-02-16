# Gmail Messenger Setup Guide

This guide explains how to configure RustyClaw to receive and respond to emails via Gmail.

## Overview

The Gmail messenger integration allows RustyClaw to:
- Monitor your Gmail inbox for new messages
- Process email content through the AI model
- Send automated replies
- Work with labels, filters, and threads

## Prerequisites

- A Google Account with Gmail enabled
- Access to Google Cloud Console
- RustyClaw 0.1.33 or later

## Step 1: Create OAuth2 Credentials

### 1.1 Create a Google Cloud Project

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Click "Select a project" → "New Project"
3. Enter project name: `RustyClaw Gmail Integration`
4. Click "Create"

### 1.2 Enable Gmail API

1. In the Cloud Console, go to "APIs & Services" → "Library"
2. Search for "Gmail API"
3. Click on it and click "Enable"

### 1.3 Configure OAuth Consent Screen

1. Go to "APIs & Services" → "OAuth consent screen"
2. Select "External" (unless you have a Google Workspace account)
3. Click "Create"
4. Fill in the required fields:
   - App name: `RustyClaw`
   - User support email: Your email
   - Developer contact: Your email
5. Click "Save and Continue"
6. On "Scopes" page, click "Add or Remove Scopes"
7. Add these scopes:
   - `https://www.googleapis.com/auth/gmail.modify`
   - `https://www.googleapis.com/auth/gmail.send`
8. Click "Update" → "Save and Continue"
9. On "Test users", add your Gmail address
10. Click "Save and Continue"

### 1.4 Create OAuth2 Credentials

1. Go to "APIs & Services" → "Credentials"
2. Click "Create Credentials" → "OAuth client ID"
3. Application type: "Desktop app"
4. Name: `RustyClaw Desktop Client`
5. Click "Create"
6. **Important**: Copy the Client ID and Client Secret
   - Store them securely - you'll need them later
7. Click "OK"

## Step 2: Obtain Refresh Token

### 2.1 Using OAuth2 Playground (Recommended)

1. Go to [OAuth2 Playground](https://developers.google.com/oauthplayground/)
2. Click the gear icon (⚙️) in the top right
3. Check "Use your own OAuth credentials"
4. Enter your Client ID and Client Secret
5. Click "Close"
6. On the left, find "Gmail API v1"
7. Select these scopes:
   - `https://www.googleapis.com/auth/gmail.modify`
   - `https://www.googleapis.com/auth/gmail.send`
8. Click "Authorize APIs"
9. Sign in with your Google account
10. Click "Allow"
11. Click "Exchange authorization code for tokens"
12. Copy the "Refresh token" value

### 2.2 Alternative: Using curl (Advanced)

```bash
# 1. Generate authorization URL
CLIENT_ID="your-client-id"
REDIRECT_URI="urn:ietf:wg:oauth:2.0:oob"
SCOPE="https://www.googleapis.com/auth/gmail.modify https://www.googleapis.com/auth/gmail.send"

AUTH_URL="https://accounts.google.com/o/oauth2/v2/auth?client_id=${CLIENT_ID}&redirect_uri=${REDIRECT_URI}&scope=${SCOPE}&response_type=code&access_type=offline&prompt=consent"

# 2. Open URL in browser
echo "Visit: $AUTH_URL"

# 3. Copy the authorization code from the browser

# 4. Exchange for refresh token
AUTH_CODE="paste-auth-code-here"
CLIENT_SECRET="your-client-secret"

curl -X POST https://oauth2.googleapis.com/token \
  -d "code=${AUTH_CODE}" \
  -d "client_id=${CLIENT_ID}" \
  -d "client_secret=${CLIENT_SECRET}" \
  -d "redirect_uri=${REDIRECT_URI}" \
  -d "grant_type=authorization_code"

# 5. Extract refresh_token from JSON response
```

## Step 3: Configure RustyClaw

### 3.1 Add Gmail Configuration

Edit `~/.rustyclaw/config.toml` and add:

```toml
[[messengers]]
name = "my-gmail"
messenger_type = "gmail"
enabled = true

# OAuth2 credentials (from Step 1.4)
client_id = "your-client-id-here.apps.googleusercontent.com"
client_secret = "your-client-secret-here"
refresh_token = "your-refresh-token-here"

# Optional settings
gmail_user = "me"              # "me" = authenticated user
gmail_label = "INBOX"          # Label to monitor
gmail_poll_interval = 60       # Poll every 60 seconds
gmail_unread_only = true       # Only process unread messages
```

### 3.2 Secure Credential Storage (Recommended)

For better security, store credentials in the vault instead of config.toml:

```bash
# Store credentials securely
rustyclaw secrets store GMAIL_CLIENT_ID "your-client-id"
rustyclaw secrets store GMAIL_CLIENT_SECRET "your-client-secret"
rustyclaw secrets store GMAIL_REFRESH_TOKEN "your-refresh-token"
```

Then update config.toml to read from environment variables:

```toml
[[messengers]]
name = "my-gmail"
messenger_type = "gmail"
enabled = true

# Credentials will be read from vault via environment
# Set these in your shell startup file:
# export GMAIL_CLIENT_ID=$(rustyclaw secrets get GMAIL_CLIENT_ID)
# export GMAIL_CLIENT_SECRET=$(rustyclaw secrets get GMAIL_CLIENT_SECRET)
# export GMAIL_REFRESH_TOKEN=$(rustyclaw secrets get GMAIL_REFRESH_TOKEN)
```

## Step 4: Test the Integration

### 4.1 Start the Gateway

```bash
rustyclaw gateway
```

You should see:

```
[messenger] Initialized my-gmail (gmail)
[gmail] Initialized successfully
[gateway] Messenger polling enabled
```

### 4.2 Send a Test Email

1. Send an email to your Gmail address
2. Subject: "Test RustyClaw"
3. Body: "What's the weather like today?"

### 4.3 Check Logs

The gateway should:
1. Poll Gmail and find the new message
2. Process it through the AI model
3. Send a reply

Watch the console for:

```
[messenger] Received message from: sender@example.com
[messenger] Processing: Test RustyClaw
...
[gmail] Sent reply to sender@example.com
```

## Configuration Options

### Required Fields

| Field | Description | Example |
|-------|-------------|---------|
| `client_id` | OAuth2 client ID from Google Cloud Console | `123456.apps.googleusercontent.com` |
| `client_secret` | OAuth2 client secret | `GOCSPX-abc123` |
| `refresh_token` | OAuth2 refresh token from playground | `1//0abc123...` |

### Optional Fields

| Field | Default | Description |
|-------|---------|-------------|
| `gmail_user` | `"me"` | Gmail user ID (use "me" for authenticated user) |
| `gmail_label` | `"INBOX"` | Label to monitor (INBOX, SENT, etc.) |
| `gmail_poll_interval` | `60` | Poll interval in seconds |
| `gmail_unread_only` | `true` | Only process unread messages |

## Advanced Usage

### Multiple Labels

Monitor different labels with separate messenger instances:

```toml
[[messengers]]
name = "gmail-inbox"
messenger_type = "gmail"
enabled = true
client_id = "..."
client_secret = "..."
refresh_token = "..."
gmail_label = "INBOX"

[[messengers]]
name = "gmail-support"
messenger_type = "gmail"
enabled = true
client_id = "..."
client_secret = "..."
refresh_token = "..."
gmail_label = "SUPPORT"
```

### Auto-Reply Only to Specific Senders

Use Gmail filters to create a custom label:

1. In Gmail, go to Settings → Filters and Blocked Addresses
2. Create a new filter
3. From: specific-sender@example.com
4. Apply label: "RustyClaw"
5. Configure RustyClaw to monitor that label:

```toml
gmail_label = "RustyClaw"
```

### Integration with DM Pairing

For security, enable DM pairing to require authorization:

```toml
[pairing]
enabled = true
require_code = true
```

First-time senders will receive a pairing code. They must reply with the code to be authorized.

## Troubleshooting

### "Failed to get access token"

**Problem**: OAuth2 refresh failed

**Solutions**:
1. Verify client_id and client_secret are correct
2. Ensure refresh_token hasn't expired
3. Check that Gmail API is enabled in Google Cloud Console
4. Re-generate refresh token using OAuth2 Playground

### "Failed to list messages"

**Problem**: API permissions issue

**Solutions**:
1. Verify the correct scopes were added:
   - `gmail.modify`
   - `gmail.send`
2. Check that your Gmail account is listed as a test user
3. Ensure OAuth consent screen is configured

### "No messages received"

**Problem**: Polling isn't finding emails

**Solutions**:
1. Check `gmail_label` matches your Gmail labels exactly
2. Verify `gmail_unread_only` setting
3. Ensure there are actually unread messages in that label
4. Check gateway logs for polling activity

### "Rate limit exceeded"

**Problem**: Too many API requests

**Solutions**:
1. Increase `gmail_poll_interval` (e.g., to 120 seconds)
2. Check your Google Cloud Console API quotas
3. Ensure you're not running multiple instances

## Security Best Practices

1. **Never commit credentials** to version control
2. **Use the vault** for storing sensitive data
3. **Enable DM pairing** for authorization
4. **Restrict OAuth scopes** to minimum required
5. **Monitor API usage** in Google Cloud Console
6. **Rotate credentials** periodically
7. **Use test users** during development

## Limitations

- **Polling-based**: Checks for new messages at fixed intervals (not instant)
- **Gmail API quotas**: Free tier has daily limits
- **OAuth tokens**: Refresh tokens can expire if unused for 6 months
- **No attachments**: Currently only processes text content
- **No HTML rendering**: HTML emails are stripped to plain text

## Future Enhancements

- [ ] Gmail Pub/Sub for real-time notifications
- [ ] Attachment processing (images, PDFs)
- [ ] HTML email composition
- [ ] Thread management
- [ ] Draft handling
- [ ] Calendar integration

## References

- [Gmail API Documentation](https://developers.google.com/gmail/api)
- [OAuth2 for Desktop Apps](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Gmail API Scopes](https://developers.google.com/gmail/api/auth/scopes)
- [OAuth2 Playground](https://developers.google.com/oauthplayground/)
- [Google Cloud Console](https://console.cloud.google.com/)

## Support

If you encounter issues:

1. Check the [troubleshooting section](#troubleshooting)
2. Review gateway logs with `rustyclaw gateway --verbose`
3. Report issues at https://github.com/aecs4u/RustyClaw/issues
