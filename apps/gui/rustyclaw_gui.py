#!/usr/bin/env python3
"""
RustyClaw GUI - PySide6 Desktop Application

A modern Qt-based GUI for RustyClaw inspired by AutoGPT frontend patterns.
Connects to RustyClaw gateway via WebSocket for real-time agent communication.

Features:
- Chat interface with streaming responses
- Task/agent visualization
- File browser with code editor
- Execution logs viewer
- Multi-agent coordination dashboard
- Real-time status updates
"""

import sys
import asyncio
import json
from pathlib import Path
from typing import Optional, Dict, Any

from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QVBoxLayout, QHBoxLayout,
    QSplitter, QTabWidget, QTextEdit, QLineEdit, QPushButton,
    QListWidget, QTreeView, QLabel, QStatusBar, QMenuBar,
    QToolBar, QDockWidget, QFileSystemModel
)
from PySide6.QtCore import Qt, Signal, Slot, QTimer, QThread, QObject
from PySide6.QtGui import QAction, QTextCursor, QFont, QColor, QPalette
import websockets
from qasync import QEventLoop, asyncSlot


class WebSocketClient(QObject):
    """Async WebSocket client for RustyClaw gateway communication"""

    message_received = Signal(dict)
    connection_status = Signal(bool, str)
    error_occurred = Signal(str)

    def __init__(self, url: str = "ws://localhost:8080"):
        super().__init__()
        self.url = url
        self.websocket: Optional[websockets.WebSocketClientProtocol] = None
        self.connected = False

    async def connect(self):
        """Establish WebSocket connection to RustyClaw gateway"""
        try:
            self.websocket = await websockets.connect(self.url)
            self.connected = True
            self.connection_status.emit(True, f"Connected to {self.url}")

            # Start listening for messages
            asyncio.create_task(self._listen())

        except Exception as e:
            self.connected = False
            self.connection_status.emit(False, f"Connection failed: {e}")
            self.error_occurred.emit(str(e))

    async def _listen(self):
        """Listen for incoming messages from gateway"""
        try:
            async for message in self.websocket:
                try:
                    data = json.loads(message)
                    self.message_received.emit(data)
                except json.JSONDecodeError as e:
                    self.error_occurred.emit(f"Invalid JSON: {e}")
        except websockets.exceptions.ConnectionClosed:
            self.connected = False
            self.connection_status.emit(False, "Connection closed")
        except Exception as e:
            self.error_occurred.emit(f"Listen error: {e}")

    async def send_message(self, message: Dict[str, Any]):
        """Send message to RustyClaw gateway"""
        if not self.connected or not self.websocket:
            self.error_occurred.emit("Not connected to gateway")
            return

        try:
            await self.websocket.send(json.dumps(message))
        except Exception as e:
            self.error_occurred.emit(f"Send error: {e}")

    async def disconnect(self):
        """Close WebSocket connection"""
        if self.websocket:
            await self.websocket.close()
            self.connected = False
            self.connection_status.emit(False, "Disconnected")


class ChatPanel(QWidget):
    """Chat interface panel inspired by AutoGPT chat UI"""

    send_message = Signal(str)

    def __init__(self):
        super().__init__()
        self.init_ui()

    def init_ui(self):
        layout = QVBoxLayout(self)

        # Chat history display
        self.chat_display = QTextEdit()
        self.chat_display.setReadOnly(True)
        self.chat_display.setFont(QFont("Courier", 10))
        layout.addWidget(self.chat_display)

        # Input area
        input_layout = QHBoxLayout()

        self.input_field = QLineEdit()
        self.input_field.setPlaceholderText("Type your message here...")
        self.input_field.returnPressed.connect(self.on_send)
        input_layout.addWidget(self.input_field)

        self.send_button = QPushButton("Send")
        self.send_button.clicked.connect(self.on_send)
        input_layout.addWidget(self.send_button)

        layout.addLayout(input_layout)

    def on_send(self):
        """Handle send button click"""
        message = self.input_field.text().strip()
        if message:
            self.send_message.emit(message)
            self.add_user_message(message)
            self.input_field.clear()

    def add_user_message(self, message: str):
        """Add user message to chat display"""
        self.chat_display.append(f'<p style="color: #2196F3;"><b>You:</b> {message}</p>')
        self.chat_display.verticalScrollBar().setValue(
            self.chat_display.verticalScrollBar().maximum()
        )

    def add_agent_message(self, message: str):
        """Add agent message to chat display"""
        self.chat_display.append(f'<p style="color: #4CAF50;"><b>Agent:</b> {message}</p>')
        self.chat_display.verticalScrollBar().setValue(
            self.chat_display.verticalScrollBar().maximum()
        )

    def add_system_message(self, message: str):
        """Add system message to chat display"""
        self.chat_display.append(f'<p style="color: #FF9800;"><i>{message}</i></p>')
        self.chat_display.verticalScrollBar().setValue(
            self.chat_display.verticalScrollBar().maximum()
        )

    def add_error_message(self, message: str):
        """Add error message to chat display"""
        self.chat_display.append(f'<p style="color: #F44336;"><b>Error:</b> {message}</p>')
        self.chat_display.verticalScrollBar().setValue(
            self.chat_display.verticalScrollBar().maximum()
        )


