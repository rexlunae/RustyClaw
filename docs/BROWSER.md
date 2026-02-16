# Browser Automation Guide

RustyClaw provides full browser automation capabilities using Chrome DevTools Protocol (CDP) through the chromiumoxide library. This enables AI agents to interact with web pages, fill forms, extract data, and perform complex web workflows.

## Overview

**Why Browser Automation?**
- 🌐 Interact with dynamic web applications
- 📝 Fill forms and submit data
- 📊 Extract data from JavaScript-heavy sites
- 🧪 Test web applications
- 📸 Capture screenshots and PDFs
- 🤖 Automate repetitive web tasks

**RustyClaw's Capabilities:**
- Chrome/Chromium browser control via CDP
- Tab management (open/close/navigate)
- Element interaction (click/type/press keys)
- JavaScript evaluation
- Screenshot capture (viewport or full page)
- Accessibility tree inspection
- Multiple tab support
- Headless or headed mode

---

## Installation & Setup

### 1. Build with Browser Feature

Browser automation is feature-gated and requires explicit compilation:

```bash
# Build with browser support
cargo build --release --features browser

# Or add to build command
rustyclaw --features browser chat
```

### 2. Install Chrome/Chromium

**Ubuntu/Debian:**
```bash
# Option A: Chrome
wget https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb
sudo dpkg -i google-chrome-stable_current_amd64.deb

# Option B: Chromium
sudo apt-get install chromium-browser
```

**macOS:**
```bash
# Chrome
brew install --cask google-chrome

# Or Chromium
brew install chromium
```

**Fedora/RHEL:**
```bash
sudo dnf install chromium
```

### 3. Verify Installation

```bash
# Check if browser is accessible
rustyclaw command "browser status"

# Expected output:
# {
#   "running": false,
#   "tabs": 0,
#   "profile": "rustyclaw"
# }
```

---

## Quick Start

### Start Browser

```bash
# Start browser (headed mode - shows window)
rustyclaw command "browser start"

# Output: "Browser started successfully."
```

### Open a Page

```bash
rustyclaw command 'browser open https://example.com'

# Returns:
# {
#   "success": true,
#   "tabId": "tab_0",
#   "url": "https://example.com"
# }
```

### Get Page Snapshot

```bash
rustyclaw command "browser snapshot"

# Returns interactive elements:
# {
#   "title": "Example Domain",
#   "url": "https://example.com",
#   "elements": [
#     {"ref": "e0", "tag": "a", "name": "More information...", "href": "..."}
#   ]
# }
```

### Click an Element

```bash
rustyclaw command 'browser act click e0'

# Clicks the first link found in snapshot
```

### Stop Browser

```bash
rustyclaw command "browser stop"

# Output: "Browser stopped."
```

---

## Browser Actions

### 1. Lifecycle Management

#### Status

Check if browser is running:

```json
{
  "tool": "browser",
  "action": "status"
}
```

Response:
```json
{
  "running": true,
  "tabs": 2,
  "profile": "rustyclaw"
}
```

#### Start

Launch browser:

```json
{
  "tool": "browser",
  "action": "start"
}
```

By default, launches in **headed mode** (visible window). See Configuration for headless mode.

#### Stop

Close browser and all tabs:

```json
{
  "tool": "browser",
  "action": "stop"
}
```

---

### 2. Tab Management

#### List Tabs

Get all open tabs:

```json
{
  "tool": "browser",
  "action": "tabs"
}
```

Response:
```json
{
  "tabs": [
    {"id": "tab_0"},
    {"id": "tab_1"}
  ]
}
```

#### Open Tab

Open new tab with URL:

```json
{
  "tool": "browser",
  "action": "open",
  "targetUrl": "https://github.com"
}
```

Response:
```json
{
  "success": true,
  "tabId": "tab_1",
  "url": "https://github.com"
}
```

#### Navigate

Navigate existing tab to new URL:

```json
{
  "tool": "browser",
  "action": "navigate",
  "targetId": "tab_0",
  "targetUrl": "https://google.com"
}
```

If `targetId` is omitted, uses the first available tab.

#### Close Tab

Close specific tab:

```json
{
  "tool": "browser",
  "action": "close",
  "targetId": "tab_0"
}
```

---

### 3. Page Inspection

#### Snapshot

Get accessibility tree with interactive elements:

```json
{
  "tool": "browser",
  "action": "snapshot",
  "targetId": "tab_0"  // Optional
}
```

