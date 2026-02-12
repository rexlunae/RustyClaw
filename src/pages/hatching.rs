//! Hatching page - displays an animated SOUL "hatching" sequence on first run.
//!
//! When RustyClaw is first launched and no SOUL.md exists (or it's the default),
//! this page shows an animated "egg hatching" sequence representing the agent
//! coming to life with its personality.

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
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// Complete - ready to transition
    Complete,
}

pub struct Hatching {
    /// Current animation state
    state: HatchState,
    /// Animation tick counter
    tick: usize,
    /// How many ticks before advancing to next state
    ticks_per_state: usize,
    /// Whether the animation is complete
    complete: bool,
}

impl Hatching {
    pub fn new() -> Result<Self> {
        Ok(Self {
            state: HatchState::Egg,
            tick: 0,
            ticks_per_state: 8, // ~2 seconds at 4 ticks/sec
            complete: false,
        })
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Advance the animation
    fn advance(&mut self) {
        self.tick += 1;
        
        if self.tick >= self.ticks_per_state {
            self.tick = 0;
            self.state = match self.state {
                HatchState::Egg => HatchState::Crack1,
                HatchState::Crack1 => HatchState::Crack2,
                HatchState::Crack2 => HatchState::Breaking,
                HatchState::Breaking => HatchState::Hatched,
                HatchState::Hatched => HatchState::Complete,
                HatchState::Complete => {
                    self.complete = true;
                    HatchState::Complete
                }
            };
        }
    }

    /// Get the ASCII art for the current state
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
            HatchState::Complete => vec![
                "                                                    ",
                "                                                    ",
                "                                                    ",
                "                                                    ",
                "                   ✨  ✨  ✨                       ",
                "                                                    ",
                "                  🦀  SOUL.md  🦀                   ",
                "                                                    ",
                "               Your agent is alive!                 ",
                "                                                    ",
                "                   ✨  ✨  ✨                       ",
                "                                                    ",
                "            Press any key to continue...            ",
                "                                                    ",
            ],
        }
    }

    /// Get the message for the current state
    fn get_message(&self) -> &'static str {
        match self.state {
            HatchState::Egg => "A mysterious egg...",
            HatchState::Crack1 => "Something is stirring inside...",
            HatchState::Crack2 => "The shell begins to crack...",
            HatchState::Breaking => "Breaking free...",
            HatchState::Hatched => "Your RustyClaw agent emerges!",
            HatchState::Complete => "Initialization complete!",
        }
    }
}

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
        // Any key skips the animation if it's complete
        if self.state == HatchState::Complete {
            match key.code {
                KeyCode::Char(_) | KeyCode::Enter | KeyCode::Esc => {
                    self.complete = true;
                    return Ok(Some(EventResponse::Stop(Action::CloseHatching)));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn update(&mut self, action: Action, _state: &mut PaneState<'_>) -> Result<Option<Action>> {
        match action {
            Action::Tick => {
                if !self.complete {
                    self.advance();
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn draw(&mut self, f: &mut Frame<'_>, area: Rect, _state: &PaneState<'_>) -> Result<()> {
        // Create a centered block for the animation
        let block = Block::default()
            .title("🦀 RustyClaw - Hatching 🦀")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::new().fg(tp::ACCENT))
            .style(Style::new().bg(tp::SURFACE));

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Split into art area and message area
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(14), // Art
                Constraint::Length(3),  // Message
                Constraint::Min(0),     // Spacer
            ])
            .split(inner);

        // Render the ASCII art
        let art = self.get_art();
        let art_text = art.join("\n");
        let art_paragraph = Paragraph::new(art_text)
            .style(Style::new().fg(tp::ACCENT_BRIGHT))
            .alignment(Alignment::Center);
        f.render_widget(art_paragraph, chunks[0]);

        // Render the message
        let message = self.get_message();
        let message_paragraph = Paragraph::new(message)
            .style(Style::new().fg(tp::INFO))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        f.render_widget(message_paragraph, chunks[1]);

        Ok(())
    }
}
