use std::{
    io::{self, stdout, Write},
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

    let screen = CrosstermScreen::new(CrosstermBackend::new(stdout()))?;
    app.run(screen).await?;

    Ok(())
}

pub fn init_panic_hook() {
    let original_hook = take_hook();
    set_hook(Box::new(move |panic_info| {
        let _ = stdout().execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();
        original_hook(panic_info);
    }));
}

#[derive(Default)]
struct CrosstermScreen<W>
where
    W: Write,
{
    terminal: Terminal<CrosstermBackend<W>>,
}

impl<W> CrosstermScreen<W>
where
    W: Write,
{
    fn new(backend: CrosstermBackend<W>) -> io::Result<Self> {
        Ok(Self {
            terminal: Terminal::new(backend)?,
        })
    }
}

impl<W> Screen for CrosstermScreen<W>
where
    W: Write,
{
    type B = CrosstermBackend<W>;

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

    fn terminal(&mut self) -> &mut Terminal<Self::B> {
        &mut self.terminal
    }
}