Response:
```json
{
  "title": "GitHub: Let's build from here",
  "url": "https://github.com",
  "elements": [
    {
      "ref": "e0",
      "tag": "a",
      "role": "link",
      "name": "Sign in",
      "href": "https://github.com/login"
    },
    {
      "ref": "e1",
      "tag": "button",
      "role": "button",
      "name": "Sign up"
    },
    {
      "ref": "e2",
      "tag": "input",
      "type": "search",
      "name": "",
      "placeholder": "Search GitHub"
    }
  ]
}
```

**Limited to first 50 elements** for performance.

#### Screenshot

Capture page as PNG:

```json
{
  "tool": "browser",
  "action": "screenshot",
  "targetId": "tab_0",
  "fullPage": true  // Optional, default false
}
```

Response:
```json
{
  "success": true,
  "format": "png",
  "data": "data:image/png;base64,iVBORw0KGgo..."
}
```

**Full page** captures entire scrollable content (can be large!).

---

### 4. Page Interaction

All interactions use the `act` action with different kinds.

#### Click Element

Click element by reference from snapshot:

```json
{
  "tool": "browser",
  "action": "act",
  "targetId": "tab_0",
  "request": {
    "kind": "click",
    "ref": "e0"
  }
}
```

Or by CSS selector:

```json
{
  "tool": "browser",
  "action": "act",
  "request": {
    "kind": "click",
    "ref": "button.submit-btn"
  }
}
```

#### Type Text

Type into input field:

```json
{
  "tool": "browser",
  "action": "act",
  "request": {
    "kind": "type",
    "ref": "e2",
    "text": "rustyclaw"
  }
}
```

Automatically clicks element before typing.

#### Press Key

Press keyboard key:

```json
{
  "tool": "browser",
  "action": "act",
  "request": {
    "kind": "press",
    "key": "Enter"
  }
}
```

Common keys: `Enter`, `Escape`, `Tab`, `Backspace`, `ArrowDown`, `ArrowUp`.

#### Evaluate JavaScript

Execute JavaScript in page context:

```json
{
  "tool": "browser",
  "action": "act",
  "request": {
    "kind": "evaluate",
    "fn": "document.title"
  }
}
```

Returns the result as JSON.

**Security note:** Be careful with user-provided JavaScript!

---

## Configuration

### Headless Mode

Edit `src/tools/browser.rs` to enable headless:

```rust
// In ensure_browser() function:
let config = BrowserConfig::builder()
    .headless()  // Change from .with_head()
    .viewport(None)
    .build()
    .map_err(|e| format!("Failed to build browser config: {}", e))?;
```

Rebuild:
```bash
cargo build --release --features browser
```

### Custom Viewport

```rust
let config = BrowserConfig::builder()
    .with_head()
    .viewport(chromiumoxide::handler::viewport::Viewport {
        width: 1920,
        height: 1080,
        device_scale_factor: Some(1.0),
        emulating_mobile: false,
        is_landscape: false,
        has_touch: false,
    })
    .build()?;
```

### Browser Path

By default, chromiumoxide searches for Chrome/Chromium in standard locations:
- `/usr/bin/google-chrome`
- `/usr/bin/chromium-browser`
- `/usr/bin/chromium`
- `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` (macOS)

To specify custom path:
```rust
let config = BrowserConfig::builder()
    .chrome_executable("/path/to/chrome")
    .build()?;
```

---

## Common Use Cases

### 1. Fill Web Form

```javascript
// 1. Open form page
browser open https://example.com/contact

// 2. Get elements
browser snapshot

// 3. Fill fields
browser act type e0 "John Doe"     // Name field
browser act type e1 "john@example.com"  // Email field
browser act type e2 "Hello world"  // Message field

// 4. Submit
browser act click e3  // Submit button
```

### 2. Search and Extract Data

```javascript
// 1. Open search engine
browser open https://google.com

// 2. Get search box
browser snapshot  // Find search input ref

// 3. Search
browser act type e0 "rustyclaw github"
browser act press Enter

// 4. Wait for results (delay)
execute_command "sleep 2"

// 5. Take screenshot
browser screenshot --fullPage
```

### 3. Login Automation

```javascript
// 1. Navigate to login
browser open https://example.com/login

// 2. Get form elements
browser snapshot

// 3. Enter credentials
browser act type e0 "username"
browser act type e1 "password"

// 4. Click login
browser act click e2

// 5. Verify success (check URL)
browser evaluate "window.location.href"
```

### 4. Data Scraping

```javascript
// 1. Open target page
browser open https://example.com/products

// 2. Extract data with JavaScript
browser evaluate `
  Array.from(document.querySelectorAll('.product')).map(p => ({
    name: p.querySelector('.name').textContent,
    price: p.querySelector('.price').textContent
  }))
`
```

### 5. Multi-Tab Workflow

