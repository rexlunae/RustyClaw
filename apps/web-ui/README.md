# RustyClaw Web UI

Mobile-friendly Progressive Web App (PWA) for controlling RustyClaw from any device.

## Features

- 📱 **Mobile-First Design** - Optimized for smartphones and tablets
- 💬 **Real-time Chat** - Send messages and receive responses via WebSocket
- 📊 **Session Management** - View and manage conversation sessions
- ⚙️ **Settings** - Configure gateway connection
- 🔌 **Offline Support** - Works without internet via service worker
- 📲 **Install as App** - Add to home screen on iOS/Android

## Quick Start

### 1. Start RustyClaw Gateway

Make sure the RustyClaw gateway is running with WebSocket support:

```bash
cargo run --release
```

The gateway should be listening on `ws://localhost:8080` by default.

### 2. Serve the Web UI

Use any static file server. For example, with Python:

```bash
cd apps/web-ui
python3 -m http.server 8000
```

Or with Node.js:

```bash
npx serve apps/web-ui
```

### 3. Open in Browser

Navigate to `http://localhost:8000` in your browser.

### 4. Connect

1. Go to the **Settings** tab
2. Enter your gateway URL (default: `ws://localhost:8080`)
3. Click **Connect**
4. Switch to the **Chat** tab to start messaging

## Install as PWA

### iOS (iPhone/iPad)

1. Open the web UI in Safari
2. Tap the Share button (box with arrow)
3. Scroll down and tap "Add to Home Screen"
4. Tap "Add" in the top right

### Android

1. Open the web UI in Chrome
2. Tap the menu (three dots)
3. Tap "Add to Home Screen" or "Install App"
4. Tap "Install"

### Desktop (Chrome/Edge)

1. Open the web UI
2. Look for the install icon in the address bar
3. Click "Install"

## Usage

### Chat Tab

- Type messages in the input box at the bottom
- Press Enter to send (Shift+Enter for new line)
- Messages appear in real-time
- Scroll through message history

### Sessions Tab

- View all active conversation sessions
- See message counts and timestamps
- Click a session to switch to it (coming soon)

### Settings Tab

- Configure gateway WebSocket URL
- Connect/disconnect from gateway
- Settings are saved locally

## Architecture

```
┌─────────────┐         WebSocket          ┌──────────────┐
│  Web UI     │ ◄─────────────────────────► │   Gateway    │
│  (Browser)  │    JSON Messages            │  (Rust)      │
└─────────────┘                             └──────────────┘
      │                                            │
      │ Service Worker                             │ Messengers
      ▼                                            ▼
┌─────────────┐                          ┌──────────────┐
│   Cache     │                          │ Telegram     │
│  (Offline)  │                          │ Discord      │
└─────────────┘                          │ Matrix       │
                                         └──────────────┘
```

## WebSocket Protocol

Messages are JSON-encoded with the following types:

### Client → Server

```json
{
  "type": "handshake",
  "client": "web-ui",
  "version": "0.0.1"
}
```

```json
{
  "type": "chat",
  "content": "User message here",
  "session_id": "optional-session-id"
}
```

### Server → Client

```json
{
  "type": "handshake_ack",
  "session_id": "unique-session-id"
}
```

```json
{
  "type": "message",
  "role": "assistant",
  "content": "Response message",
  "timestamp": 1234567890
}
```

```json
{
  "type": "session_update",
  "sessions": [...]
}
```

## Files

- `index.html` - Main HTML structure and CSS styles
- `app.js` - WebSocket client and UI logic
- `manifest.json` - PWA manifest for installation
- `sw.js` - Service worker for offline support
- `icon.svg` - App icon (RustyClaw crab logo)

## Development

The web UI is built with vanilla HTML/CSS/JavaScript for simplicity:

- No build process required
- No dependencies
- Works in any modern browser
- Minimal file size

## Troubleshooting

### Can't connect to gateway

- Ensure RustyClaw gateway is running
- Check the WebSocket URL in Settings
- Verify firewall isn't blocking port 8080
- Check browser console for errors

### Messages not appearing

- Verify WebSocket connection (status indicator should be green)
- Check browser console for protocol errors
- Try refreshing the page

### PWA not installing

- Ensure you're using HTTPS (or localhost)
- Check that manifest.json is accessible
- Verify service worker registration in DevTools

## Future Enhancements

- [ ] Voice input/output
- [ ] File attachments
- [ ] Session switching
- [ ] Notification support
- [ ] Dark/light theme toggle
- [ ] Multi-language support
