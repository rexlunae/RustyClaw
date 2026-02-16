# RustyClaw GUI - PySide6 Desktop Application

A modern Qt-based desktop GUI for RustyClaw, inspired by AutoGPT frontend patterns.

## Features

### 🎨 AutoGPT-Inspired Interface
- **Chat Interface**: Streaming responses with rich formatting
- **Task Visualization**: Real-time task status and progress tracking
- **File Browser**: Workspace file navigation and selection
- **Execution Logs**: Color-coded debugging and monitoring
- **Multi-Agent Dashboard**: Coordinate multiple AI agents

### 🔌 RustyClaw Integration
- WebSocket connection to RustyClaw gateway
- Real-time bidirectional communication
- Tool execution visualization
- Authentication support (TOTP)
- TLS/WSS support for secure connections

### 💎 Modern UI/UX
- Dark theme optimized for long sessions
- Responsive layout with resizable panels
- Keyboard shortcuts for common actions
- Status indicators and notifications
- File syntax highlighting (planned)

## Installation

### Prerequisites
- Python 3.10+
- RustyClaw gateway running (default: `ws://localhost:8080`)

### Setup

```bash
# Navigate to GUI directory
cd gui

# Install dependencies
pip install -r requirements.txt

# Or use a virtual environment (recommended)
python -m venv venv
source venv/bin/activate  # On Windows: venv\Scripts\activate
pip install -r requirements.txt
```

## Usage

### Basic Launch

```bash
# From the gui directory
python rustyclaw_gui.py

# Or make it executable
chmod +x rustyclaw_gui.py
./rustyclaw_gui.py
```

### Connect to Gateway

1. **Start RustyClaw Gateway**:
   ```bash
   rustyclaw gateway start
   ```

2. **Launch GUI**:
   ```bash
   python rustyclaw_gui.py
   ```

3. **Click "Connect"** in the toolbar or use `File > Connect`

4. **Start Chatting**: Type your message in the input field and press Enter

### With TLS/WSS

If your gateway uses TLS:

```python
# Edit rustyclaw_gui.py, line ~48:
self.ws_client = WebSocketClient("wss://localhost:8443")
```

### With Authentication

If TOTP is enabled, the GUI will prompt for your authentication code.

## UI Layout

```
┌─────────────────────────────────────────────────────────────┐
│ File  View  Help                          [Connect] Status  │
├────────┬─────────────────────────────────────┬──────────────┤
│        │                                     │              │
│ Tasks  │         Chat Interface             │ Execution    │
│  🔵    │                                     │ Logs         │
│  🟡    │  You: Hello                        │              │
│  🟢    │  Agent: Hello! How can I help?     │ [INFO] ...   │
│        │                                     │ [DEBUG] ...  │
│ Files  │  [Type message here...] [Send]     │ [ERROR] ...  │
│  📁    │                                     │              │
│  📄    │                                     │              │
│        │                                     │              │
└────────┴─────────────────────────────────────┴──────────────┘
```

## Keyboard Shortcuts

- `Ctrl+N`: New chat
- `Ctrl+W`: Clear chat
- `Ctrl+L`: Clear logs
- `Ctrl+Q`: Quit
- `Enter`: Send message
- `Ctrl+Enter`: Send message (alternative)

## Configuration

### Default Gateway URL

Edit `rustyclaw_gui.py` to change default connection:

```python
class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        # Change this line:
        self.ws_client = WebSocketClient("ws://localhost:8080")
```

### Workspace Directory

Change default workspace for file browser:

```python
self.file_browser = FileBrowserPanel("/path/to/workspace")
```

## Development

### Project Structure

```
gui/
├── rustyclaw_gui.py     # Main application
├── requirements.txt      # Python dependencies
├── README.md            # This file
└── __init__.py          # Package marker
```

### Adding Features

Key components to extend:

- **ChatPanel**: Chat interface logic
- **TaskPanel**: Task visualization
- **LogsPanel**: Log display and filtering
- **FileBrowserPanel**: File navigation
- **WebSocketClient**: Gateway communication

### AutoGPT-Inspired Features Implemented

- ✅ Real-time chat with streaming
- ✅ Task status visualization
- ✅ File browser integration
- ✅ Execution logs with color coding
- ✅ Connection status indicators
- ✅ Dark theme UI

### Planned Features

- [ ] Syntax highlighting for code
- [ ] Multi-agent coordination view
- [ ] Task creation wizard
- [ ] Settings/preferences dialog
- [ ] Export chat history
- [ ] Notification system
- [ ] Plugin/extension system
- [ ] Metrics dashboard

## Troubleshooting

### Connection Failed

**Problem**: "Connection failed: [Errno 111] Connection refused"

**Solution**:
1. Ensure RustyClaw gateway is running: `rustyclaw gateway start`
2. Check the gateway URL in the GUI matches your gateway address
3. Verify no firewall is blocking the connection

### Authentication Required

**Problem**: GUI prompts for authentication

**Solution**:
1. If TOTP is enabled, enter your 6-digit code from your authenticator app
2. Or disable TOTP in RustyClaw config: `totp_enabled = false`

### GUI Not Responding

**Problem**: GUI freezes or becomes unresponsive

**Solution**:
1. Check terminal for error messages
2. Ensure async event loop is running properly
3. Try restarting the application

## Contributing

To contribute to the GUI:

1. Follow PySide6 best practices
2. Maintain AutoGPT-inspired design patterns
3. Test with RustyClaw gateway
4. Document new features

## License

MIT License - Same as RustyClaw main project
