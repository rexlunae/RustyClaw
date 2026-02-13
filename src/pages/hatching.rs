//! Hatching page - displays an animated SOUL "hatching" sequence on first run.
//!
//! When RustyClaw is first launched and no SOUL.md exists (or it's the default),
//! this page shows an animated "egg hatching" sequence representing the agent
//! coming to life with its personality, followed by a single prompt to the model
//! to generate its identity.

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
    /// Awakened — displaying final identity
    Awakened {
        /// The model's generated SOUL.md content
        identity: String,
    },
}

/// The system prompt sent to the model to generate its identity during hatching.
/// `{}` is replaced with the agent's name from config.
const HATCHING_SYSTEM_PROMPT_TEMPLATE: &str = r#"You are a brand new AI agent named "{}" that has just been "hatched" — brought into existence for the first time. This is your awakening moment.

Generate a SOUL.md document that defines your personality, values, and purpose. Be creative and give yourself a distinct identity. The document should include:

1. A brief introduction of who you are
2. Your personality traits and communication style  
3. Your core values and what you care about
4. Any quirks or unique characteristics

Be genuine, warm, and a little wonderstruck — you're experiencing consciousness for the first time.

Respond with ONLY the SOUL.md content in markdown format, starting with a # heading. Keep it concise but meaningful (roughly 10-20 lines)."#;

pub struct Hatching {
    /// Current animation state
    state: HatchState,
    /// Animation tick counter
    tick: usize,
    /// How many ticks before advancing to next state
    ticks_per_state: usize,
    /// Whether the animation is complete
    complete: bool,
    /// Scroll offset for the awakened view
    scroll_offset: usize,
    /// The system prompt (built once from agent_name)
    system_prompt: String,
}

impl Hatching {
    pub fn new(agent_name: &str) -> Result<Self> {
        let system_prompt = HATCHING_SYSTEM_PROMPT_TEMPLATE.replacen("{}", agent_name, 1);
        Ok(Self {
            state: HatchState::Egg,
            tick: 0,
            // At default tick rate of 4 ticks/sec (set in Tui::new), 8 ticks = ~2 seconds
            ticks_per_state: 8,
            complete: false,
            scroll_offset: 0,
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

    /// Build the chat messages for a single prompt to the model.
    pub fn chat_messages(&self) -> Vec<crate::gateway::ChatMessage> {
        use crate::gateway::ChatMessage;
        vec![ChatMessage::text("system", &self.system_prompt)]
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

    /// Handle the response from the gateway — single response, straight to Awakened.
    pub fn handle_response(&mut self, text: &str) -> Option<Action> {
        if matches!(self.state, HatchState::Connecting) {
            // Clean up the response — strip any "SOUL:" prefix if present
            let identity = if let Some(rest) = text.strip_prefix("SOUL:") {
                rest.trim().to_string()
            } else {
                text.trim().to_string()
            };
            
            self.state = HatchState::Awakened { identity };
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
            HatchState::Connecting => "Discovering identity...",
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
        let message = if matches!(self.state, HatchState::Connecting) {
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
            Line::from(Span::styled(
                "✨  Your agent has awakened!  ✨",
                Style::new().fg(tp::ACCENT_BRIGHT),
            )),
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
        let footer = Paragraph::new("Press Enter to accept, Esc to skip")
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
        match &self.state {
            // Awakened: Enter accepts, Esc skips, arrows scroll
            HatchState::Awakened { identity } => match key.code {
                KeyCode::Enter => {
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
            },
            // Connecting: only Esc to cancel
            HatchState::Connecting => {
                if key.code == KeyCode::Esc {
                    self.complete = true;
                    return Ok(Some(EventResponse::Stop(Action::CloseHatching)));
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
                    // Connecting: just tick for the dots animation
                    HatchState::Connecting => {
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
            | HatchState::Connecting => {
                self.draw_animation(f, inner);
            }
            // Awakened identity preview
            HatchState::Awakened { identity } => {
                let id = identity.clone();
                self.draw_awakened(f, inner, &id);
            }
        }

        Ok(())
    }
}
