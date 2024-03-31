use std::{fmt::Write, fs::read_to_string, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::Backend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Style, Stylize},
    widgets::{Block, BorderType, Clear, List, Paragraph, StatefulWidget, Widget},
    Terminal,
};
use serde_json::Value;

use hitman::{
    env::{find_available_requests, find_root_dir, load_env},
    request::{build_client, do_request},
    substitute::substitute,
};

use super::{
    output::{OutputView, OutputViewState},
    select::{RequestSelector, RequestSelectorState},
};

pub struct App {
    root_dir: PathBuf,
    selector_state: RequestSelectorState,
    output_state: OutputViewState,
    environment_state: Option<bool>,
}

impl App {
    pub fn new() -> Result<Self> {
        let root_dir = find_root_dir()?.context("No hitman.toml found")?;

        let reqs = find_available_requests(&root_dir)?;
        let reqs: Vec<String> = reqs
            .iter()
            .filter_map(|p| p.to_str())
            .map(|s| String::from(s))
            .collect();

        let selector_state = RequestSelectorState::new(&reqs);
        let output_state = OutputViewState::default();

        Ok(Self {
            root_dir: root_dir.into(),
            selector_state,
            output_state,
            environment_state: None,
        })
    }

    pub async fn run<B>(&mut self, mut terminal: Terminal<B>) -> Result<()>
    where
        B: Backend,
    {
        let mut should_quit = false;
        while !should_quit {
            terminal.draw(|frame| self.render(frame.size(), frame.buffer_mut()))?;
            should_quit = self.handle_events().await?;
        }

        Ok(())
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let layout = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Length(1)],
        )
        .split(area);

        self.render_main(layout[0], buf);
        self.render_status(layout[1], buf);
        self.render_popup(area, buf);
    }

    fn render_main(&mut self, area: Rect, buf: &mut Buffer) {
        let layout = Layout::new(
            Direction::Horizontal,
            [Constraint::Max(40), Constraint::Min(1)],
        )
        .split(area);

        RequestSelector::default().render(layout[0], buf, &mut self.selector_state);
        OutputView::default().render(layout[1], buf, &mut self.output_state);
    }

    fn render_status(&mut self, area: Rect, buf: &mut Buffer) {
        let status_line = Paragraph::new("S: Select environment").style(Style::new().dark_gray());
        status_line.render(area, buf);
    }

    fn render_popup(&mut self, area: Rect, buf: &mut Buffer) {
        if let Some(st) = self.environment_state {
            let list = List::new(["Local", "Remote", "Prod"])
                .block(
                    Block::bordered()
                        .title("Select environment")
                        .border_type(BorderType::Rounded),
                )
                .style(Style::new().white().on_blue());

            let inner_area = area.inner(&Margin::new(42, 10));
            Clear.render(inner_area, buf);
            Widget::render(list, inner_area, buf);
        }
    }

    async fn handle_events(&mut self) -> Result<bool> {
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        match key.code {
                            KeyCode::Char('c') | KeyCode::Char('q') => {
                                return Ok(true);
                            }
                            KeyCode::Char('j') => {
                                self.selector_state.select_next();
                            }
                            KeyCode::Char('k') => {
                                self.selector_state.select_prev();
                            }
                            KeyCode::Char('p') => {
                                self.output_state.scroll_up();
                            }
                            KeyCode::Char('n') => {
                                self.output_state.scroll_down();
                            }
                            KeyCode::Char('s') => {
                                self.environment_state = Some(true);
                            }
                            _ => (),
                        }
                    } else {
                        match key.code {
                            KeyCode::Down => {
                                self.selector_state.select_next();
                            }
                            KeyCode::Up => {
                                self.selector_state.select_prev();
                            }
                            KeyCode::Char(ch) => {
                                self.selector_state.input(ch);
                            }
                            KeyCode::Esc | KeyCode::Backspace => {
                                self.selector_state.clear_search();
                            }
                            KeyCode::Enter => match self.selector_state.selected_path() {
                                Some(file_path) => {
                                    let options = vec![];
                                    let path = PathBuf::try_from(file_path)?;
                                    let env = load_env(&self.root_dir, &path, &options)?;

                                    let client = build_client()?;
                                    let buf = substitute(&read_to_string(file_path)?, &env)?;

                                    let mut request = String::new();
                                    for line in buf.lines() {
                                        writeln!(request, "> {}", line)?;
                                    }
                                    writeln!(request)?;

                                    let (res, elapsed) = do_request(&client, &buf).await?;

                                    let mut head = String::new();
                                    for (name, value) in res.headers() {
                                        head.push_str(&format!("{}: {}\n", name, value.to_str()?));
                                    }

                                    let mut response = String::new();
                                    for line in head.lines() {
                                        writeln!(response, "< {}", line)?;
                                    }
                                    writeln!(response)?;

                                    if let Ok(json) = res.json::<Value>().await {
                                        writeln!(
                                            response,
                                            "{}",
                                            serde_json::to_string_pretty(&json)?
                                        )?;
                                        // let vars = extract_variables(&json, env)?;
                                        // update_data(&vars)?;
                                    }

                                    self.output_state.update(request, response);
                                }
                                None => (),
                            },
                            _ => (),
                        }
                    }
                }
                _ => (),
            }
        }
        Ok(false)
    }
}