class TaskPanel(QWidget):
    """Task visualization panel inspired by AutoGPT task list"""

    def __init__(self):
        super().__init__()
        self.init_ui()

    def init_ui(self):
        layout = QVBoxLayout(self)

        # Title
        title = QLabel("<h2>Active Tasks</h2>")
        layout.addWidget(title)

        # Task list
        self.task_list = QListWidget()
        layout.addWidget(self.task_list)

        # Task controls
        controls = QHBoxLayout()

        self.add_task_button = QPushButton("Add Task")
        controls.addWidget(self.add_task_button)

        self.clear_button = QPushButton("Clear Completed")
        controls.addWidget(self.clear_button)

        layout.addLayout(controls)

    def add_task(self, task_id: str, description: str, status: str = "pending"):
        """Add a task to the list"""
        status_color = {
            "pending": "🔵",
            "in_progress": "🟡",
            "completed": "🟢",
            "failed": "🔴"
        }.get(status, "⚪")

        self.task_list.addItem(f"{status_color} [{task_id}] {description}")

    def update_task_status(self, task_id: str, status: str):
        """Update task status in the list"""
        # Implementation would search and update matching task
        pass


class LogsPanel(QWidget):
    """Execution logs panel for debugging and monitoring"""

    def __init__(self):
        super().__init__()
        self.init_ui()

    def init_ui(self):
        layout = QVBoxLayout(self)

        # Title and controls
        header = QHBoxLayout()
        title = QLabel("<h2>Execution Logs</h2>")
        header.addWidget(title)
        header.addStretch()

        self.clear_logs_button = QPushButton("Clear")
        self.clear_logs_button.clicked.connect(self.clear_logs)
        header.addWidget(self.clear_logs_button)

        layout.addLayout(header)

        # Log display
        self.log_display = QTextEdit()
        self.log_display.setReadOnly(True)
        self.log_display.setFont(QFont("Courier", 9))
        self.log_display.setStyleSheet("background-color: #1E1E1E; color: #D4D4D4;")
        layout.addWidget(self.log_display)

    def add_log(self, level: str, message: str):
        """Add a log entry"""
        colors = {
            "INFO": "#61AFEF",
            "WARNING": "#E5C07B",
            "ERROR": "#E06C75",
            "DEBUG": "#98C379",
            "SUCCESS": "#56B6C2"
        }
        color = colors.get(level, "#ABB2BF")

        self.log_display.append(
            f'<span style="color: {color};">[{level}]</span> {message}'
        )
        self.log_display.verticalScrollBar().setValue(
            self.log_display.verticalScrollBar().maximum()
        )

    def clear_logs(self):
        """Clear all logs"""
        self.log_display.clear()


