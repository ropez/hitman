#![allow(unused)]

use std::{
    fmt::Write,
    fs::read_to_string,
    io::stdout,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Text,
    widgets::{self, Block, Borders, HighlightSpacing, List, ListState, Paragraph},
    Frame, Terminal,
};

use hitman::env::{find_available_requests, find_root_dir, select_env};
use hitman::request::do_request;
use hitman::{
    env::load_env, extract::extract_variables, request::build_client, substitute::substitute,
};
use serde_json::Value;
use ui::select::{RequestSelector, RequestSelectorState};

mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    let _deferred = RawMode::enable()?;

    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let root_dir = find_root_dir()?.context("No hitman.toml found")?;
    // select_env(&root_dir)?;

    let reqs = find_available_requests(&root_dir)?;
    let reqs: Vec<String> = reqs
        .iter()
        .filter_map(|p| p.to_str())
        .map(|s| String::from(s))
        .collect();

    let mut selector_state = RequestSelectorState::new(&reqs);
    let mut output = String::new();
    let mut output_scroll: (u16, u16) = (0, 0);

    let mut should_quit = false;
    while !should_quit {
        terminal.draw(|frame| render_ui(frame, &mut selector_state, &output, output_scroll))?;
        should_quit = handle_events(
            &root_dir,
            &mut selector_state,
            &mut output,
            &mut output_scroll,
        )
        .await?;
    }

    stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}

fn render_ui(
    frame: &mut Frame,
    selector_state: &mut RequestSelectorState,
    output: &String,
    output_scroll: (u16, u16),
) {
    let vert_layout = Layout::new(
        Direction::Vertical,
        [
            // Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ],
    )
    .split(frame.size());

    let main_layout = Layout::new(
        Direction::Horizontal,
        [Constraint::Max(40), Constraint::Min(1)],
    )
    .split(vert_layout[0]);

    frame.render_stateful_widget(RequestSelector::default(), main_layout[0], selector_state);

    frame.render_widget(
        Paragraph::new(output.clone())
            .scroll(output_scroll)
            .block(Block::default().title("Output").borders(Borders::ALL)),
        main_layout[1],
    );
}

async fn handle_events(
    root_dir: &Path,
    selector_state: &mut RequestSelectorState,
    output: &mut String,
    output_scroll: &mut (u16, u16),
) -> Result<bool> {
    if event::poll(Duration::from_millis(50))? {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') => {
                    return Ok(true);
                }
                KeyCode::Char('j') => {
                    selector_state.select_next();
                }
                KeyCode::Char('k') => {
                    selector_state.select_prev();
                }
                KeyCode::Char('p') => {
                    if output_scroll.0 <= 5 {
                        output_scroll.0 = 0;
                    } else {
                        output_scroll.0 -= 5;
                    }
                }
                KeyCode::Char('n') => {
                    output_scroll.0 += 5;
                }
                KeyCode::Enter => match selector_state.selected_path() {
                    Some(file_path) => {
                        let options = vec![];
                        let path = PathBuf::try_from(file_path)?;
                        let env = load_env(root_dir, &path, &options)?;

                        output.clear();

                        let client = build_client()?;
                        let buf = substitute(&read_to_string(file_path)?, &env)?;

                        for line in buf.lines() {
                            writeln!(output, "> {}", line);
                        }
                        writeln!(output);

                        let (res, elapsed) = do_request(&client, &buf).await?;

                        let mut head = String::new();
                        for (name, value) in res.headers() {
                            head.push_str(&format!("{}: {}\n", name, value.to_str()?));
                        }

                        for line in head.lines() {
                            writeln!(output, "< {}", line);
                        }
                        writeln!(output);

                        if let Ok(json) = res.json::<Value>().await {
                            writeln!(output, "{}", serde_json::to_string_pretty(&json)?);
                            // let vars = extract_variables(&json, env)?;
                            // update_data(&vars)?;
                        }
                    }
                    None => (),
                },
                _ => (),
            },
            _ => (),
        }
    }
    Ok(false)
}

struct RawMode;

impl RawMode {
    fn enable() -> Result<Self, std::io::Error> {
        enable_raw_mode()?;
        Ok(RawMode)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}
