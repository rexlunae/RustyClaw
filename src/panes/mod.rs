pub mod config_pane;
pub mod footer;
pub mod header;
pub mod messages;
pub mod secrets_pane;
pub mod skills_pane;

use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::{Constraint, Rect};

use crate::action::Action;
use crate::tui::{Event, EventResponse, Frame};

/// Connection status of the gateway WebSocket.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayStatus {
    /// No gateway URL configured
    #[default]
    Unconfigured,
    /// Not connected / connection lost
    Disconnected,
    /// Connection attempt in progress
    Connecting,
    /// Successfully connected to the gateway
    Connected,
    /// Gateway has validated the model connection and is ready for chat
    ModelReady,
    /// Gateway reported a model/credential error
    ModelError,
    /// Connection attempt failed
    Error,
    /// Gateway vault is locked — password needed
    VaultLocked,
    /// Gateway requires TOTP authentication
    AuthRequired,
}

impl GatewayStatus {
    pub fn label(self) -> &'static str {
        match self {
            GatewayStatus::Unconfigured => "no gateway",
            GatewayStatus::Disconnected => "disconnected",
            GatewayStatus::Connecting => "connecting…",
            GatewayStatus::Connected => "connected",
            GatewayStatus::ModelReady => "model ready",
            GatewayStatus::ModelError => "model error",
            GatewayStatus::Error => "error",
            GatewayStatus::VaultLocked => "vault locked 🔒",
            GatewayStatus::AuthRequired => "auth required 🔑",
        }
    }
}

// ── Message types ───────────────────────────────────────────────────────────

/// Role / category of a chat-pane message.
///
/// Determines the icon and colour used when rendering the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    /// User-submitted prompt (▶)
    User,
    /// Model / assistant reply (◀)
    Assistant,
    /// Neutral informational (ℹ)
    Info,
    /// Positive confirmation (✅)
    Success,
    /// Non-critical warning (⚠)
    Warning,
    /// Hard error (❌)
    Error,
    /// Generic system status (📡)
    System,
    /// The model is invoking a tool (🔧)
    ToolCall,
    /// Result of a tool invocation (📎)
    ToolResult,
}

impl MessageRole {
    /// Leading icon character for display.
    pub fn icon(self) -> &'static str {
        match self {
            Self::User => "▶",
            Self::Assistant => "◀",
            Self::Info => "ℹ",
            Self::Success => "✅",
            Self::Warning => "⚠",
            Self::Error => "❌",
            Self::System => "📡",
            Self::ToolCall => "🔧",
            Self::ToolResult => "📎",
        }
    }
}

/// A single message in the chat / log pane.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: MessageRole,
    pub content: String,
    /// Cached styled lines (lazily computed on first access).
    /// This avoids re-parsing markdown on every frame.
    cached_lines: std::cell::OnceCell<Vec<ratatui::text::Line<'static>>>,
}

