use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;
use crate::pages::Page;
use crate::panes::{
    messages::MessagesPane,
    Pane, PaneState,
};
use rustyclaw_core::types::InputMode;
use crate::tui::EventResponse;

pub struct Home {
    command_tx: Option<UnboundedSender<Action>>,
    panes: Vec<Box<dyn Pane>>,
    focused_pane_index: usize,
}

impl Home {
    pub fn new() -> Result<Self> {
        use crate::tui_palette as tp;
        let focused_border_style = tp::focused_border();

        Ok(Self {
            command_tx: None,
            panes: vec![
                Box::new(MessagesPane::new(true, focused_border_style)),
            ],
            focused_pane_index: 0,
        })
    }
}

impl Page for Home {
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        self.command_tx = Some(tx);
        Ok(())
    }

    fn init(&mut self, state: &PaneState<'_>) -> Result<()> {
        for pane in &mut self.panes {
            pane.init(state)?;
        }
        Ok(())
    }

    fn focus(&mut self) -> Result<()> {
        Ok(())
    }

    fn handle_key_events(
        &mut self,
        key: KeyEvent,
        state: &mut PaneState<'_>,
    ) -> Result<Option<EventResponse<Action>>> {
        // When the user is typing in the input bar, don't capture keys here
        if state.input_mode == InputMode::Input {
            return Ok(None);
        }

        match key.code {
            KeyCode::Tab => {
                return Ok(Some(EventResponse::Stop(Action::FocusNext)));
            }
            KeyCode::BackTab => {
                return Ok(Some(EventResponse::Stop(Action::FocusPrev)));
            }
            KeyCode::Char('j') | KeyCode::Down => {
                return Ok(Some(EventResponse::Stop(Action::Down)));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                return Ok(Some(EventResponse::Stop(Action::Up)));
            }
            KeyCode::Enter => {
                return Ok(Some(EventResponse::Stop(Action::Submit)));
            }
            KeyCode::Char('f') => {
                return Ok(Some(EventResponse::Stop(Action::ToggleFullScreen)));
            }
            KeyCode::Char('c') => {
                return Ok(Some(EventResponse::Stop(Action::CopyMessage)));
            }
            _ => {}
        }

        Ok(None)
    }

    fn update(&mut self, action: Action, state: &mut PaneState<'_>) -> Result<Option<Action>> {
        match action {
            Action::FocusNext | Action::FocusPrev | Action::ToggleFullScreen => {
                // Single pane — nothing to cycle or fullscreen.
            }
            Action::Tab(_n) => {
                // Single pane — ignore tab switching.
            }
            Action::Tick => {
                for pane in &mut self.panes {
                    pane.update(Action::Tick, state)?;
                }
            }
            _ => {
                // Forward action to the (only) pane
                if let Some(result) =
                    self.panes[self.focused_pane_index].update(action, state)?
                {
                    return Ok(Some(result));
                }
            }
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, state: &PaneState<'_>) -> Result<()> {
        // Single full-width messages pane
        self.panes[0].draw(frame, area, state)?;
        Ok(())
    }
}
