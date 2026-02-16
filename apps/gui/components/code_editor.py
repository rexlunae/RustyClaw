"""
Code Editor Component with Syntax Highlighting

Provides syntax-highlighted code display and editing capabilities
inspired by AutoGPT's code viewer.
"""

from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QTextEdit,
    QLabel, QComboBox, QPushButton, QFileDialog
)
from PySide6.QtCore import Signal, Qt
from PySide6.QtGui import QFont, QTextCharFormat, QColor, QSyntaxHighlighter
import re


class PythonHighlighter(QSyntaxHighlighter):
    """Python syntax highlighter"""

    def __init__(self, document):
        super().__init__(document)

        # Define syntax highlighting rules
        self.highlighting_rules = []

        # Keywords
        keyword_format = QTextCharFormat()
        keyword_format.setForeground(QColor("#C586C0"))
        keyword_format.setFontWeight(QFont.Bold)
        keywords = [
            'and', 'as', 'assert', 'break', 'class', 'continue', 'def',
            'del', 'elif', 'else', 'except', 'False', 'finally', 'for',
            'from', 'global', 'if', 'import', 'in', 'is', 'lambda', 'None',
            'nonlocal', 'not', 'or', 'pass', 'raise', 'return', 'True',
            'try', 'while', 'with', 'yield', 'async', 'await'
        ]
        for word in keywords:
            pattern = f'\\b{word}\\b'
            self.highlighting_rules.append((re.compile(pattern), keyword_format))

        # Built-in functions
        builtin_format = QTextCharFormat()
        builtin_format.setForeground(QColor("#DCDCAA"))
        builtins = [
            'abs', 'all', 'any', 'bin', 'bool', 'chr', 'dict', 'dir',
            'enumerate', 'filter', 'float', 'int', 'len', 'list', 'map',
            'max', 'min', 'open', 'print', 'range', 'set', 'str', 'sum',
            'tuple', 'type', 'zip'
        ]
        for word in builtins:
            pattern = f'\\b{word}\\b'
            self.highlighting_rules.append((re.compile(pattern), builtin_format))

        # Strings
        string_format = QTextCharFormat()
        string_format.setForeground(QColor("#CE9178"))
        self.highlighting_rules.append((re.compile(r'"[^"\\]*(\\.[^"\\]*)*"'), string_format))
        self.highlighting_rules.append((re.compile(r"'[^'\\]*(\\.[^'\\]*)*'"), string_format))

        # Comments
        comment_format = QTextCharFormat()
        comment_format.setForeground(QColor("#6A9955"))
        comment_format.setFontItalic(True)
        self.highlighting_rules.append((re.compile(r'#[^\n]*'), comment_format))

        # Numbers
        number_format = QTextCharFormat()
        number_format.setForeground(QColor("#B5CEA8"))
        self.highlighting_rules.append((re.compile(r'\b\d+\.?\d*\b'), number_format))

        # Functions/methods
        function_format = QTextCharFormat()
        function_format.setForeground(QColor("#DCDCAA"))
        self.highlighting_rules.append((re.compile(r'\b[A-Za-z_][A-Za-z0-9_]*(?=\()'), function_format))

        # Decorators
        decorator_format = QTextCharFormat()
        decorator_format.setForeground(QColor("#C586C0"))
        self.highlighting_rules.append((re.compile(r'@[A-Za-z_][A-Za-z0-9_]*'), decorator_format))

    def highlightBlock(self, text):
        """Apply syntax highlighting to a block of text"""
        for pattern, fmt in self.highlighting_rules:
            for match in pattern.finditer(text):
                self.setFormat(match.start(), match.end() - match.start(), fmt)


class RustHighlighter(QSyntaxHighlighter):
    """Rust syntax highlighter"""

    def __init__(self, document):
        super().__init__(document)

        self.highlighting_rules = []

        # Keywords
        keyword_format = QTextCharFormat()
        keyword_format.setForeground(QColor("#C586C0"))
        keyword_format.setFontWeight(QFont.Bold)
        keywords = [
            'as', 'break', 'const', 'continue', 'crate', 'else', 'enum',
            'extern', 'false', 'fn', 'for', 'if', 'impl', 'in', 'let',
            'loop', 'match', 'mod', 'move', 'mut', 'pub', 'ref', 'return',
            'self', 'Self', 'static', 'struct', 'super', 'trait', 'true',
            'type', 'unsafe', 'use', 'where', 'while', 'async', 'await',
            'dyn', 'Box', 'Vec', 'String', 'Option', 'Result'
        ]
        for word in keywords:
            pattern = f'\\b{word}\\b'
            self.highlighting_rules.append((re.compile(pattern), keyword_format))

        # Strings
        string_format = QTextCharFormat()
        string_format.setForeground(QColor("#CE9178"))
        self.highlighting_rules.append((re.compile(r'"[^"\\]*(\\.[^"\\]*)*"'), string_format))

        # Comments
        comment_format = QTextCharFormat()
        comment_format.setForeground(QColor("#6A9955"))
        comment_format.setFontItalic(True)
        self.highlighting_rules.append((re.compile(r'//[^\n]*'), comment_format))

        # Macros
        macro_format = QTextCharFormat()
        macro_format.setForeground(QColor("#4EC9B0"))
        self.highlighting_rules.append((re.compile(r'\b[a-z_][a-z0-9_]*!'), macro_format))

        # Numbers
        number_format = QTextCharFormat()
        number_format.setForeground(QColor("#B5CEA8"))
        self.highlighting_rules.append((re.compile(r'\b\d+\.?\d*\b'), number_format))

    def highlightBlock(self, text):
        for pattern, fmt in self.highlighting_rules:
            for match in pattern.finditer(text):
                self.setFormat(match.start(), match.end() - match.start(), fmt)