impl DisplayMessage {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self { role, content: content.into(), cached_lines: std::cell::OnceCell::new() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(MessageRole::User, content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Assistant, content)
    }
    pub fn info(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Info, content)
    }
    pub fn success(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Success, content)
    }
    pub fn warning(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Warning, content)
    }
    pub fn error(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Error, content)
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(MessageRole::System, content)
    }
    pub fn tool_call(content: impl Into<String>) -> Self {
        Self::new(MessageRole::ToolCall, content)
    }
    pub fn tool_result(content: impl Into<String>) -> Self {
        Self::new(MessageRole::ToolResult, content)
    }

    /// Update the content and invalidate the cache.
    /// Used during streaming when content is appended.
    pub fn update_content(&mut self, new_content: String) {
        self.content = new_content;
        self.cached_lines = std::cell::OnceCell::new();
    }

    /// Get cached lines, computing them if needed.
    pub fn get_lines(&self, tab_width: usize) -> &Vec<ratatui::text::Line<'static>> {
        self.cached_lines.get_or_init(|| Self::build_lines_cached(&self.role, &self.content, tab_width))
    }

    /// Build styled lines for this message (internal implementation).
    fn build_lines_cached(role: &MessageRole, content: &str, tab_width: usize) -> Vec<ratatui::text::Line<'static>> {
        use ratatui::text::{Line, Span};
        use ratatui::style::{Style, Color};
        use tui_markdown::{from_str_with_options, Options};
        use crate::theme::tui_palette::{self as tp, RustyClawMarkdownStyle};

        // Expand tab characters to spaces
        let content: String = if content.contains('\t') {
            content.replace('\t', &" ".repeat(tab_width))
        } else {
            content.to_string()
        };

        let color = match role {
            MessageRole::User => tp::ACCENT_BRIGHT,
            MessageRole::Assistant => tp::TEXT,
            MessageRole::Info => tp::INFO,
            MessageRole::Success => tp::SUCCESS,
            MessageRole::Warning => tp::WARN,
            MessageRole::Error => tp::ERROR,
            MessageRole::System => tp::MUTED,
            MessageRole::ToolCall => tp::MUTED,
            MessageRole::ToolResult => tp::TEXT_DIM,
        };

        let is_assistant = matches!(role, MessageRole::Assistant);

        if !is_assistant {
            // Non-assistant messages stay single-line (no markdown processing).
            let mut spans: Vec<Span<'static>> = Vec::new();
            spans.push(Span::raw(" "));
            let icon_fn = || match role {
                MessageRole::User => "▶",
                MessageRole::Assistant => "◀",
                MessageRole::Info => "ℹ",
                MessageRole::Success => "✅",
                MessageRole::Warning => "⚠",
                MessageRole::Error => "❌",
                MessageRole::System => "📡",
                MessageRole::ToolCall => "🔧",
                MessageRole::ToolResult => "📎",
            };
            if matches!(role, MessageRole::User | MessageRole::Assistant | MessageRole::Info | 
                       MessageRole::Success | MessageRole::Warning | MessageRole::Error | 
                       MessageRole::System | MessageRole::ToolCall | MessageRole::ToolResult) {
                let icon = icon_fn();
                spans.push(Span::styled(
                    format!("{icon} "),
                    Style::default().fg(color),
                ));
            }
            spans.push(Span::styled(content, Style::default().fg(color)));
            return vec![Line::from(spans)];
        }

        // Assistant: use tui-markdown for full CommonMark rendering
        let options = Options::new(RustyClawMarkdownStyle);
        let text = from_str_with_options(&content, &options);

        // Add a leading space to each line for padding
        text.lines
            .into_iter()
            .map(|line| {
                let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
                for span in line.spans {
                    spans.push(Span::styled(
                        span.content.into_owned(),
                        span.style,
                    ));
                }
                Line::from(spans)
            })
            .collect()
    }
}

/// Shared state passed to every pane during update and draw.
pub struct PaneState<'a> {
    pub config: &'a crate::config::Config,
    pub secrets_manager: &'a mut crate::secrets::SecretsManager,
    pub skill_manager: &'a mut crate::skills::SkillManager,
    pub soul_manager: &'a crate::soul::SoulManager,
    pub messages: &'a mut Vec<DisplayMessage>,
    pub input_mode: InputMode,
    pub gateway_status: GatewayStatus,
    /// Animated loading line shown at the bottom of the messages list.
    pub loading_line: Option<String>,
    /// When streaming started (for elapsed time display in footer).
    pub streaming_started: Option<std::time::Instant>,
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum InputMode {
    /// Pane navigation keys are active (input bar is empty / not typing)
    #[default]
    Normal,
    /// User is typing in the input bar
    Input,
}

/// A focusable, drawable pane — mirrors openapi-tui's `Pane` trait.
pub trait Pane {
    fn init(&mut self, _state: &PaneState<'_>) -> Result<()> {
        Ok(())
    }

    fn height_constraint(&self) -> Constraint;

    fn handle_events(
        &mut self,
        event: Event,
        state: &mut PaneState<'_>,
    ) -> Result<Option<EventResponse<Action>>> {
        let r = match event {
            Event::Key(key_event) => self.handle_key_events(key_event, state)?,
            Event::Mouse(mouse_event) => self.handle_mouse_events(mouse_event, state)?,
            _ => None,
        };
        Ok(r)
    }

    #[allow(unused_variables)]
    fn handle_key_events(
        &mut self,
        key: KeyEvent,
        state: &mut PaneState<'_>,
    ) -> Result<Option<EventResponse<Action>>> {
        Ok(None)
    }

    #[allow(unused_variables)]
    fn handle_mouse_events(
        &mut self,
        mouse: MouseEvent,
        state: &mut PaneState<'_>,
    ) -> Result<Option<EventResponse<Action>>> {
        Ok(None)
    }

    #[allow(unused_variables)]
    fn update(&mut self, action: Action, state: &mut PaneState<'_>) -> Result<Option<Action>> {
        Ok(None)
    }

    fn draw(&mut self, f: &mut Frame<'_>, area: Rect, state: &PaneState<'_>) -> Result<()>;
}
