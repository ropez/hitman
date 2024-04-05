use std::{io::stdout, panic::{take_hook, set_hook}};

use anyhow::Result;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, Clear, ClearType},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};

use ui::app::App;

mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    init_panic_hook();

    let _guard = Screen::enable()?;

    let mut app = App::new()?;

    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    app.run(terminal).await?;

    Ok(())
}

pub fn init_panic_hook() {
    let original_hook = take_hook();
    set_hook(Box::new(move |panic_info| {
        // intentionally ignore errors here since we're already in a panic
        let _ = restore_tui();
        original_hook(panic_info);
    }));
}

struct Screen;

impl Screen {
    fn enable() -> Result<Self, std::io::Error> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        stdout().execute(Clear(ClearType::All))?;

        Ok(Screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = restore_tui();
    }
}

fn restore_tui() -> Result<()> {
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
