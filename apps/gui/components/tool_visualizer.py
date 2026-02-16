"""
Tool Execution Visualizer

Real-time visualization of tool calls and execution flow,
inspired by AutoGPT's execution trace viewer.
"""

from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QListWidget, QListWidgetItem, QFrame, QPushButton,
    QTextEdit
)
from PySide6.QtCore import Signal, Qt, QTimer
from PySide6.QtGui import QColor, QFont
from datetime import datetime
from typing import Optional, Dict, Any


class ToolCallItem(QFrame):
    """Individual tool call display item"""

    clicked = Signal(str)  # tool_call_id

    def __init__(self, tool_call_id: str, tool_name: str, status: str = "pending"):
        super().__init__()
        self.tool_call_id = tool_call_id
        self.tool_name = tool_name
        self.status = status
        self.start_time = datetime.now()
        self.init_ui()

    def init_ui(self):
        self.setFrameStyle(QFrame.Box | QFrame.Raised)
        self.setStyleSheet("""
            ToolCallItem {
                background-color: #2D2D2D;
                border-left: 4px solid #007ACC;
                border-radius: 4px;
                padding: 8px;
                margin: 4px;
            }
            ToolCallItem:hover {
                background-color: #3E3E3E;
            }
        """)
        self.setCursor(Qt.PointingHandCursor)

        layout = QVBoxLayout(self)

        # Header
        header = QHBoxLayout()

        # Status icon
        self.status_icon = QLabel()
        self.update_status(self.status)
        header.addWidget(self.status_icon)

        # Tool name
        tool_label = QLabel(f"<b>{self.tool_name}</b>")
        tool_label.setStyleSheet("font-size: 11pt; color: #FFFFFF;")
        header.addWidget(tool_label)

        header.addStretch()

        # Timestamp
        timestamp = self.start_time.strftime("%H:%M:%S")
        time_label = QLabel(timestamp)
        time_label.setStyleSheet("font-size: 9pt; color: #858585;")
        header.addWidget(time_label)

        layout.addLayout(header)

        # Details (initially hidden)
        self.details_widget = QWidget()
        self.details_layout = QVBoxLayout(self.details_widget)
        self.details_widget.setVisible(False)

        self.args_label = QLabel()
        self.args_label.setStyleSheet("font-size: 9pt; color: #D4D4D4;")
        self.args_label.setWordWrap(True)
        self.details_layout.addWidget(self.args_label)

        self.result_label = QLabel()
        self.result_label.setStyleSheet("font-size: 9pt; color: #4CAF50;")
        self.result_label.setWordWrap(True)
        self.details_layout.addWidget(self.result_label)

        layout.addWidget(self.details_widget)

    def update_status(self, status: str):
        """Update tool call status"""
        self.status = status

        status_info = {
            "pending": ("⏳", "#FFA500"),
            "running": ("⚙️", "#2196F3"),
            "completed": ("✅", "#4CAF50"),
            "error": ("❌", "#F44336")
        }

        icon, color = status_info.get(status, ("❓", "#858585"))
        self.status_icon.setText(icon)
        self.status_icon.setStyleSheet(f"font-size: 14pt; color: {color};")

        # Update border color
        border_colors = {
            "pending": "#FFA500",
            "running": "#2196F3",
            "completed": "#4CAF50",
            "error": "#F44336"
        }
        border_color = border_colors.get(status, "#007ACC")
        self.setStyleSheet(f"""
            ToolCallItem {{
                background-color: #2D2D2D;
                border-left: 4px solid {border_color};
                border-radius: 4px;
                padding: 8px;
                margin: 4px;
            }}
            ToolCallItem:hover {{
                background-color: #3E3E3E;
            }}
        """)

    def set_arguments(self, args: Dict[str, Any]):
        """Set tool call arguments"""
        args_str = ", ".join([f"{k}={v}" for k, v in args.items()])
        self.args_label.setText(f"Args: {args_str}")

    def set_result(self, result: str):
        """Set tool call result"""
        if len(result) > 100:
            result = result[:100] + "..."
        self.result_label.setText(f"Result: {result}")

    def toggle_details(self):
        """Toggle details visibility"""
        self.details_widget.setVisible(not self.details_widget.isVisible())

    def mousePressEvent(self, event):
        """Handle click"""
        self.toggle_details()
        self.clicked.emit(self.tool_call_id)
        super().mousePressEvent(event)


