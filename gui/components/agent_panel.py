"""
Multi-Agent Coordination Panel

Displays and manages multiple AI agents simultaneously,
inspired by AutoGPT's multi-agent orchestration view.
"""

from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QPushButton, QListWidget, QListWidgetItem, QGroupBox,
    QProgressBar, QFrame
)
from PySide6.QtCore import Signal, Qt
from PySide6.QtGui import QColor, QFont
from typing import Dict, Optional


class AgentCard(QFrame):
    """Individual agent status card"""

    agent_clicked = Signal(str)  # agent_id

    def __init__(self, agent_id: str, name: str, status: str = "idle"):
        super().__init__()
        self.agent_id = agent_id
        self.status = status
        self.init_ui(name)

    def init_ui(self, name: str):
        self.setFrameStyle(QFrame.Box | QFrame.Raised)
        self.setLineWidth(2)
        self.setStyleSheet("""
            AgentCard {
                background-color: #2D2D2D;
                border: 2px solid #3E3E3E;
                border-radius: 8px;
                padding: 12px;
            }
            AgentCard:hover {
                border: 2px solid #007ACC;
            }
        """)
        self.setCursor(Qt.PointingHandCursor)

        layout = QVBoxLayout(self)

        # Agent name
        self.name_label = QLabel(f"<b>{name}</b>")
        self.name_label.setStyleSheet("font-size: 14pt; color: #FFFFFF;")
        layout.addWidget(self.name_label)

        # Agent ID
        id_label = QLabel(f"ID: {self.agent_id}")
        id_label.setStyleSheet("font-size: 9pt; color: #858585;")
        layout.addWidget(id_label)

        # Status indicator
        self.status_label = QLabel()
        self.update_status(self.status)
        layout.addWidget(self.status_label)

        # Progress bar
        self.progress = QProgressBar()
        self.progress.setMaximum(100)
        self.progress.setValue(0)
        self.progress.setTextVisible(False)
        self.progress.setStyleSheet("""
            QProgressBar {
                border: 1px solid #3E3E3E;
                border-radius: 3px;
                background-color: #1E1E1E;
                height: 8px;
            }
            QProgressBar::chunk {
                background-color: #007ACC;
                border-radius: 3px;
            }
        """)
        layout.addWidget(self.progress)

        # Metrics
        self.metrics_label = QLabel("Tasks: 0 | Messages: 0")
        self.metrics_label.setStyleSheet("font-size: 9pt; color: #858585;")
        layout.addWidget(self.metrics_label)

    def update_status(self, status: str):
        """Update agent status"""
        self.status = status

        status_icons = {
            "idle": ("⚪", "#858585"),
            "thinking": ("🤔", "#FFA500"),
            "working": ("⚙️", "#4CAF50"),
            "waiting": ("⏸️", "#2196F3"),
            "error": ("❌", "#F44336"),
            "completed": ("✅", "#4CAF50")
        }

        icon, color = status_icons.get(status, ("❓", "#858585"))
        self.status_label.setText(f"{icon} <b>{status.capitalize()}</b>")
        self.status_label.setStyleSheet(f"font-size: 11pt; color: {color};")

    def set_progress(self, value: int):
        """Set progress bar value (0-100)"""
        self.progress.setValue(value)

    def update_metrics(self, tasks: int, messages: int):
        """Update agent metrics"""
        self.metrics_label.setText(f"Tasks: {tasks} | Messages: {messages}")

    def mousePressEvent(self, event):
        """Handle click on agent card"""
        self.agent_clicked.emit(self.agent_id)
        super().mousePressEvent(event)


class AgentPanel(QWidget):
    """
    Multi-agent coordination dashboard
    Inspired by AutoGPT's agent orchestration
    """

    agent_selected = Signal(str)  # agent_id
    create_agent = Signal(str)    # agent_type

    def __init__(self):
        super().__init__()
        self.agents: Dict[str, AgentCard] = {}
        self.init_ui()

    def init_ui(self):
        layout = QVBoxLayout(self)

        # Header
        header = QHBoxLayout()
        title = QLabel("<h2>🤖 Active Agents</h2>")
        header.addWidget(title)
        header.addStretch()

        # Create agent button
        self.create_btn = QPushButton("➕ New Agent")
        self.create_btn.clicked.connect(self.on_create_agent)
        self.create_btn.setStyleSheet("""
            QPushButton {
                background-color: #007ACC;
                color: white;
                border: none;
                border-radius: 4px;
                padding: 8px 16px;
                font-weight: bold;
            }
            QPushButton:hover {
                background-color: #005A9E;
            }
        """)
        header.addWidget(self.create_btn)

        layout.addLayout(header)

        # Agent grid container
        self.agent_container = QVBoxLayout()
        layout.addLayout(self.agent_container)

        layout.addStretch()

        # Summary section
        summary_group = QGroupBox("Summary")
        summary_layout = QVBoxLayout(summary_group)

        self.total_label = QLabel("Total Agents: 0")
        self.active_label = QLabel("Active: 0")
        self.idle_label = QLabel("Idle: 0")

        for label in [self.total_label, self.active_label, self.idle_label]:
            label.setStyleSheet("font-size: 10pt; color: #D4D4D4;")
            summary_layout.addWidget(label)

        layout.addWidget(summary_group)

    def add_agent(self, agent_id: str, name: str, status: str = "idle"):
        """Add a new agent to the panel"""
        if agent_id in self.agents:
            return  # Already exists

        card = AgentCard(agent_id, name, status)
        card.agent_clicked.connect(self.agent_selected.emit)

        self.agents[agent_id] = card
        self.agent_container.addWidget(card)

        self.update_summary()

    def remove_agent(self, agent_id: str):
        """Remove an agent from the panel"""
        if agent_id in self.agents:
            card = self.agents[agent_id]
            self.agent_container.removeWidget(card)
            card.deleteLater()
            del self.agents[agent_id]

            self.update_summary()

    def update_agent_status(self, agent_id: str, status: str):
        """Update agent status"""
        if agent_id in self.agents:
            self.agents[agent_id].update_status(status)
            self.update_summary()

    def update_agent_progress(self, agent_id: str, progress: int):
        """Update agent progress"""
        if agent_id in self.agents:
            self.agents[agent_id].set_progress(progress)

    def update_agent_metrics(self, agent_id: str, tasks: int, messages: int):
        """Update agent metrics"""
        if agent_id in self.agents:
            self.agents[agent_id].update_metrics(tasks, messages)

    def update_summary(self):
        """Update summary statistics"""
        total = len(self.agents)
        active = sum(1 for a in self.agents.values() if a.status in ["thinking", "working"])
        idle = sum(1 for a in self.agents.values() if a.status == "idle")

        self.total_label.setText(f"Total Agents: {total}")
        self.active_label.setText(f"Active: {active}")
        self.idle_label.setText(f"Idle: {idle}")

    def on_create_agent(self):
        """Handle create agent button click"""
        self.create_agent.emit("default")
