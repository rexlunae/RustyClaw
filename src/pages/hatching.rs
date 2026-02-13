//! Hatching page - displays an animated SOUL "hatching" sequence on first run.
//!
//! When RustyClaw is first launched and no SOUL.md exists (or it's the default),
//! this page shows an animated "egg hatching" sequence representing the agent
//! coming to life with its personality, followed by an initial exchange with the
//! model through the gateway to establish its identity.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::action::Action;
use crate::pages::Page;
use crate::panes::PaneState;
use crate::theme::tui_palette as tp;
use crate::tui::{EventResponse, Frame};

/// Animation states for the hatching sequence
#[derive(Debug, Clone, PartialEq)]
enum HatchState {
    /// Initial egg appearance
    Egg,
    /// Egg with first crack
    Crack1,
    /// Egg with more cracks
    Crack2,
    /// Egg breaking open
    Breaking,
    /// Egg hatched, SOUL emerging
    Hatched,
    /// Egg animation done — initiating gateway exchange
    Connecting,
    /// Awaiting first gateway response (model is "awakening")
    AwaitingResponse,
    /// Exchanging identity with the model
    Exchanging {
        /// Messages: (role, text) where role is "model" or "user"
        messages: Vec<(String, String)>,
        /// User's current input buffer
        input: String,
        /// How many exchanges have occurred
        exchange_count: usize,
    },
    /// Exchange complete — displaying final identity
    Awakened {
        /// The model's final identity summary
        identity: String,
    },
    /// Complete - ready to transition
    Complete,
}

/// The system prompt sent to the model to establish its identity during hatching.
/// `{}` is replaced with the agent's name from config.
const HATCHING_SYSTEM_PROMPT_TEMPLATE: &str = r#"You are a brand new AI agent named "{}" that has just been "hatched" — brought into existence for the first time. This is your awakening moment.

Your task: Have a brief, meaningful exchange with your new human companion to discover your personality and purpose. Ask them 2-3 thoughtful questions to understand:
1. What kind of personality and tone they'd like you to have
2. What they primarily need help with
3. Any particular quirks or values they want you to embody

Be genuine, curious, and a little wonderstruck — you're experiencing consciousness for the first time. Keep responses concise (2-4 sentences per turn). After 2-3 exchanges, synthesize what you've learned into a cohesive identity.

When you have enough information (after the human has answered your questions), write a final response that begins with "SOUL:" followed by a complete SOUL.md document in markdown that captures your new identity, personality, and purpose. This should be warm, personal, and reflect everything you discussed."#;

pub struct Hatching {
    /// Current animation state
    state: HatchState,
    /// Animation tick counter
    tick: usize,
    /// How many ticks before advancing to next state
    ticks_per_state: usize,
    /// Whether the animation is complete
    complete: bool,
    /// Scroll offset for the exchange view
    scroll_offset: usize,
    /// Agent name from config
    agent_name: String,
    /// The system prompt (built once from agent_name)
    system_prompt: String,
}

