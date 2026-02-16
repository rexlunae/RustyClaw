"""
Settings and Preferences Dialog

Allows users to configure GUI and gateway connection settings,
inspired by AutoGPT's settings panel.
"""

from PySide6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout, QLabel,
    QLineEdit, QSpinBox, QCheckBox, QPushButton,
    QTabWidget, QWidget, QGroupBox, QComboBox,
    QFileDialog
)
from PySide6.QtCore import Signal
from typing import Dict, Any
import json
from pathlib import Path


class SettingsDialog(QDialog):
    """Settings and preferences dialog"""

    settings_changed = Signal(dict)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.settings = self.load_settings()
        self.init_ui()

    def init_ui(self):
        self.setWindowTitle("Settings")
        self.setMinimumSize(600, 500)

        layout = QVBoxLayout(self)

        # Tab widget
        tabs = QTabWidget()

        # Connection tab
        conn_tab = self.create_connection_tab()
        tabs.addTab(conn_tab, "Connection")

        # Appearance tab
        appearance_tab = self.create_appearance_tab()
        tabs.addTab(appearance_tab, "Appearance")

        # Behavior tab
        behavior_tab = self.create_behavior_tab()
        tabs.addTab(behavior_tab, "Behavior")

        # Advanced tab
        advanced_tab = self.create_advanced_tab()
        tabs.addTab(advanced_tab, "Advanced")

        layout.addWidget(tabs)

        # Buttons
        button_layout = QHBoxLayout()
        button_layout.addStretch()

        self.reset_button = QPushButton("Reset to Defaults")
        self.reset_button.clicked.connect(self.reset_settings)
        button_layout.addWidget(self.reset_button)

        self.cancel_button = QPushButton("Cancel")
        self.cancel_button.clicked.connect(self.reject)
        button_layout.addWidget(self.cancel_button)

        self.save_button = QPushButton("Save")
        self.save_button.clicked.connect(self.save_settings)
        self.save_button.setDefault(True)
        button_layout.addWidget(self.save_button)

        layout.addLayout(button_layout)

    def create_connection_tab(self) -> QWidget:
        """Create connection settings tab"""
        tab = QWidget()
        layout = QVBoxLayout(tab)

        # Gateway connection
        gateway_group = QGroupBox("Gateway Connection")
        gateway_layout = QVBoxLayout(gateway_group)

        # WebSocket URL
        url_layout = QHBoxLayout()
        url_layout.addWidget(QLabel("WebSocket URL:"))
        self.ws_url_input = QLineEdit()
        self.ws_url_input.setText(self.settings.get("gateway_url", "ws://localhost:8080"))
        self.ws_url_input.setPlaceholderText("ws://localhost:8080 or wss://...")
        url_layout.addWidget(self.ws_url_input)
        gateway_layout.addLayout(url_layout)

        # Auto-connect
        self.auto_connect_check = QCheckBox("Auto-connect on startup")
        self.auto_connect_check.setChecked(self.settings.get("auto_connect", False))
        gateway_layout.addWidget(self.auto_connect_check)

        # Reconnect
        reconnect_layout = QHBoxLayout()
        self.auto_reconnect_check = QCheckBox("Auto-reconnect on disconnect")
        self.auto_reconnect_check.setChecked(self.settings.get("auto_reconnect", True))
        reconnect_layout.addWidget(self.auto_reconnect_check)

        reconnect_layout.addWidget(QLabel("Retry interval:"))
        self.reconnect_interval = QSpinBox()
        self.reconnect_interval.setRange(1, 60)
        self.reconnect_interval.setValue(self.settings.get("reconnect_interval", 5))
        self.reconnect_interval.setSuffix(" seconds")
        reconnect_layout.addWidget(self.reconnect_interval)
        gateway_layout.addLayout(reconnect_layout)

        layout.addWidget(gateway_group)

        # Authentication
        auth_group = QGroupBox("Authentication")
        auth_layout = QVBoxLayout(auth_group)

        self.save_credentials_check = QCheckBox("Save credentials securely")
        self.save_credentials_check.setChecked(self.settings.get("save_credentials", False))
        auth_layout.addWidget(self.save_credentials_check)

        layout.addWidget(auth_group)

        layout.addStretch()

        return tab

    def create_appearance_tab(self) -> QWidget:
        """Create appearance settings tab"""
        tab = QWidget()
        layout = QVBoxLayout(tab)

        # Theme
        theme_group = QGroupBox("Theme")
        theme_layout = QVBoxLayout(theme_group)

        theme_select = QHBoxLayout()
        theme_select.addWidget(QLabel("Color scheme:"))
        self.theme_combo = QComboBox()
        self.theme_combo.addItems(["Dark (Default)", "Light", "Auto (System)"])
        current_theme = self.settings.get("theme", "Dark (Default)")
        self.theme_combo.setCurrentText(current_theme)
        theme_select.addWidget(self.theme_combo)
        theme_layout.addLayout(theme_select)

        layout.addWidget(theme_group)

        # Font
        font_group = QGroupBox("Font")
        font_layout = QVBoxLayout(font_group)

        # Font family
        font_family_layout = QHBoxLayout()
        font_family_layout.addWidget(QLabel("Font family:"))
        self.font_combo = QComboBox()
        self.font_combo.addItems([
            "Fira Code", "Source Code Pro", "JetBrains Mono",
            "Courier New", "Consolas", "System Default"
        ])
        current_font = self.settings.get("font_family", "Fira Code")
        self.font_combo.setCurrentText(current_font)
        font_family_layout.addWidget(self.font_combo)
        font_layout.addLayout(font_family_layout)

        # Font size
        font_size_layout = QHBoxLayout()
        font_size_layout.addWidget(QLabel("Font size:"))
        self.font_size_spin = QSpinBox()
        self.font_size_spin.setRange(8, 20)
        self.font_size_spin.setValue(self.settings.get("font_size", 10))
        font_size_layout.addWidget(self.font_size_spin)
        font_layout.addLayout(font_size_layout)

        layout.addWidget(font_group)

        # Chat display
        chat_group = QGroupBox("Chat Display")
        chat_layout = QVBoxLayout(chat_group)

        self.show_timestamps_check = QCheckBox("Show timestamps")
        self.show_timestamps_check.setChecked(self.settings.get("show_timestamps", True))
        chat_layout.addWidget(self.show_timestamps_check)

        self.show_avatars_check = QCheckBox("Show user/agent avatars")
        self.show_avatars_check.setChecked(self.settings.get("show_avatars", True))
        chat_layout.addWidget(self.show_avatars_check)

        layout.addWidget(chat_group)

        layout.addStretch()

        return tab

    def create_behavior_tab(self) -> QWidget:
        """Create behavior settings tab"""
        tab = QWidget()
        layout = QVBoxLayout(tab)

        # Chat behavior
        chat_group = QGroupBox("Chat Behavior")
        chat_layout = QVBoxLayout(chat_group)

        self.auto_scroll_check = QCheckBox("Auto-scroll to latest message")
        self.auto_scroll_check.setChecked(self.settings.get("auto_scroll", True))
        chat_layout.addWidget(self.auto_scroll_check)

        self.sound_notifications_check = QCheckBox("Sound notifications")
        self.sound_notifications_check.setChecked(self.settings.get("sound_notifications", False))
        chat_layout.addWidget(self.sound_notifications_check)

        send_on_layout = QHBoxLayout()
        send_on_layout.addWidget(QLabel("Send message with:"))
        self.send_key_combo = QComboBox()
        self.send_key_combo.addItems(["Enter", "Ctrl+Enter", "Shift+Enter"])
        current_send_key = self.settings.get("send_key", "Enter")
        self.send_key_combo.setCurrentText(current_send_key)
        send_on_layout.addWidget(self.send_key_combo)
        chat_layout.addLayout(send_on_layout)

        layout.addWidget(chat_group)

        # File browser
        file_group = QGroupBox("File Browser")
        file_layout = QVBoxLayout(file_group)

        workspace_layout = QHBoxLayout()
        workspace_layout.addWidget(QLabel("Workspace directory:"))
        self.workspace_input = QLineEdit()
        self.workspace_input.setText(self.settings.get("workspace_dir", "."))
        workspace_layout.addWidget(self.workspace_input)

        browse_btn = QPushButton("Browse...")
        browse_btn.clicked.connect(self.browse_workspace)
        workspace_layout.addWidget(browse_btn)
        file_layout.addLayout(workspace_layout)

        self.show_hidden_files_check = QCheckBox("Show hidden files")
        self.show_hidden_files_check.setChecked(self.settings.get("show_hidden_files", False))
        file_layout.addWidget(self.show_hidden_files_check)

        layout.addWidget(file_group)

        layout.addStretch()

        return tab

    def create_advanced_tab(self) -> QWidget:
        """Create advanced settings tab"""
        tab = QWidget()
        layout = QVBoxLayout(tab)

        # Performance
        perf_group = QGroupBox("Performance")
        perf_layout = QVBoxLayout(perf_group)

        max_messages_layout = QHBoxLayout()
        max_messages_layout.addWidget(QLabel("Max chat messages to display:"))
        self.max_messages_spin = QSpinBox()
        self.max_messages_spin.setRange(50, 1000)
        self.max_messages_spin.setValue(self.settings.get("max_messages", 200))
        max_messages_layout.addWidget(self.max_messages_spin)
        perf_layout.addLayout(max_messages_layout)

        log_level_layout = QHBoxLayout()
        log_level_layout.addWidget(QLabel("Log level:"))
        self.log_level_combo = QComboBox()
        self.log_level_combo.addItems(["DEBUG", "INFO", "WARNING", "ERROR"])
        current_log_level = self.settings.get("log_level", "INFO")
        self.log_level_combo.setCurrentText(current_log_level)
        log_level_layout.addWidget(self.log_level_combo)
        perf_layout.addLayout(log_level_layout)

        layout.addWidget(perf_group)

        # Data
        data_group = QGroupBox("Data")
        data_layout = QVBoxLayout(data_group)

        self.save_chat_history_check = QCheckBox("Save chat history")
        self.save_chat_history_check.setChecked(self.settings.get("save_chat_history", True))
        data_layout.addWidget(self.save_chat_history_check)

        export_btn = QPushButton("Export Settings...")
        export_btn.clicked.connect(self.export_settings)
        data_layout.addWidget(export_btn)

        import_btn = QPushButton("Import Settings...")
        import_btn.clicked.connect(self.import_settings)
        data_layout.addWidget(import_btn)

        layout.addWidget(data_group)

        layout.addStretch()

        return tab

    def browse_workspace(self):
        """Browse for workspace directory"""
        directory = QFileDialog.getExistingDirectory(
            self,
            "Select Workspace Directory",
            self.workspace_input.text()
        )
        if directory:
            self.workspace_input.setText(directory)

    def load_settings(self) -> Dict[str, Any]:
        """Load settings from file"""
        settings_file = Path.home() / ".rustyclaw" / "gui_settings.json"

        if settings_file.exists():
            try:
                with open(settings_file, 'r') as f:
                    return json.load(f)
            except Exception:
                pass

        # Default settings
        return {
            "gateway_url": "ws://localhost:8080",
            "auto_connect": False,
            "auto_reconnect": True,
            "reconnect_interval": 5,
            "save_credentials": False,
            "theme": "Dark (Default)",
            "font_family": "Fira Code",
            "font_size": 10,
            "show_timestamps": True,
            "show_avatars": True,
            "auto_scroll": True,
            "sound_notifications": False,
            "send_key": "Enter",
            "workspace_dir": ".",
            "show_hidden_files": False,
            "max_messages": 200,
            "log_level": "INFO",
            "save_chat_history": True
        }

    def save_settings(self):
        """Save settings to file"""
        # Collect settings from UI
        self.settings = {
            "gateway_url": self.ws_url_input.text(),
            "auto_connect": self.auto_connect_check.isChecked(),
            "auto_reconnect": self.auto_reconnect_check.isChecked(),
            "reconnect_interval": self.reconnect_interval.value(),
            "save_credentials": self.save_credentials_check.isChecked(),
            "theme": self.theme_combo.currentText(),
            "font_family": self.font_combo.currentText(),
            "font_size": self.font_size_spin.value(),
            "show_timestamps": self.show_timestamps_check.isChecked(),
            "show_avatars": self.show_avatars_check.isChecked(),
            "auto_scroll": self.auto_scroll_check.isChecked(),
            "sound_notifications": self.sound_notifications_check.isChecked(),
            "send_key": self.send_key_combo.currentText(),
            "workspace_dir": self.workspace_input.text(),
            "show_hidden_files": self.show_hidden_files_check.isChecked(),
            "max_messages": self.max_messages_spin.value(),
            "log_level": self.log_level_combo.currentText(),
            "save_chat_history": self.save_chat_history_check.isChecked()
        }

        # Save to file
        settings_file = Path.home() / ".rustyclaw" / "gui_settings.json"
        settings_file.parent.mkdir(parents=True, exist_ok=True)

        try:
            with open(settings_file, 'w') as f:
                json.dump(self.settings, f, indent=2)

            self.settings_changed.emit(self.settings)
            self.accept()
        except Exception as e:
            print(f"Error saving settings: {e}")

    def reset_settings(self):
        """Reset settings to defaults"""
        self.settings = self.load_settings()
        # Update UI with default values
        self.ws_url_input.setText(self.settings["gateway_url"])
        self.auto_connect_check.setChecked(self.settings["auto_connect"])
        # ... update other fields ...

    def export_settings(self):
        """Export settings to file"""
        filename, _ = QFileDialog.getSaveFileName(
            self,
            "Export Settings",
            "rustyclaw_settings.json",
            "JSON Files (*.json)"
        )

        if filename:
            try:
                with open(filename, 'w') as f:
                    json.dump(self.settings, f, indent=2)
            except Exception as e:
                print(f"Error exporting settings: {e}")

    def import_settings(self):
        """Import settings from file"""
        filename, _ = QFileDialog.getOpenFileName(
            self,
            "Import Settings",
            "",
            "JSON Files (*.json)"
        )

        if filename:
            try:
                with open(filename, 'r') as f:
                    imported = json.load(f)
                    self.settings.update(imported)
                    # Update UI
                    # ... refresh UI with imported settings ...
            except Exception as e:
                print(f"Error importing settings: {e}")