class FileBrowserPanel(QWidget):
    """File browser panel for workspace navigation"""

    file_selected = Signal(str)

    def __init__(self, root_path: str = "."):
        super().__init__()
        self.root_path = root_path
        self.init_ui()

    def init_ui(self):
        layout = QVBoxLayout(self)

        # Title
        title = QLabel("<h2>Workspace Files</h2>")
        layout.addWidget(title)

        # File tree view
        self.model = QFileSystemModel()
        self.model.setRootPath(self.root_path)

        self.tree = QTreeView()
        self.tree.setModel(self.model)
        self.tree.setRootIndex(self.model.index(self.root_path))
        self.tree.setColumnWidth(0, 250)
        self.tree.clicked.connect(self.on_file_clicked)

        layout.addWidget(self.tree)

    def on_file_clicked(self, index):
        """Handle file selection"""
        file_path = self.model.filePath(index)
        self.file_selected.emit(file_path)


class MainWindow(QMainWindow):
    """Main application window with AutoGPT-inspired layout"""

    def __init__(self):
        super().__init__()
        self.ws_client = WebSocketClient()
        self.init_ui()
        self.setup_connections()

    def init_ui(self):
        self.setWindowTitle("RustyClaw - AI Agent Desktop")
        self.setGeometry(100, 100, 1400, 900)

        # Create menu bar
        self.create_menu_bar()

        # Create toolbar
        self.create_toolbar()

        # Create central widget with splitter layout
        central_widget = QWidget()
        self.setCentralWidget(central_widget)

        main_layout = QHBoxLayout(central_widget)

        # Main splitter (left/right)
        main_splitter = QSplitter(Qt.Horizontal)

        # Left panel - Tasks and File Browser
        left_panel = QTabWidget()
        self.task_panel = TaskPanel()
        self.file_browser = FileBrowserPanel()
        left_panel.addTab(self.task_panel, "Tasks")
        left_panel.addTab(self.file_browser, "Files")
        left_panel.setMaximumWidth(400)

        # Center panel - Chat
        self.chat_panel = ChatPanel()

        # Right panel - Logs
        self.logs_panel = LogsPanel()
        self.logs_panel.setMaximumWidth(400)

        # Add panels to splitter
        main_splitter.addWidget(left_panel)
        main_splitter.addWidget(self.chat_panel)
        main_splitter.addWidget(self.logs_panel)

        # Set splitter sizes
        main_splitter.setSizes([300, 700, 400])

        main_layout.addWidget(main_splitter)

        # Status bar
        self.status_bar = QStatusBar()
        self.setStatusBar(self.status_bar)
        self.status_bar.showMessage("Disconnected")

        # Apply dark theme
        self.apply_theme()

    def create_menu_bar(self):
        """Create application menu bar"""
        menubar = self.menuBar()

        # File menu
        file_menu = menubar.addMenu("&File")

        connect_action = QAction("&Connect", self)
        connect_action.triggered.connect(self.connect_to_gateway)
        file_menu.addAction(connect_action)

        disconnect_action = QAction("&Disconnect", self)
        disconnect_action.triggered.connect(self.disconnect_from_gateway)
        file_menu.addAction(disconnect_action)

        file_menu.addSeparator()

        exit_action = QAction("E&xit", self)
        exit_action.triggered.connect(self.close)
        file_menu.addAction(exit_action)

        # View menu
        view_menu = menubar.addMenu("&View")

        # Help menu
        help_menu = menubar.addMenu("&Help")
        about_action = QAction("&About", self)
        help_menu.addAction(about_action)

    def create_toolbar(self):
        """Create application toolbar"""
        toolbar = QToolBar()
        self.addToolBar(toolbar)

        self.connect_btn = QAction("Connect", self)
        self.connect_btn.triggered.connect(self.connect_to_gateway)
        toolbar.addAction(self.connect_btn)

        self.disconnect_btn = QAction("Disconnect", self)
        self.disconnect_btn.triggered.connect(self.disconnect_from_gateway)
        toolbar.addAction(self.disconnect_btn)

        toolbar.addSeparator()

        self.status_label = QLabel(" Status: Disconnected ")
        toolbar.addWidget(self.status_label)

    def setup_connections(self):
        """Setup signal/slot connections"""
        # Chat panel
        self.chat_panel.send_message.connect(self.on_send_chat_message)

        # WebSocket client
        self.ws_client.message_received.connect(self.on_ws_message_received)
        self.ws_client.connection_status.connect(self.on_connection_status)
        self.ws_client.error_occurred.connect(self.on_ws_error)

        # File browser
        self.file_browser.file_selected.connect(self.on_file_selected)

    @asyncSlot()
    async def connect_to_gateway(self):
        """Connect to RustyClaw gateway"""
        self.logs_panel.add_log("INFO", "Connecting to RustyClaw gateway...")
        await self.ws_client.connect()

    @asyncSlot()
    async def disconnect_from_gateway(self):
        """Disconnect from gateway"""
        self.logs_panel.add_log("INFO", "Disconnecting from gateway...")
        await self.ws_client.disconnect()

    @asyncSlot()
    async def on_send_chat_message(self, message: str):
        """Handle chat message send"""
        self.logs_panel.add_log("DEBUG", f"Sending message: {message}")

        # Create chat request
        request = {
            "type": "chat",
            "messages": [
                {"role": "user", "content": message}
            ]
        }

        await self.ws_client.send_message(request)

    @Slot(dict)
    def on_ws_message_received(self, data: Dict[str, Any]):
        """Handle incoming WebSocket message"""
        msg_type = data.get("type", "unknown")

        self.logs_panel.add_log("DEBUG", f"Received: {msg_type}")

        if msg_type == "text":
            # Agent response
            content = data.get("text", "")
            self.chat_panel.add_agent_message(content)

        elif msg_type == "tool_call":
            # Tool execution notification
            tool_name = data.get("name", "unknown")
            self.chat_panel.add_system_message(f"Executing tool: {tool_name}")

        elif msg_type == "tool_result":
            # Tool result
            self.chat_panel.add_system_message("Tool completed")

        elif msg_type == "error":
            # Error message
            error = data.get("message", "Unknown error")
            self.chat_panel.add_error_message(error)

        elif msg_type == "auth_challenge":
            # Authentication required
            self.chat_panel.add_system_message("Authentication required")

    @Slot(bool, str)
    def on_connection_status(self, connected: bool, message: str):
        """Handle connection status change"""
        if connected:
            self.status_bar.showMessage("Connected")
            self.status_label.setText(" Status: Connected ")
            self.status_label.setStyleSheet("color: #4CAF50;")
            self.logs_panel.add_log("SUCCESS", message)
        else:
            self.status_bar.showMessage("Disconnected")
            self.status_label.setText(" Status: Disconnected ")
            self.status_label.setStyleSheet("color: #F44336;")
            self.logs_panel.add_log("WARNING", message)

    @Slot(str)
    def on_ws_error(self, error: str):
        """Handle WebSocket error"""
        self.logs_panel.add_log("ERROR", error)
        self.chat_panel.add_error_message(error)

    @Slot(str)
    def on_file_selected(self, file_path: str):
        """Handle file selection from browser"""
        self.logs_panel.add_log("INFO", f"Selected file: {file_path}")

    def apply_theme(self):
        """Apply dark theme to application"""
        palette = QPalette()
        palette.setColor(QPalette.Window, QColor(53, 53, 53))
        palette.setColor(QPalette.WindowText, Qt.white)
        palette.setColor(QPalette.Base, QColor(35, 35, 35))
        palette.setColor(QPalette.AlternateBase, QColor(53, 53, 53))
        palette.setColor(QPalette.ToolTipBase, QColor(25, 25, 25))
        palette.setColor(QPalette.ToolTipText, Qt.white)
        palette.setColor(QPalette.Text, Qt.white)
        palette.setColor(QPalette.Button, QColor(53, 53, 53))
        palette.setColor(QPalette.ButtonText, Qt.white)
        palette.setColor(QPalette.BrightText, Qt.red)
        palette.setColor(QPalette.Link, QColor(42, 130, 218))
        palette.setColor(QPalette.Highlight, QColor(42, 130, 218))
        palette.setColor(QPalette.HighlightedText, QColor(35, 35, 35))

        self.setPalette(palette)


def main():
    """Main application entry point"""
    app = QApplication(sys.argv)
    app.setApplicationName("RustyClaw GUI")
    app.setOrganizationName("RustyClaw")

    # Create event loop for async support
    loop = QEventLoop(app)
    asyncio.set_event_loop(loop)

    # Create and show main window
    window = MainWindow()
    window.show()

    # Run application
    with loop:
        loop.run_forever()


if __name__ == "__main__":
    main()
