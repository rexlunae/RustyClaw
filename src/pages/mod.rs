pub mod hatching;
pub mod home;

use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;
use crate::panes::PaneState;
use crate::tui::{Event, EventResponse, Frame};

/// A Page is a full-screen layout composed of multiple panes.
/// Mirrors openapi-tui's `Page` trait.
pub trait Page {
    #[allow(unused_variables)]
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        Ok(())
    }

    fn init(&mut self, _state: &PaneState<'_>) -> Result<()> {
        Ok(())
    }

    fn focus(&mut self) -> Result<()> {
        Ok(())
    }

    fn unfocus(&mut self) -> Result<()> {
        Ok(())
    }

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