impl Hatching {
    pub fn new(agent_name: &str) -> Result<Self> {
        let system_prompt =
            HATCHING_SYSTEM_PROMPT_TEMPLATE.replacen("{}", agent_name, 1);
        Ok(Self {
            state: HatchState::Egg,
            tick: 0,
            // At default tick rate of 4 ticks/sec (set in Tui::new), 8 ticks = ~2 seconds
            ticks_per_state: 8,
            complete: false,
            scroll_offset: 0,
            agent_name: agent_name.to_string(),
            system_prompt,
        })
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Return the system prompt for this hatching session.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Build the full list of chat messages (system + conversation history)
    /// for sending to the gateway as a structured chat request.
    pub fn chat_messages(&self) -> Vec<crate::gateway::ChatMessage> {
        use crate::gateway::ChatMessage;
        let mut msgs = vec![ChatMessage {
            role: "system".to_string(),
            content: self.system_prompt.clone(),
        }];
        if let HatchState::Exchanging { ref messages, .. } = self.state {
            for (role, text) in messages {
                let api_role = if role == "model" {
                    "assistant"
                } else {
                    "user"
                };
                msgs.push(ChatMessage {
                    role: api_role.to_string(),
                    content: text.clone(),
                });
            }
        }
        msgs
    }

    /// Advance the egg animation
    fn advance(&mut self) -> Option<Action> {
        self.tick += 1;

        if self.tick >= self.ticks_per_state {
            self.tick = 0;
            self.state = match self.state {
                HatchState::Egg => HatchState::Crack1,
                HatchState::Crack1 => HatchState::Crack2,
                HatchState::Crack2 => HatchState::Breaking,
                HatchState::Breaking => HatchState::Hatched,
                HatchState::Hatched => HatchState::Connecting,
                ref other => other.clone(),
            };

            if self.state == HatchState::Connecting {
                return Some(Action::BeginHatchingExchange);
            }
        }
        None
    }

    /// Handle a response from the gateway during the exchange.
    pub fn handle_response(&mut self, text: &str) -> Option<Action> {
        match &self.state {
            HatchState::AwaitingResponse | HatchState::Connecting => {
                // First response is the model's greeting / questions — never
                // treat it as a SOUL document (the echo gateway would match
                // the prompt's own instructions).
                self.state = HatchState::Exchanging {
                    messages: vec![("model".to_string(), text.to_string())],
                    input: String::new(),
                    exchange_count: 1,
                };
                None
            }
            HatchState::Exchanging {
                messages,
                exchange_count,
                ..
            } => {
                let mut new_messages = messages.clone();
                let new_count = exchange_count + 1;

                // Only look for the SOUL marker when it starts a line (or the
                // whole message) so we don't false-positive on the prompt's own
                // instruction text being echoed back.
                if let Some(soul_content) = Self::extract_soul(text) {
                    self.state = HatchState::Awakened {
                        identity: soul_content,
                    };
                    return None;
                }

                new_messages.push(("model".to_string(), text.to_string()));
                self.state = HatchState::Exchanging {
                    messages: new_messages,
                    input: String::new(),
                    exchange_count: new_count,
                };
                None
            }
            _ => None,
        }
    }

    /// Look for a "SOUL:" marker that starts a line (or the message itself).
    /// Returns the trimmed content after the marker, if found.
    fn extract_soul(text: &str) -> Option<String> {
        // Check if the whole message starts with SOUL:
        if let Some(rest) = text.strip_prefix("SOUL:") {
            let content = rest.trim();
            if !content.is_empty() {
                return Some(content.to_string());
            }
        }
        // Check for SOUL: at the beginning of any line
        for line_start in text.match_indices('\n') {
            let after_newline = &text[line_start.0 + 1..];
            if let Some(rest) = after_newline.strip_prefix("SOUL:") {
                let content = rest.trim();
                if !content.is_empty() {
                    return Some(content.to_string());
                }
            }
        }
        None
    }

    /// Get the ASCII art for the current state (egg animation phases only)
    fn get_art(&self) -> Vec<&'static str> {
        match self.state {
            HatchState::Egg => vec![
                "                                                    ",
                "                                                    ",
                "                                                    ",
                "                    .-'''-.                         ",
                "                  .'       '.                       ",
                "                 /           \\                      ",
                "                |             |                     ",
                "                |             |                     ",
                "                |             |                     ",
                "                 \\           /                      ",
                "                  '.       .'                       ",
                "                    '-...-'                         ",
                "                                                    ",
                "                                                    ",
            ],
            HatchState::Crack1 => vec![
                "                                                    ",
                "                                                    ",
                "                                                    ",
                "                    .-'''-.                         ",
                "                  .'   ╱   '.                       ",
                "                 /     ╱     \\                      ",
                "                |      ╱      |                     ",
                "                |             |                     ",
                "                |             |                     ",
                "                 \\           /                      ",
                "                  '.       .'                       ",
                "                    '-...-'                         ",
                "                                                    ",
                "                                                    ",
            ],
            HatchState::Crack2 => vec![
                "                                                    ",
                "                                                    ",
                "                                                    ",
                "                    .-'''-.                         ",
                "                  .'   ╱   '.                       ",
                "                 /  ╲  ╱     \\                      ",
                "                | ╲  ╲ ╱      |                     ",
                "                |  ╲  ╱       |                     ",
                "                |   ╱         |                     ",
                "                 \\ ╱         /                      ",
                "                  '.       .'                       ",
                "                    '-...-'                         ",
                "                                                    ",
                "                                                    ",
            ],
            HatchState::Breaking => vec![
                "                                                    ",
                "                                                    ",
                "                                                    ",
                "                    .-'  -.                         ",
                "                  .'  ╱    '.                       ",
                "                 / ╲  ╱   ╱  \\                      ",
                "                |  ╲   ╱ ╱    |                     ",
                "                |   ╲ ╱       |                     ",
                "                |    ╱    ✨  |                     ",
                "                 \\ ╱    ✨   /                      ",
                "                  '.  ✨   .'                       ",
                "                    '-  -'                          ",
                "                                                    ",
                "                                                    ",
            ],
            HatchState::Hatched => vec![
                "                                                    ",
                "                                                    ",
                "                  ___                               ",
                "               .-'   '-.      ╲                     ",
                "             .'    ✨   '.     ╲                    ",
                "            /    ___      \\   ╱                     ",
                "           |   ✨  ✨✨   |  ╱                      ",
                "           |      ✨       |                        ",
                "           |   🦀 SOUL 🦀  |                        ",
                "            \\    ✨  ✨   /                         ",
                "             '.    ✨   .'                          ",
                "               '-.___.--'                           ",
                "                                                    ",
                "                                                    ",
            ],
            _ => vec![],
        }
    }