```javascript
// 1. Open multiple tabs
browser open https://github.com  // Returns tab_0
browser open https://stackoverflow.com  // Returns tab_1

// 2. Work in first tab
browser snapshot --targetId tab_0

// 3. Switch to second tab
browser navigate --targetId tab_1 https://stackoverflow.com/questions

// 4. Close tab when done
browser close --targetId tab_0
```

---

## Advanced Features

### Extract Complex Data

```javascript
browser evaluate `
(() => {
  // Get all links with metadata
  const links = Array.from(document.querySelectorAll('a')).map(a => ({
    text: a.textContent.trim(),
    href: a.href,
    visible: a.offsetParent !== null
  }));

  // Get form structure
  const forms = Array.from(document.querySelectorAll('form')).map(f => ({
    action: f.action,
    method: f.method,
    fields: Array.from(f.elements).map(e => ({
      name: e.name,
      type: e.type,
      required: e.required
    }))
  }));

  return { links, forms };
})()
`
```

### Wait for Element

```javascript
browser evaluate `
new Promise((resolve) => {
  const check = () => {
    const el = document.querySelector('.async-content');
    if (el) resolve(true);
    else setTimeout(check, 100);
  };
  check();
})
`
```

### Scroll Page

```javascript
browser evaluate "window.scrollTo(0, document.body.scrollHeight)"
```

### Get Cookies

```javascript
browser evaluate "document.cookie"
```

### Local Storage

```javascript
browser evaluate "JSON.stringify(localStorage)"
```

---

## Troubleshooting

### "Browser not found"

**Problem:**
```
Failed to launch browser: Chrome/Chromium not found
```

**Solution:**
```bash
# Install Chrome/Chromium
sudo apt-get install chromium-browser  # Linux
brew install --cask google-chrome      # macOS

# Or specify custom path in config
```

---

### "Browser feature not enabled"

**Problem:**
```
Browser automation requires the 'browser' feature
```

**Solution:**
```bash
# Rebuild with browser feature
cargo build --release --features browser
```

---

### "Failed to connect to browser"

**Problem:**
```
Failed to launch browser: Connection refused
```

**Possible causes:**
1. Browser process crashed
2. Port 9222 already in use
3. Insufficient permissions

**Solution:**
```bash
# 1. Stop existing Chrome instances
pkill chrome
pkill chromium

# 2. Check port availability
lsof -i :9222

# 3. Restart browser
rustyclaw command "browser stop"
rustyclaw command "browser start"
```

---

### Element Not Found

**Problem:**
```
Element not found: e5
```

**Debug:**
```javascript
// 1. Get fresh snapshot
browser snapshot

// 2. Check element still exists
browser evaluate "document.querySelector('.my-element')"

// 3. Wait for dynamic content
execute_command "sleep 2"
browser snapshot
```

---

### Timeout Issues

**Problem:** Pages load slowly or hang.

**Solution:**
```javascript
// Set navigation timeout (in browser config)
// Default is 30 seconds

// Or check if page is loaded
browser evaluate "document.readyState"  // Should be "complete"
```

---

## Security Considerations

### 1. Sanitize User Input

⚠️ **Never execute untrusted JavaScript:**

```javascript
// BAD:
browser evaluate userInput  // Could be malicious!

// GOOD:
browser evaluate `document.querySelector('${CSS.escape(userSelector)}')`
```

### 2. Validate URLs

```rust
// Validate URL before opening
if !url.starts_with("https://") {
    return Err("Only HTTPS URLs allowed".to_string());
}
```

### 3. Limit Automation Scope

```toml
# In skills/browser-skill.toml
[gating]
allowed_domains = [
    "example.com",
    "trusted-site.com"
]
```

### 4. Screenshot Privacy

Screenshots may contain sensitive information:
- Passwords (if visible in cleartext)
- Personal data
- Session tokens

**Best practice:** Mask sensitive fields before capture.

---

## Performance Tips

### 1. Reuse Tabs

Instead of opening new tabs:
```javascript
// BAD (creates many tabs):
for url in urls:
    browser open url

// GOOD (reuse tab):
browser open urls[0]
for url in urls[1:]:
    browser navigate url
```

### 2. Use Headless Mode

Headless is faster (no rendering overhead):
```rust
.headless()  // vs .with_head()
```

### 3. Disable Images

For text-only scraping:
```javascript
browser evaluate `
  const style = document.createElement('style');
  style.textContent = 'img { display: none !important; }';
  document.head.appendChild(style);
`
```

### 4. Limit Snapshot Size

Only first 50 elements are returned. For large pages, target specific areas:
```javascript
browser evaluate `
  Array.from(document.querySelectorAll('.specific-section a')).slice(0, 20)
`
```

---

