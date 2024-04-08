use std::{
    io::{self, stdout},
    panic::{set_hook, take_hook},
};

use anyhow::Result;
use crossterm::{
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};

use ui::app::{App, Screen};

mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    init_panic_hook();
    let mut app = App::new()?;

    let screen = CrosstermScreen::default();
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    app.run(terminal, screen).await?;

    Ok(())
}

pub fn init_panic_hook() {
    let original_hook = take_hook();
    set_hook(Box::new(move |panic_info| {
        let _ = CrosstermScreen::default().leave();
        original_hook(panic_info);
    }));
}

#[derive(Default)]
struct CrosstermScreen;

impl Screen for CrosstermScreen {
    fn enter(&self) -> io::Result<()> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        Ok(())
    }

    fn leave(&self) -> io::Result<()> {
        stdout().execute(LeaveAlternateScreen)?;
        disable_raw_mode()?;
        Ok(())
    }
}
