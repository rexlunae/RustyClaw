use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;

use crate::config::Config;
use crate::secrets::SecretsManager;
use crate::skills::SkillManager;
use crate::soul::SoulManager;

pub struct App {
    config: Config,
    secrets_manager: SecretsManager,
    skill_manager: SkillManager,
    soul_manager: SoulManager,
    input: String,
    messages: Vec<String>,
    current_view: View,
    should_quit: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum View {
    Main,
    Skills,
    Secrets,
    Config,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let secrets_manager = SecretsManager::new("rustyclaw");
        let skills_dir = config.skills_dir.clone()
            .unwrap_or_else(|| config.settings_dir.join("skills"));
        let mut skill_manager = SkillManager::new(skills_dir);
        skill_manager.load_skills()?;

        let soul_path = config.soul_path.clone()
            .unwrap_or_else(|| config.settings_dir.join("SOUL.md"));
        let mut soul_manager = SoulManager::new(soul_path);
        soul_manager.load()?;

        Ok(Self {
            config,
            secrets_manager,
            skill_manager,
            soul_manager,
            input: String::new(),
            messages: vec![
                "Welcome to RustyClaw!".to_string(),
                "Type 'help' for available commands".to_string(),
            ],
            current_view: View::Main,
            should_quit: false,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Run the app
        let res = self.run_app(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        if let Err(err) = res {
            eprintln!("Error: {:?}", err);
        }

        Ok(())
    }

    fn run_app<B: ratatui::backend::Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            terminal.draw(|f| self.ui(f))?;

            if event::poll(std::time::Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') if self.current_view == View::Main => {
                                self.should_quit = true;
                            }
                            KeyCode::Char(c) => {
                                self.input.push(c);
                            }
                            KeyCode::Backspace => {
                                self.input.pop();
                            }
                            KeyCode::Enter => {
                                self.handle_input();
                            }
                            KeyCode::Esc => {
                                self.current_view = View::Main;
                            }
                            KeyCode::F(1) => {
                                self.current_view = View::Main;
                            }
                            KeyCode::F(2) => {
                                self.current_view = View::Skills;
                            }
                            KeyCode::F(3) => {
                                self.current_view = View::Secrets;
                            }
                            KeyCode::F(4) => {
                                self.current_view = View::Config;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if self.should_quit {
                return Ok(());
            }
        }
    }

    fn ui(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .split(f.size());

        // Title
        let title = Paragraph::new("RustyClaw - Lightweight Secure Agent")
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(title, chunks[0]);

        // Main content area
        match self.current_view {
            View::Main => self.render_main_view(f, chunks[1]),
            View::Skills => self.render_skills_view(f, chunks[1]),
            View::Secrets => self.render_secrets_view(f, chunks[1]),
            View::Config => self.render_config_view(f, chunks[1]),
        }

        // Input area
        let input = Paragraph::new(self.input.as_str())
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).title("Input"));
        f.render_widget(input, chunks[2]);

        // Help bar
        let help_text = vec![
            Span::raw("F1: Main | F2: Skills | F3: Secrets | F4: Config | "),
            Span::styled("q", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw(": Quit (Main view) | "),
            Span::styled("ESC", Style::default().fg(Color::Yellow)),
            Span::raw(": Back to Main"),
        ];
        let help = Paragraph::new(Line::from(help_text))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(help, chunks[3]);
    }

    fn render_main_view(&self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .messages
            .iter()
            .map(|m| ListItem::new(m.as_str()))
            .collect();

        let messages_list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Messages"))
            .style(Style::default().fg(Color::White));

        f.render_widget(messages_list, area);
    }

    fn render_skills_view(&self, f: &mut Frame, area: Rect) {
        let skills = self.skill_manager.get_skills();
        let items: Vec<ListItem> = skills
            .iter()
            .map(|s| {
                let status = if s.enabled { "✓" } else { "✗" };
                let text = format!("{} {} - {}", status, s.name, 
                    s.description.as_deref().unwrap_or("No description"));
                ListItem::new(text)
            })
            .collect();

        let skills_list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Skills"))
            .style(Style::default().fg(Color::White));

        f.render_widget(skills_list, area);
    }

    fn render_secrets_view(&self, f: &mut Frame, area: Rect) {
        let agent_access = if self.secrets_manager.has_agent_access() {
            "Enabled"
        } else {
            "Disabled"
        };

        let text = [
            format!("Agent Access: {}", agent_access),
            String::new(),
            "Commands:".to_string(),
            "  enable-access - Enable agent access to secrets".to_string(),
            "  disable-access - Disable agent access to secrets".to_string(),
            "  store <key> <value> - Store a secret".to_string(),
            "  delete <key> - Delete a secret".to_string(),
        ];

        let items: Vec<ListItem> = text
            .iter()
            .map(|t| ListItem::new(t.as_str()))
            .collect();

        let secrets_info = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Secrets Management"))
            .style(Style::default().fg(Color::White));

        f.render_widget(secrets_info, area);
    }

    fn render_config_view(&self, f: &mut Frame, area: Rect) {
        let soul_content = self.soul_manager.get_content()
            .unwrap_or("No SOUL content loaded");

        let text = [
            format!("Settings Directory: {}", self.config.settings_dir.display()),
            format!("SOUL Path: {}", self.soul_manager.get_path().display()),
            format!("Use Secrets: {}", self.config.use_secrets),
            String::new(),
            "SOUL Content Preview:".to_string(),
            soul_content.lines().take(10).collect::<Vec<_>>().join("\n"),
        ];

        let config_text = text.join("\n");
        let config_para = Paragraph::new(config_text)
            .block(Block::default().borders(Borders::ALL).title("Configuration"))
            .wrap(Wrap { trim: true });

        f.render_widget(config_para, area);
    }

    fn handle_input(&mut self) {
        let input = self.input.clone();
        self.input.clear();

        if input.is_empty() {
            return;
        }

        // Parse and handle commands
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        match parts[0] {
            "help" => {
                self.messages.push("Available commands:".to_string());
                self.messages.push("  help - Show this help".to_string());
                self.messages.push("  clear - Clear messages".to_string());
                self.messages.push("  enable-access - Enable agent access to secrets".to_string());
                self.messages.push("  disable-access - Disable agent access to secrets".to_string());
                self.messages.push("  reload-skills - Reload skills".to_string());
                self.messages.push("  q - Quit (from Main view)".to_string());
            }
            "clear" => {
                self.messages.clear();
                self.messages.push("Messages cleared.".to_string());
            }
            "enable-access" => {
                self.secrets_manager.set_agent_access(true);
                self.messages.push("Agent access to secrets enabled.".to_string());
            }
            "disable-access" => {
                self.secrets_manager.set_agent_access(false);
                self.messages.push("Agent access to secrets disabled.".to_string());
            }
            "reload-skills" => {
                match self.skill_manager.load_skills() {
                    Ok(_) => {
                        let count = self.skill_manager.get_skills().len();
                        self.messages.push(format!("Reloaded {} skills.", count));
                    }
                    Err(e) => {
                        self.messages.push(format!("Error reloading skills: {}", e));
                    }
                }
            }
            _ => {
                self.messages.push(format!("Unknown command: {}", input));
                self.messages.push("Type 'help' for available commands".to_string());
            }
        }
    }
}