## Comparison: RustyClaw vs Competitors

| Feature | RustyClaw | IronClaw | OpenClaw | PicoClaw |
|---------|-----------|----------|----------|----------|
| **CDP Support** | ✅ chromiumoxide | ✅ chromiumoxide | ✅ puppeteer | ❌ None |
| **Headless Mode** | ✅ Yes | ✅ Yes | ✅ Yes | ❌ N/A |
| **Profile Management** | ⚠️ Single | ✅ Multiple | ✅ Multiple | ❌ N/A |
| **Screenshot** | ✅ PNG | ✅ PNG/JPEG | ✅ PNG | ❌ N/A |
| **JavaScript Eval** | ✅ Yes | ✅ Yes | ✅ Yes | ❌ N/A |
| **Tab Management** | ✅ Multiple | ✅ Multiple | ✅ Multiple | ❌ N/A |
| **A11y Tree** | ✅ Snapshot | ✅ Full | ✅ Full | ❌ N/A |
| **Feature-Gated** | ✅ Optional | ✅ Optional | ❌ Always | N/A |

**RustyClaw Advantages:**
- Feature-gated (smaller binary when not needed)
- Pure Rust implementation (type safety)
- Integrated with sandbox system

**Areas for Enhancement:**
- Multiple browser profiles
- Full accessibility tree inspection
- PDF generation
- Console log capture

---

## API Reference

### Actions

| Action | Description | Required Params | Optional Params |
|--------|-------------|-----------------|-----------------|
| `status` | Get browser status | - | - |
| `start` | Launch browser | - | - |
| `stop` | Close browser | - | - |
| `tabs` | List open tabs | - | - |
| `open` | Open new tab | `targetUrl` | - |
| `navigate` | Navigate to URL | `targetUrl` | `targetId` |
| `close` | Close tab | `targetId` | - |
| `snapshot` | Get page elements | - | `targetId` |
| `screenshot` | Capture page | - | `targetId`, `fullPage` |
| `act` | Interact with page | `request` | `targetId` |

### Act Kinds

| Kind | Description | Required in `request` | Optional |
|------|-------------|----------------------|----------|
| `click` | Click element | `ref` | - |
| `type` | Type text | `ref`, `text` | - |
| `press` | Press key | `key` | - |
| `evaluate` | Run JavaScript | `fn` | - |

### Element Reference

Elements are identified by:
- `ref` from snapshot (e.g., `e0`, `e1`, ...)
- CSS selector (e.g., `button.submit`, `#login-form`)

---

## FAQ

### Q: Can I run multiple browsers?

**A:** Currently, RustyClaw uses a singleton browser instance. Multiple tabs are supported, but not multiple browser processes.

**Workaround:** Use multiple RustyClaw instances or spawn sub-sessions.

---

### Q: Does it work with Firefox/Safari?

**A:** No, only Chrome/Chromium (CDP protocol). Firefox uses its own protocol (Marionette), and Safari has limited automation support.

---

### Q: Can I use browser extensions?

**A:** Not currently. The browser profile is managed and doesn't load extensions.

**Future:** Profile management could enable extension support.

---

### Q: How do I handle authentication?

**A:** Use the `browser` tool to fill login forms, or inject cookies via JavaScript:
```javascript
browser evaluate "document.cookie = 'session=abc123; path=/'"
```

---

### Q: Can I automate file downloads?

**A:** Yes, but files download to the browser's default download directory. Monitor that directory with file system tools.

---

### Q: What about CAPTCHAs?

**A:** CAPTCHAs are designed to prevent automation. You'll need:
1. Human intervention
2. CAPTCHA solving services (external)
3. API alternatives (avoid web scraping)

---

## Resources

- [Chromiumoxide Documentation](https://docs.rs/chromiumoxide/)
- [Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/)
- [RustyClaw Tools Guide](./TOOLS.md)
- [RustyClaw Security Guide](./SECURITY.md)

---

## Summary

**RustyClaw Browser Automation Provides:**
- ✅ Full Chrome/Chromium control via CDP
- ✅ Tab management and navigation
- ✅ Element interaction (click/type/press)
- ✅ JavaScript evaluation
- ✅ Screenshots and accessibility snapshots
- ✅ Feature-gated compilation (optional)
- ✅ Integration with RustyClaw security system

**Getting Started:**
```bash
# 1. Build with feature
cargo build --release --features browser

# 2. Start browser
rustyclaw command "browser start"

# 3. Open page
rustyclaw command "browser open https://example.com"

# 4. Get elements
rustyclaw command "browser snapshot"

# 5. Interact
rustyclaw command 'browser act click e0'
```

**Happy automating! 🤖🦞**
