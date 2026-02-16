"""
Metrics and Monitoring Panel

Displays real-time metrics and performance data from RustyClaw gateway,
inspired by AutoGPT's monitoring dashboard.
"""

from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QGroupBox, QProgressBar, QFrame
)
from PySide6.QtCore import Signal, QTimer, Qt
from PySide6.QtGui import QFont
from typing import Dict


class MetricCard(QFrame):
    """Individual metric display card"""

    def __init__(self, title: str, value: str = "0", unit: str = ""):
        super().__init__()
        self.init_ui(title, value, unit)

    def init_ui(self, title: str, value: str, unit: str):
        self.setFrameStyle(QFrame.Box | QFrame.Plain)
        self.setStyleSheet("""
            MetricCard {
                background-color: #2D2D2D;
                border: 1px solid #3E3E3E;
                border-radius: 6px;
                padding: 12px;
            }
        """)

        layout = QVBoxLayout(self)

        # Title
        title_label = QLabel(title)
        title_label.setStyleSheet("font-size: 10pt; color: #858585;")
        layout.addWidget(title_label)

        # Value
        self.value_label = QLabel(f"<b>{value}</b>{unit}")
        self.value_label.setStyleSheet("font-size: 24pt; color: #4CAF50;")
        layout.addWidget(self.value_label)

    def update_value(self, value: str, unit: str = ""):
        """Update metric value"""
        self.value_label.setText(f"<b>{value}</b>{unit}")


class MetricsPanel(QWidget):
    """
    Metrics and performance monitoring panel
    Inspired by AutoGPT's dashboard
    """

    def __init__(self):
        super().__init__()
        self.metrics: Dict[str, MetricCard] = {}
        self.init_ui()

        # Auto-refresh timer
        self.refresh_timer = QTimer()
        self.refresh_timer.timeout.connect(self.refresh_metrics)
        self.refresh_timer.start(2000)  # Refresh every 2 seconds

    def init_ui(self):
        layout = QVBoxLayout(self)

        # Title
        title = QLabel("<h2>📊 System Metrics</h2>")
        layout.addWidget(title)

        # Connection metrics
        conn_group = QGroupBox("Gateway Status")
        conn_layout = QVBoxLayout(conn_group)

        conn_metrics = QHBoxLayout()
        self.metrics["connections"] = MetricCard("Active Connections", "0")
        self.metrics["uptime"] = MetricCard("Uptime", "0", "s")
        conn_metrics.addWidget(self.metrics["connections"])
        conn_metrics.addWidget(self.metrics["uptime"])
        conn_layout.addLayout(conn_metrics)

        layout.addWidget(conn_group)

        # Request metrics
        req_group = QGroupBox("Request Statistics")
        req_layout = QVBoxLayout(req_group)

        req_row1 = QHBoxLayout()
        self.metrics["total_requests"] = MetricCard("Total Requests", "0")
        self.metrics["success_rate"] = MetricCard("Success Rate", "0", "%")
        req_row1.addWidget(self.metrics["total_requests"])
        req_row1.addWidget(self.metrics["success_rate"])
        req_layout.addLayout(req_row1)

        req_row2 = QHBoxLayout()
        self.metrics["avg_latency"] = MetricCard("Avg Latency", "0", "ms")
        self.metrics["requests_per_min"] = MetricCard("Requests/min", "0")
        req_row2.addWidget(self.metrics["avg_latency"])
        req_row2.addWidget(self.metrics["requests_per_min"])
        req_layout.addLayout(req_row2)

        layout.addWidget(req_group)

        # Tool metrics
        tool_group = QGroupBox("Tool Execution")
        tool_layout = QVBoxLayout(tool_group)

        tool_row = QHBoxLayout()
        self.metrics["tool_calls"] = MetricCard("Tool Calls", "0")
        self.metrics["tool_errors"] = MetricCard("Tool Errors", "0")
        tool_row.addWidget(self.metrics["tool_calls"])
        tool_row.addWidget(self.metrics["tool_errors"])
        tool_layout.addLayout(tool_row)

        layout.addWidget(tool_group)

        # Token metrics
        token_group = QGroupBox("Token Usage")
        token_layout = QVBoxLayout(token_group)

        token_row = QHBoxLayout()
        self.metrics["input_tokens"] = MetricCard("Input Tokens", "0")
        self.metrics["output_tokens"] = MetricCard("Output Tokens", "0")
        token_row.addWidget(self.metrics["input_tokens"])
        token_row.addWidget(self.metrics["output_tokens"])
        token_layout.addLayout(token_row)

        layout.addWidget(token_group)

        # Security metrics
        security_group = QGroupBox("Security Events")
        security_layout = QVBoxLayout(security_group)

        security_row = QHBoxLayout()
        self.metrics["ssrf_blocked"] = MetricCard("SSRF Blocked", "0")
        self.metrics["injection_detected"] = MetricCard("Injections Detected", "0")
        security_row.addWidget(self.metrics["ssrf_blocked"])
        security_row.addWidget(self.metrics["injection_detected"])
        security_layout.addLayout(security_row)

        layout.addWidget(security_group)

        layout.addStretch()

    def update_metric(self, name: str, value: str, unit: str = ""):
        """Update a specific metric"""
        if name in self.metrics:
            self.metrics[name].update_value(value, unit)

    def refresh_metrics(self):
        """Refresh all metrics from gateway"""
        # This would fetch metrics from the Prometheus endpoint
        # For now, we'll just update with placeholder logic
        pass

    def set_connection_count(self, count: int):
        """Update active connection count"""
        self.update_metric("connections", str(count))

    def set_uptime(self, seconds: int):
        """Update uptime"""
        if seconds < 60:
            self.update_metric("uptime", str(seconds), "s")
        elif seconds < 3600:
            self.update_metric("uptime", f"{seconds // 60}", "m")
        else:
            self.update_metric("uptime", f"{seconds // 3600}", "h")

    def increment_requests(self):
        """Increment total request counter"""
        current = int(self.metrics["total_requests"].value_label.text().replace("<b>", "").replace("</b>", ""))
        self.update_metric("total_requests", str(current + 1))

    def set_success_rate(self, rate: float):
        """Set success rate percentage"""
        self.update_metric("success_rate", f"{rate:.1f}", "%")

    def add_tool_call(self, success: bool):
        """Record a tool call"""
        # Increment tool_calls
        current_calls = int(self.metrics["tool_calls"].value_label.text().split("<")[1].split(">")[1])
        self.update_metric("tool_calls", str(current_calls + 1))

        # Increment errors if failed
        if not success:
            current_errors = int(self.metrics["tool_errors"].value_label.text().split("<")[1].split(">")[1])
            self.update_metric("tool_errors", str(current_errors + 1))

    def add_tokens(self, input_tokens: int, output_tokens: int):
        """Add token usage"""
        # Input tokens
        current_input = int(self.metrics["input_tokens"].value_label.text().split("<")[1].split(">")[1])
        self.update_metric("input_tokens", str(current_input + input_tokens))

        # Output tokens
        current_output = int(self.metrics["output_tokens"].value_label.text().split("<")[1].split(">")[1])
        self.update_metric("output_tokens", str(current_output + output_tokens))

    def record_security_event(self, event_type: str):
        """Record a security event"""
        if event_type == "ssrf":
            current = int(self.metrics["ssrf_blocked"].value_label.text().split("<")[1].split(">")[1])
            self.update_metric("ssrf_blocked", str(current + 1))
        elif event_type == "injection":
            current = int(self.metrics["injection_detected"].value_label.text().split("<")[1].split(">")[1])
            self.update_metric("injection_detected", str(current + 1))
