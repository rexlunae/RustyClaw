mod config;
mod messenger;
mod secrets;
mod skills;
mod soul;
mod tui;

use anyhow::Result;
use config::Config;
use tui::App;

fn main() -> Result<()> {
    // Load configuration
    let config = Config::load(None)?;

    // Create and run the TUI application
    let mut app = App::new(config)?;
    app.run()?;

    Ok(())
}