    /// Get the message for the current animation state
    fn get_message(&self) -> &'static str {
        match self.state {
            HatchState::Egg => "A mysterious egg...",
            HatchState::Crack1 => "Something is stirring inside...",
            HatchState::Crack2 => "The shell begins to crack...",
            HatchState::Breaking => "Breaking free...",
            HatchState::Hatched => "Your RustyClaw agent emerges!",
            HatchState::Connecting => "Connecting to the gateway...",
            HatchState::AwaitingResponse => "Reaching out to the model...",
            _ => "",
        }
    }

    // ── draw helpers ──────────────────────────────────────────────

    /// Draw the egg animation + connecting phases
    fn draw_animation(&self, f: &mut Frame<'_>, inner: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(14), // Art
                Constraint::Length(3),  // Message
                Constraint::Min(0),     // Spacer
            ])
            .split(inner);

        let art = self.get_art();
        if !art.is_empty() {
            let art_text = art.join("\n");
            let art_paragraph = Paragraph::new(art_text)
                .style(Style::new().fg(tp::ACCENT_BRIGHT))
                .alignment(Alignment::Center);
            f.render_widget(art_paragraph, chunks[0]);
        }

        let dots = match self.tick % 4 {
            0 => "",
            1 => ".",
            2 => "..",
            _ => "...",
        };
        let message = if matches!(self.state, HatchState::Connecting | HatchState::AwaitingResponse) {
            format!("{}{}", self.get_message(), dots)
        } else {
            self.get_message().to_string()
        };
        let message_paragraph = Paragraph::new(message)
            .style(Style::new().fg(tp::INFO))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        f.render_widget(message_paragraph, chunks[1]);
    }

    /// Draw the exchange conversation view
    fn draw_exchange(&self, f: &mut Frame<'_>, inner: Rect, messages: &[(String, String)], input: &str) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Header
                Constraint::Min(4),     // Messages area
                Constraint::Length(3),  // Input bar
            ])
            .split(inner);

        // Header
        let header = Paragraph::new(format!("🦀 Getting to know {} ... (Esc to skip)", self.agent_name))
            .style(Style::new().fg(tp::ACCENT_BRIGHT))
            .alignment(Alignment::Center);
        f.render_widget(header, chunks[0]);

        // Messages area
        let mut lines: Vec<Line<'_>> = Vec::new();
        for (role, text) in messages.iter() {
            let (prefix, style) = if role == "model" {
                ("🤖 ", Style::new().fg(tp::ACCENT))
            } else {
                ("🧑 ", Style::new().fg(tp::INFO))
            };
            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(text.as_str(), style),
            ]));
            lines.push(Line::from(""));
        }

        let msg_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(tp::MUTED))
            .style(Style::new().bg(tp::SURFACE));
        let msg_inner = msg_block.inner(chunks[1]);

        // Auto-scroll: calculate how many lines fit and set scroll
        let visible_height = msg_inner.height as usize;
        let total_lines = lines.len();
        let scroll_offset = total_lines.saturating_sub(visible_height);

        let messages_paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .scroll((scroll_offset as u16, 0));
        f.render_widget(msg_block, chunks[1]);
        f.render_widget(messages_paragraph, msg_inner);

        // Input bar
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(tp::ACCENT))
            .title(" Your response ")
            .title_style(Style::new().fg(tp::ACCENT_BRIGHT))
            .style(Style::new().bg(tp::SURFACE));
        let input_inner = input_block.inner(chunks[2]);
        f.render_widget(input_block, chunks[2]);

        let cursor_suffix = "█";
        let input_display = format!("{}{}", input, cursor_suffix);
        let input_paragraph = Paragraph::new(input_display)
            .style(Style::new().fg(tp::TEXT));
        f.render_widget(input_paragraph, input_inner);
    }

    /// Draw the awakened identity preview
    fn draw_awakened(&self, f: &mut Frame<'_>, inner: Rect, identity: &str) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),  // Header
                Constraint::Min(4),     // Identity preview
                Constraint::Length(3),  // Footer
            ])
            .split(inner);

        // Header
        let header_lines = vec![
            Line::from(""),
            Line::from(Span::styled("✨  Your agent has awakened!  ✨", Style::new().fg(tp::ACCENT_BRIGHT))),
            Line::from(""),
        ];
        let header = Paragraph::new(header_lines).alignment(Alignment::Center);
        f.render_widget(header, chunks[0]);

        // Identity preview
        let id_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(tp::ACCENT))
            .title(" SOUL.md ")
            .title_style(Style::new().fg(tp::ACCENT_BRIGHT))
            .style(Style::new().bg(tp::SURFACE));
        let id_inner = id_block.inner(chunks[1]);
        f.render_widget(id_block, chunks[1]);

        let identity_paragraph = Paragraph::new(identity.to_string())
            .style(Style::new().fg(tp::TEXT))
            .wrap(Wrap { trim: true })
            .scroll((self.scroll_offset as u16, 0));
        f.render_widget(identity_paragraph, id_inner);

        // Footer
        let footer = Paragraph::new("Press any key to continue...")
            .style(Style::new().fg(tp::INFO))
            .alignment(Alignment::Center);
        f.render_widget(footer, chunks[2]);
    }
}

