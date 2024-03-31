use std::io::stdout;

use anyhow::Result;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};

use ui::app::App;

mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    let _guard = Screen::enable()?;

    let mut app = App::new()?;

    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    app.run(terminal).await?;

    Ok(())
}

struct Screen;

impl Screen {
    fn enable() -> Result<Self, std::io::Error> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;

        Ok(Screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = stdout().execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
