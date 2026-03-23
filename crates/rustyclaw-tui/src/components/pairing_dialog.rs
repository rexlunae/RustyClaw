//! Pairing dialog for TUI.
//!
//! This component handles the SSH key pairing flow:
//! 1. Display the client's public key for copy/paste
//! 2. Show QR code (if enabled)
//! 3. Input gateway host:port
//! 4. Connect and verify

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// State for the pairing dialog.
#[derive(Debug, Clone)]
pub struct PairingDialog {
    /// Current step in the pairing flow.
    pub step: PairingStep,
    
    /// The client's public key in OpenSSH format.
    pub public_key: String,
    
    /// The key fingerprint.
    pub fingerprint: String,
    
    /// The key fingerprint art (visual hash).
    pub fingerprint_art: String,
    
    /// QR code ASCII art (if available).
    pub qr_ascii: Option<String>,
    
    /// Gateway host input.
    pub gateway_host: String,
    
    /// Gateway port input.
    pub gateway_port: String,
    
    /// Cursor position in the active input field.
    pub cursor_pos: usize,
    
    /// Which input field is active.
    pub active_field: PairingField,
    
    /// Error message to display.
    pub error: Option<String>,
    
    /// Success message.
    pub success: Option<String>,
    
    /// Whether the dialog is visible.
    pub visible: bool,
}

/// Steps in the pairing flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingStep {
    /// Show the public key for the user to copy.
    ShowKey,
    /// Enter gateway connection details.
    EnterGateway,
    /// Connecting to gateway.
    Connecting,
    /// Pairing complete.
    Complete,
}

/// Input fields in the pairing dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingField {
    Host,
    Port,
}

impl Default for PairingDialog {
    fn default() -> Self {
        Self {
            step: PairingStep::ShowKey,
            public_key: String::new(),
            fingerprint: String::new(),
            fingerprint_art: String::new(),
            qr_ascii: None,
            gateway_host: String::new(),
            gateway_port: "2222".to_string(),
            cursor_pos: 0,
            active_field: PairingField::Host,
            error: None,
            success: None,
            visible: false,
        }
    }
}

impl PairingDialog {
    /// Create a new pairing dialog with the given key info.
    pub fn new(public_key: String, fingerprint: String, fingerprint_art: String) -> Self {
        Self {
            public_key,
            fingerprint,
            fingerprint_art,
            ..Default::default()
        }
    }
    
    /// Set the QR code ASCII art.
    pub fn with_qr(mut self, qr: String) -> Self {
        self.qr_ascii = Some(qr);
        self
    }
    
    /// Show the dialog.
    pub fn show(&mut self) {
        self.visible = true;
        self.error = None;
        self.success = None;
    }
    
    /// Hide the dialog.
    pub fn hide(&mut self) {
        self.visible = false;
    }
    
    /// Move to the next step.
    pub fn next_step(&mut self) {
        self.step = match self.step {
            PairingStep::ShowKey => PairingStep::EnterGateway,
            PairingStep::EnterGateway => PairingStep::Connecting,
            PairingStep::Connecting => PairingStep::Complete,
            PairingStep::Complete => PairingStep::Complete,
        };
        self.error = None;
    }
    
    /// Move to the previous step.
    pub fn prev_step(&mut self) {
        self.step = match self.step {
            PairingStep::ShowKey => PairingStep::ShowKey,
            PairingStep::EnterGateway => PairingStep::ShowKey,
            PairingStep::Connecting => PairingStep::EnterGateway,
            PairingStep::Complete => PairingStep::EnterGateway,
        };
        self.error = None;
    }
    
    /// Toggle the active input field.
    pub fn toggle_field(&mut self) {
        self.active_field = match self.active_field {
            PairingField::Host => PairingField::Port,
            PairingField::Port => PairingField::Host,
        };
        self.cursor_pos = match self.active_field {
            PairingField::Host => self.gateway_host.len(),
            PairingField::Port => self.gateway_port.len(),
        };
    }
    