class CodeEditor(QWidget):
    """
    Code editor with syntax highlighting
    Inspired by AutoGPT's code viewer
    """

    code_changed = Signal(str)
    save_requested = Signal(str, str)  # filename, content

    def __init__(self):
        super().__init__()
        self.current_file = None
        self.highlighter = None
        self.init_ui()

    def init_ui(self):
        layout = QVBoxLayout(self)

        # Toolbar
        toolbar = QHBoxLayout()

        # Language selector
        self.language_combo = QComboBox()
        self.language_combo.addItems(["Python", "Rust", "JavaScript", "JSON", "Plain Text"])
        self.language_combo.currentTextChanged.connect(self.on_language_changed)
        toolbar.addWidget(QLabel("Language:"))
        toolbar.addWidget(self.language_combo)

        toolbar.addStretch()

        # Action buttons
        self.open_button = QPushButton("Open")
        self.open_button.clicked.connect(self.open_file)
        toolbar.addWidget(self.open_button)

        self.save_button = QPushButton("Save")
        self.save_button.clicked.connect(self.save_file)
        toolbar.addWidget(self.save_button)

        self.copy_button = QPushButton("Copy")
        self.copy_button.clicked.connect(self.copy_to_clipboard)
        toolbar.addWidget(self.copy_button)

        layout.addLayout(toolbar)

        # Editor
        self.editor = QTextEdit()
        self.editor.setFont(QFont("Fira Code", 10))
        self.editor.setStyleSheet("""
            QTextEdit {
                background-color: #1E1E1E;
                color: #D4D4D4;
                border: 1px solid #3E3E3E;
                border-radius: 4px;
                padding: 8px;
            }
        """)
        self.editor.textChanged.connect(self.on_text_changed)
        layout.addWidget(self.editor)

        # Status bar
        self.status_label = QLabel("Ready")
        self.status_label.setStyleSheet("color: #858585; font-size: 9pt;")
        layout.addWidget(self.status_label)

        # Set initial highlighter
        self.on_language_changed("Python")

    def on_language_changed(self, language: str):
        """Change syntax highlighter based on language selection"""
        # Remove old highlighter
        if self.highlighter:
            self.highlighter.setDocument(None)

        # Set new highlighter
        if language == "Python":
            self.highlighter = PythonHighlighter(self.editor.document())
        elif language == "Rust":
            self.highlighter = RustHighlighter(self.editor.document())
        else:
            self.highlighter = None  # Plain text

        self.status_label.setText(f"Language: {language}")

    def set_code(self, code: str, language: str = "Python"):
        """Set code content and language"""
        self.editor.setPlainText(code)

        # Set language
        index = self.language_combo.findText(language)
        if index >= 0:
            self.language_combo.setCurrentIndex(index)

    def get_code(self) -> str:
        """Get current code content"""
        return self.editor.toPlainText()

    def on_text_changed(self):
        """Handle text change"""
        self.code_changed.emit(self.get_code())

    def open_file(self):
        """Open a file for editing"""
        filename, _ = QFileDialog.getOpenFileName(
            self,
            "Open File",
            "",
            "All Files (*);;Python Files (*.py);;Rust Files (*.rs)"
        )

        if filename:
            try:
                with open(filename, 'r', encoding='utf-8') as f:
                    content = f.read()
                self.editor.setPlainText(content)
                self.current_file = filename
                self.status_label.setText(f"Opened: {filename}")

                # Auto-detect language
                if filename.endswith('.py'):
                    self.language_combo.setCurrentText("Python")
                elif filename.endswith('.rs'):
                    self.language_combo.setCurrentText("Rust")
                elif filename.endswith('.js'):
                    self.language_combo.setCurrentText("JavaScript")
            except Exception as e:
                self.status_label.setText(f"Error: {e}")

    def save_file(self):
        """Save current content"""
        if not self.current_file:
            filename, _ = QFileDialog.getSaveFileName(
                self,
                "Save File",
                "",
                "All Files (*)"
            )
            if filename:
                self.current_file = filename

        if self.current_file:
            content = self.get_code()
            self.save_requested.emit(self.current_file, content)

            try:
                with open(self.current_file, 'w', encoding='utf-8') as f:
                    f.write(content)
                self.status_label.setText(f"Saved: {self.current_file}")
            except Exception as e:
                self.status_label.setText(f"Error saving: {e}")

    def copy_to_clipboard(self):
        """Copy code to clipboard"""
        from PySide6.QtWidgets import QApplication
        clipboard = QApplication.clipboard()
        clipboard.setText(self.get_code())
        self.status_label.setText("Copied to clipboard")