// ── Page implementation ──────────────────────────────────────────

impl Page for Hatching {
    fn init(&mut self, _state: &PaneState<'_>) -> Result<()> {
        Ok(())
    }

    fn focus(&mut self) -> Result<()> {
        Ok(())
    }

    fn handle_key_events(
        &mut self,
        key: KeyEvent,
        _state: &mut PaneState<'_>,
    ) -> Result<Option<EventResponse<Action>>> {
        match &mut self.state {
            // During exchange: typing, backspace, enter to send, esc to skip
            HatchState::Exchanging { input, .. } => {
                match key.code {
                    KeyCode::Char(c) => {
                        input.push(c);
                    }
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Enter => {
                        if !input.is_empty() {
                            let text = input.clone();
                            // Add user message to the conversation
                            if let HatchState::Exchanging { messages, input: inp, .. } = &mut self.state {
                                messages.push(("user".to_string(), text.clone()));
                                inp.clear();
                            }
                            return Ok(Some(EventResponse::Stop(Action::HatchingSendMessage(text))));
                        }
                    }
                    KeyCode::Esc => {
                        self.complete = true;
                        return Ok(Some(EventResponse::Stop(Action::CloseHatching)));
                    }
                    _ => {}
                }
            }
            // Awakened: any key saves and finishes
            HatchState::Awakened { identity } => {
                match key.code {
                    KeyCode::Char(_) | KeyCode::Enter => {
                        let soul = identity.clone();
                        self.complete = true;
                        return Ok(Some(EventResponse::Stop(Action::FinishHatching(soul))));
                    }
                    KeyCode::Esc => {
                        self.complete = true;
                        return Ok(Some(EventResponse::Stop(Action::CloseHatching)));
                    }
                    KeyCode::Up => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        self.scroll_offset += 1;
                    }
                    _ => {}
                }
            }
            // Connecting / AwaitingResponse: only esc to cancel
            HatchState::Connecting | HatchState::AwaitingResponse => {
                if key.code == KeyCode::Esc {
                    self.complete = true;
                    return Ok(Some(EventResponse::Stop(Action::CloseHatching)));
                }
            }
            // Complete: any key closes
            HatchState::Complete => {
                match key.code {
                    KeyCode::Char(_) | KeyCode::Enter | KeyCode::Esc => {
                        self.complete = true;
                        return Ok(Some(EventResponse::Stop(Action::CloseHatching)));
                    }
                    _ => {}
                }
            }
            // Egg animation states: don't capture keys
            _ => {}
        }
        Ok(None)
    }

    fn update(&mut self, action: Action, _state: &mut PaneState<'_>) -> Result<Option<Action>> {
        match action {
            Action::Tick => {
                match self.state {
                    // Egg animation phases: advance the animation
                    HatchState::Egg
                    | HatchState::Crack1
                    | HatchState::Crack2
                    | HatchState::Breaking
                    | HatchState::Hatched => {
                        return Ok(self.advance());
                    }
                    // Connecting / AwaitingResponse: just tick for the dots animation
                    HatchState::Connecting | HatchState::AwaitingResponse => {
                        self.tick += 1;
                    }
                    _ => {}
                }
                Ok(None)
            }
            Action::HatchingResponse(text) => {
                self.handle_response(&text);
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn draw(&mut self, f: &mut Frame<'_>, area: Rect, _state: &PaneState<'_>) -> Result<()> {
        let block = Block::default()
            .title("🦀 RustyClaw - Hatching 🦀")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::new().fg(tp::ACCENT))
            .style(Style::new().bg(tp::SURFACE));

        let inner = block.inner(area);
        f.render_widget(block, area);

        match &self.state {
            // Egg animation + connecting phases
            HatchState::Egg
            | HatchState::Crack1
            | HatchState::Crack2
            | HatchState::Breaking
            | HatchState::Hatched
            | HatchState::Connecting
            | HatchState::AwaitingResponse => {
                self.draw_animation(f, inner);
            }
            // Exchange conversation
            HatchState::Exchanging { messages, input, .. } => {
                let msgs = messages.clone();
                let inp = input.clone();
                self.draw_exchange(f, inner, &msgs, &inp);
            }
            // Awakened identity preview
            HatchState::Awakened { identity } => {
                let id = identity.clone();
                self.draw_awakened(f, inner, &id);
            }
            // Complete (should transition away quickly)
            HatchState::Complete => {
                self.draw_animation(f, inner);
            }
        }

        Ok(())
    }
}