    /// Handle character input.
    pub fn insert_char(&mut self, c: char) {
        let field = match self.active_field {
            PairingField::Host => &mut self.gateway_host,
            PairingField::Port => &mut self.gateway_port,
        };
        
        // For port, only allow digits
        if self.active_field == PairingField::Port && !c.is_ascii_digit() {
            return;
        }
        
        field.insert(self.cursor_pos, c);
        self.cursor_pos += 1;
    }
    
    /// Handle backspace.
    pub fn delete_char(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        
        let field = match self.active_field {
            PairingField::Host => &mut self.gateway_host,
            PairingField::Port => &mut self.gateway_port,
        };
        
        self.cursor_pos -= 1;
        field.remove(self.cursor_pos);
    }
    
    /// Move cursor left.
    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }
    
    /// Move cursor right.
    pub fn cursor_right(&mut self) {
        let len = match self.active_field {
            PairingField::Host => self.gateway_host.len(),
            PairingField::Port => self.gateway_port.len(),
        };
        if self.cursor_pos < len {
            self.cursor_pos += 1;
        }
    }
    
    /// Set an error message.
    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }
    
    /// Set a success message.
    pub fn set_success(&mut self, success: impl Into<String>) {
        self.success = Some(success.into());
    }
    
    /// Get the gateway address to connect to.
    pub fn gateway_address(&self) -> String {
        let port = self.gateway_port.parse::<u16>().unwrap_or(2222);
        format!("{}:{}", self.gateway_host, port)
    }
    
    /// Render the dialog.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }
        
        // Calculate dialog size (60% width, 80% height)
        let dialog_width = (area.width as f32 * 0.6) as u16;
        let dialog_height = (area.height as f32 * 0.8) as u16;
        
        let dialog_x = (area.width - dialog_width) / 2;
        let dialog_y = (area.height - dialog_height) / 2;
        
        let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);
        
        // Clear the background
        frame.render_widget(Clear, dialog_area);
        
        // Draw the dialog border
        let title = match self.step {
            PairingStep::ShowKey => " Pair with Gateway — Step 1/2 ",
            PairingStep::EnterGateway => " Pair with Gateway — Step 2/2 ",
            PairingStep::Connecting => " Connecting... ",
            PairingStep::Complete => " Pairing Complete ",
        };
        
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_alignment(Alignment::Center)
            .style(Style::default().fg(Color::White).bg(Color::Black));
        
        frame.render_widget(block, dialog_area);
        
        // Render content inside the dialog
        let inner = Rect::new(
            dialog_area.x + 2,
            dialog_area.y + 1,
            dialog_area.width.saturating_sub(4),
            dialog_area.height.saturating_sub(2),
        );
        
        match self.step {
            PairingStep::ShowKey => self.render_show_key(frame, inner),
            PairingStep::EnterGateway => self.render_enter_gateway(frame, inner),
            PairingStep::Connecting => self.render_connecting(frame, inner),
            PairingStep::Complete => self.render_complete(frame, inner),
        }
    }
    
    fn render_show_key(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Instructions
                Constraint::Length(4),  // Public key
                Constraint::Length(2),  // Fingerprint
                Constraint::Min(10),    // Fingerprint art or QR
                Constraint::Length(2),  // Help text
            ])
            .split(area);
        
        // Instructions
        let instructions = Paragraph::new(vec![
            Line::from("Copy your public key and add it to the gateway's"),
            Line::from(Span::styled(
                "~/.rustyclaw/authorized_clients",
                Style::default().add_modifier(Modifier::BOLD),
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(instructions, chunks[0]);
        
        // Public key (scrollable if too long)
        let key_block = Block::default()
            .borders(Borders::ALL)
            .title(" Public Key ");
        let key_text = Paragraph::new(self.public_key.clone())
            .block(key_block)
            .wrap(Wrap { trim: false });
        frame.render_widget(key_text, chunks[1]);
        
        // Fingerprint
        let fingerprint = Paragraph::new(Line::from(vec![
            Span::raw("Fingerprint: "),
            Span::styled(&self.fingerprint, Style::default().fg(Color::Cyan)),
        ]))
        .alignment(Alignment::Center);
        frame.render_widget(fingerprint, chunks[2]);
        
        // Fingerprint art or QR code
        let visual = if let Some(ref qr) = self.qr_ascii {
            qr.clone()
        } else {
            self.fingerprint_art.clone()
        };
        let visual_block = Block::default()
            .borders(Borders::ALL)
            .title(if self.qr_ascii.is_some() { " QR Code " } else { " Key Art " });
        let visual_text = Paragraph::new(visual)
            .block(visual_block)
            .alignment(Alignment::Center);
        frame.render_widget(visual_text, chunks[3]);
        
        // Help text
        let help = Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Next  "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Cancel"),
        ]))
        .alignment(Alignment::Center);
        frame.render_widget(help, chunks[4]);
    }
    
    fn render_enter_gateway(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),  // Instructions
                Constraint::Length(3),  // Host input
                Constraint::Length(3),  // Port input
                Constraint::Min(1),     // Spacer
                Constraint::Length(2),  // Error/help
            ])
            .split(area);
        
        // Instructions
        let instructions = Paragraph::new("Enter the gateway's SSH address:")
            .alignment(Alignment::Center);
        frame.render_widget(instructions, chunks[0]);
        
        // Host input
        let host_style = if self.active_field == PairingField::Host {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let host_block = Block::default()
            .borders(Borders::ALL)
            .title(" Host ")
            .border_style(host_style);
        let host_text = Paragraph::new(self.gateway_host.clone())
            .block(host_block);
        frame.render_widget(host_text, chunks[1]);
        
        // Port input
        let port_style = if self.active_field == PairingField::Port {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let port_block = Block::default()
            .borders(Borders::ALL)
            .title(" Port ")
            .border_style(port_style);
        let port_text = Paragraph::new(self.gateway_port.clone())
            .block(port_block);
        frame.render_widget(port_text, chunks[2]);
        
        // Error or help text
        let bottom_text = if let Some(ref err) = self.error {
            Paragraph::new(Line::from(Span::styled(
                err.as_str(),
                Style::default().fg(Color::Red),
            )))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled("[Tab]", Style::default().fg(Color::Yellow)),
                Span::raw(" Switch field  "),
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Connect  "),
                Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
                Span::raw(" Back"),
            ]))
        };
        frame.render_widget(bottom_text.alignment(Alignment::Center), chunks[4]);
    }
    
    fn render_connecting(&self, frame: &mut Frame, area: Rect) {
        let text = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "Connecting to gateway...",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(self.gateway_address()),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(text, area);
    }
    
    fn render_complete(&self, frame: &mut Frame, area: Rect) {
        let message = self.success.as_deref().unwrap_or("Pairing successful!");
        
        let text = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "✓ ",
                Style::default().fg(Color::Green),
            )),
            Line::from(Span::styled(
                message,
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Close"),
            ]),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(text, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_pairing_dialog_flow() {
        let mut dialog = PairingDialog::new(
            "ssh-ed25519 AAAA...".to_string(),
            "SHA256:abc123".to_string(),
            "+---[ED25519]---+".to_string(),
        );
        
        assert_eq!(dialog.step, PairingStep::ShowKey);
        
        dialog.next_step();
        assert_eq!(dialog.step, PairingStep::EnterGateway);
        
        dialog.gateway_host = "example.com".to_string();
        assert_eq!(dialog.gateway_address(), "example.com:2222");
    }
    
    #[test]
    fn test_input_handling() {
        let mut dialog = PairingDialog::default();
        dialog.active_field = PairingField::Host;
        dialog.cursor_pos = 0;
        
        dialog.insert_char('t');
        dialog.insert_char('e');
        dialog.insert_char('s');
        dialog.insert_char('t');
        
        assert_eq!(dialog.gateway_host, "test");
        assert_eq!(dialog.cursor_pos, 4);
        
        dialog.delete_char();
        assert_eq!(dialog.gateway_host, "tes");
    }
}