class ToolVisualizer(QWidget):
    """
    Tool execution visualizer panel
    Inspired by AutoGPT's execution flow view
    """

    tool_selected = Signal(str)  # tool_call_id

    def __init__(self):
        super().__init__()
        self.tool_calls: Dict[str, ToolCallItem] = {}
        self.init_ui()

    def init_ui(self):
        layout = QVBoxLayout(self)

        # Header
        header = QHBoxLayout()
        title = QLabel("<h2>🔧 Tool Execution</h2>")
        header.addWidget(title)
        header.addStretch()

        # Clear button
        self.clear_btn = QPushButton("Clear")
        self.clear_btn.clicked.connect(self.clear_all)
        header.addWidget(self.clear_btn)

        layout.addLayout(header)

        # Tool call list
        self.tool_list = QListWidget()
        self.tool_list.setStyleSheet("""
            QListWidget {
                background-color: #1E1E1E;
                border: none;
            }
        """)
        self.tool_list.setSpacing(2)
        layout.addWidget(self.tool_list)

        # Statistics
        stats_layout = QHBoxLayout()

        self.total_label = QLabel("Total: 0")
        self.success_label = QLabel("Success: 0")
        self.error_label = QLabel("Errors: 0")
        self.pending_label = QLabel("Pending: 0")

        for label in [self.total_label, self.success_label, self.error_label, self.pending_label]:
            label.setStyleSheet("font-size: 9pt; color: #D4D4D4; padding: 4px;")
            stats_layout.addWidget(label)

        stats_layout.addStretch()

        layout.addLayout(stats_layout)

    def add_tool_call(
        self,
        tool_call_id: str,
        tool_name: str,
        args: Optional[Dict[str, Any]] = None
    ):
        """Add a new tool call"""
        if tool_call_id in self.tool_calls:
            return  # Already exists

        item_widget = ToolCallItem(tool_call_id, tool_name, "pending")
        item_widget.clicked.connect(self.tool_selected.emit)

        if args:
            item_widget.set_arguments(args)

        self.tool_calls[tool_call_id] = item_widget

        # Add to list
        list_item = QListWidgetItem(self.tool_list)
        list_item.setSizeHint(item_widget.sizeHint())
        self.tool_list.setItemWidget(list_item, item_widget)

        # Scroll to bottom
        self.tool_list.scrollToBottom()

        self.update_statistics()

    def update_tool_status(self, tool_call_id: str, status: str, result: Optional[str] = None):
        """Update tool call status"""
        if tool_call_id in self.tool_calls:
            item = self.tool_calls[tool_call_id]
            item.update_status(status)

            if result and status == "completed":
                item.set_result(result)

            self.update_statistics()

    def clear_all(self):
        """Clear all tool calls"""
        self.tool_list.clear()
        self.tool_calls.clear()
        self.update_statistics()

    def update_statistics(self):
        """Update statistics display"""
        total = len(self.tool_calls)
        success = sum(1 for t in self.tool_calls.values() if t.status == "completed")
        error = sum(1 for t in self.tool_calls.values() if t.status == "error")
        pending = sum(1 for t in self.tool_calls.values() if t.status in ["pending", "running"])

        self.total_label.setText(f"Total: {total}")
        self.success_label.setText(f"Success: {success}")
        self.success_label.setStyleSheet("font-size: 9pt; color: #4CAF50; padding: 4px;")
        self.error_label.setText(f"Errors: {error}")
        self.error_label.setStyleSheet("font-size: 9pt; color: #F44336; padding: 4px;")
        self.pending_label.setText(f"Pending: {pending}")
        self.pending_label.setStyleSheet("font-size: 9pt; color: #FFA500; padding: 4px;")
