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
use serde_json::Value;

use hitman::env::{find_available_requests, find_root_dir, select_env};
use hitman::request::do_request;
use hitman::{
    env::load_env, extract::extract_variables, request::build_client, substitute::substitute,
};

use ui::{
    output::{OutputView, OutputViewState},
    select::{RequestSelector, RequestSelectorState},
};

mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    let _guard = Screen::enable()?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let root_dir = find_root_dir()?.context("No hitman.toml found")?;

    let mut state = State::new(&root_dir)?;

    let mut should_quit = false;
    while !should_quit {
        terminal.draw(|frame| render_ui(frame, &mut state))?;
        should_quit = handle_events(&root_dir, &mut state).await?;
    }

    Ok(())
}

fn render_ui(frame: &mut Frame, state: &mut State) {
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

    frame.render_stateful_widget(
        RequestSelector::default(),
        main_layout[0],
        &mut state.selector_state,
    );

    frame.render_stateful_widget(
        OutputView::default(),
        main_layout[1],
        &mut state.output_state,
    );
}

async fn handle_events(root_dir: &Path, state: &mut State) -> Result<bool> {
    if event::poll(Duration::from_millis(50))? {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') => {
                    return Ok(true);
                }
                KeyCode::Char('j') => {
                    state.selector_state.select_next();
                }
                KeyCode::Char('k') => {
                    state.selector_state.select_prev();
                }
                KeyCode::Char('p') => {
                    state.output_state.scroll_up();
                }
                KeyCode::Char('n') => {
                    state.output_state.scroll_down();
                }
                KeyCode::Enter => match state.selector_state.selected_path() {
                    Some(file_path) => {
                        let options = vec![];
                        let path = PathBuf::try_from(file_path)?;
                        let env = load_env(root_dir, &path, &options)?;

                        let client = build_client()?;
                        let buf = substitute(&read_to_string(file_path)?, &env)?;

                        let mut request = String::new();
                        for line in buf.lines() {
                            writeln!(request, "> {}", line);
                        }
                        writeln!(request);

                        let (res, elapsed) = do_request(&client, &buf).await?;

                        let mut head = String::new();
                        for (name, value) in res.headers() {
                            head.push_str(&format!("{}: {}\n", name, value.to_str()?));
                        }

                        let mut response = String::new();
                        for line in head.lines() {
                            writeln!(response, "< {}", line);
                        }
                        writeln!(response);

                        if let Ok(json) = res.json::<Value>().await {
                            writeln!(response, "{}", serde_json::to_string_pretty(&json)?);
                            // let vars = extract_variables(&json, env)?;
                            // update_data(&vars)?;
                        }

                        state.output_state.update(request, response);
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

struct State {
    selector_state: RequestSelectorState,
    output_state: OutputViewState,
}

impl State {
    pub fn new(root_dir: &Path) -> Result<Self> {
        let reqs = find_available_requests(&root_dir)?;
        let reqs: Vec<String> = reqs
            .iter()
            .filter_map(|p| p.to_str())
            .map(|s| String::from(s))
            .collect();

        let mut selector_state = RequestSelectorState::new(&reqs);
        let mut output_state = OutputViewState::default();

        Ok(Self {
            selector_state,
            output_state,
        })
    }
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
