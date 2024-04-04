use std::{fmt::Write, fs::read_to_string, path::PathBuf, time::Duration};

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::Backend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, BorderType, Clear, List, Paragraph, StatefulWidget, Widget,
    },
    Terminal,
};
use serde_json::Value;

use hitman::{
    env::{find_available_requests, find_root_dir, load_env, update_data},
    extract::extract_variables,
    request::{build_client, do_request},
    substitute::{substitute, SubstituteError, SubstitutionType},
};
use tokio::task::JoinHandle;

use super::{
    output::{OutputView, OutputViewState},
    select::{RequestSelector, RequestSelectorState, Select, SelectState},
};

pub struct App {
    root_dir: PathBuf,
    selector_state: RequestSelectorState,
    output_state: OutputViewState,
    search: String, // Move into selector_state

    state: AppState,
}

pub enum AppState {
    Normal,

    PendingValue {
        key: String,
        pending_options: Vec<(String, String)>,

        pending_state: PendingState,
    },

    RunningRequest {
        handle: JoinHandle<Result<(String, String)>>,
    },

    SelectEnvironment,
}

pub enum PendingState {
    Prompt {
        fallback: Option<String>,
    },
    Select {
        values: Vec<toml::Value>,
        select_state: SelectState,
    },
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
            search: String::new(),
            state: AppState::Normal,
        })
    }

    pub async fn run<B>(&mut self, mut terminal: Terminal<B>) -> Result<()>
    where
        B: Backend,
    {
        let mut should_quit = false;
        while !should_quit {
            terminal.draw(|frame| {
                self.render(frame.size(), frame.buffer_mut());

                // XXX Coupling
                // Check out tui-input
                frame.set_cursor(
                    frame.size().x + 11 + self.search.len() as u16,
                    frame.size().y + frame.size().height - 3,
                );
            })?;

            if let AppState::RunningRequest { handle } = &mut self.state {
                if handle.is_finished() {
                    let (request, response) = handle.await??;
                    self.output_state.update(request, response);
                    self.state = AppState::Normal;
                }
            }

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
            [Constraint::Max(60), Constraint::Min(1)],
        )
        .split(area);

        self.render_left(layout[0], buf);

        StatefulWidget::render(
            OutputView::default(),
            layout[1],
            buf,
            &mut self.output_state,
        );
    }

    fn render_left(&mut self, area: Rect, buf: &mut Buffer) {
        let layout = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Max(3)],
        )
        .split(area);

        // XXX Consider passing items to widget instance, instead of using state
        StatefulWidget::render(
            RequestSelector::new(&self.search),
            layout[0],
            buf,
            &mut self.selector_state,
        );

        Widget::render(
            Paragraph::new(Line::from(vec![
                Span::from("  Search: "),
                Span::from(self.search.clone()).yellow(),
            ]))
            .block(Block::bordered().border_set(
                symbols::border::Set {
                    top_left: symbols::border::PLAIN.vertical_right,
                    top_right: symbols::border::PLAIN.vertical_left,
                    ..symbols::border::PLAIN
                },
            )),
            layout[1],
            buf,
        );
    }

    fn render_status(&mut self, area: Rect, buf: &mut Buffer) {
        let status_line = Paragraph::new("S: Select environment")
            .style(Style::new().dark_gray());
        status_line.render(area, buf);
    }

    fn render_popup(&mut self, area: Rect, buf: &mut Buffer) {
        match &mut self.state {
            AppState::PendingValue {
                key,
                pending_state,
                // ref mut select_state,
                ..
            } => {
                match pending_state {
                    PendingState::Prompt { fallback } => {
                        // TODO: Check out using tui-input

                        let inner_area = area.inner(&Margin::new(42, 10));
                        Widget::render(Clear, inner_area, buf);
                        Widget::render(
                            Block::bordered()
                                .title(format!("Enter value for {key}")),
                            inner_area,
                            buf,
                        );
                    }
                    PendingState::Select {
                        values,
                        select_state,
                    } => {
                        let select = Select::default().block(
                            Block::bordered().title(format!(
                                "Select substitution value for {{{{{key}}}}}"
                            )),
                        );

                        let inner_area = area.inner(&Margin::new(42, 10));
                        Widget::render(Clear, inner_area, buf);
                        StatefulWidget::render(
                            select,
                            inner_area,
                            buf,
                            select_state,
                        );
                    }
                }
            }

            AppState::SelectEnvironment => {
                let list = List::new(["Local", "Remote", "Prod"])
                    .block(
                        Block::bordered()
                            .title("Select environment")
                            .border_type(BorderType::Rounded),
                    )
                    .style(Style::new().white().on_blue());

                let inner_area = area.inner(&Margin::new(42, 10));
                Widget::render(Clear, inner_area, buf);
                Widget::render(list, inner_area, buf);
            }

            AppState::RunningRequest { .. } => {
                // TODO: Stateful progress spinner widget
                let loading = Paragraph::new("...waiting...")
                    .centered()
                    .block(Block::bordered().border_type(BorderType::Rounded));

                let inner_area = area.inner(&Margin::new(72, 15));
                Widget::render(Clear, inner_area, buf);
                Widget::render(loading, inner_area, buf);
            }

            _ => (),
        }
    }

    async fn handle_events(&mut self) -> Result<bool> {
        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match &mut self.state {
                        AppState::PendingValue { pending_state, .. } => {
                            match pending_state {
                                PendingState::Select {
                                    select_state, ..
                                } => {
                                    // XXX Pass the event and the state to a function,
                                    // rather than calling a method on state?
                                    if select_state.handle_event(event) {
                                        return Ok(false);
                                    }
                                }
                                PendingState::Prompt { fallback } => {
                                    // TODO
                                }
                            }

                            if key.modifiers.contains(KeyModifiers::CONTROL) {
                                match key.code {
                                    KeyCode::Char('c') | KeyCode::Char('q') => {
                                        self.state = AppState::Normal;
                                    }
                                    _ => (),
                                }
                            } else {
                                match key.code {
                                    KeyCode::Enter => {
                                        self.try_request()?;
                                    }
                                    _ => (),
                                }
                            }
                        }

                        AppState::Normal => {
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
                                    KeyCode::Char('w') => {
                                        self.search.clear();
                                    }
                                    KeyCode::Char('s') => {
                                        self.state =
                                            AppState::SelectEnvironment;
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
                                        if self.search.len() < 24 {
                                            self.search.push(ch);
                                            self.selector_state.select_first();
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        self.search.pop();
                                    }
                                    KeyCode::Esc => {
                                        self.search.clear();
                                    }
                                    KeyCode::Enter => {
                                        self.try_request()?;
                                    }
                                    _ => (),
                                }
                            }
                        }

                        AppState::RunningRequest { handle } => {
                            if key.modifiers.contains(KeyModifiers::CONTROL) {
                                match key.code {
                                    KeyCode::Char('c') | KeyCode::Char('q') => {
                                        handle.abort();
                                        self.state = AppState::Normal;
                                    }
                                    _ => (),
                                }
                            }
                        }

                        AppState::SelectEnvironment => {
                            if key.modifiers.contains(KeyModifiers::CONTROL) {
                                match key.code {
                                    KeyCode::Char('c') | KeyCode::Char('q') => {
                                        self.state = AppState::Normal;
                                    }
                                    _ => (),
                                }
                            }
                        }
                    }
                }
                _ => (),
            }
        }
        Ok(false)
    }

    fn try_request(&mut self) -> Result<(), anyhow::Error> {
        if let Some(file_path) = self.selector_state.selected_path() {
            let root_dir = self.root_dir.clone();

            let mut pending_options = match &self.state {
                AppState::PendingValue {
                    pending_options, ..
                } => pending_options.clone(),
                _ => Vec::new(),
            };

            if let AppState::PendingValue {
                key, pending_state, ..
            } = &self.state
            {
                match pending_state {
                    PendingState::Prompt { fallback } => {
                        // TODO: Input
                        let value = fallback.as_deref().unwrap_or("");
                        pending_options.push((key.clone(), value.into()));
                    }
                    PendingState::Select {
                        values,
                        select_state,
                    } => {
                        if let Some(selected) = select_state.selected() {
                            let value = match &values[selected] {
                                toml::Value::Table(t) => match t.get("value") {
                                    Some(toml::Value::String(value)) => {
                                        value.clone()
                                    }
                                    Some(value) => value.to_string(),
                                    _ => bail!("Replacement not found: {key}"),
                                },
                                other => other.to_string(),
                            };

                            pending_options.push((key.clone(), value));
                        }
                    }
                }
            }

            let path = PathBuf::try_from(file_path)?;
            let env = load_env(&root_dir, &path, &pending_options)?;

            match substitute(&read_to_string(path)?, &env) {
                Ok(buf) => {
                    let handle =
                        tokio::spawn(async move { make_request(&buf).await });

                    self.state = AppState::RunningRequest { handle };
                }
                Err(err) => match err {
                    SubstituteError::ReplacementNotFound(not_found) => {
                        match &not_found.substitution_type {
                            SubstitutionType::Select { values } => {
                                // FIXME: We need to keep the 'values' in state,
                                // so it doesn't make sense to prematurely
                                // convert map to strings here
                                let items = values.iter().map(|v| match v {
                                    toml::Value::Table(t) => {
                                        match t.get("name") {
                                            Some(toml::Value::String(
                                                value,
                                            )) => value.clone(),
                                            Some(value) => value.to_string(),
                                            None => t.to_string(),
                                        }
                                    }
                                    other => other.to_string(),
                                });

                                self.state = AppState::PendingValue {
                                    key: not_found.key,
                                    pending_options,
                                    pending_state: PendingState::Select {
                                        values: values.clone(),
                                        select_state: SelectState::new(
                                            items.collect(),
                                        ),
                                    },
                                };
                            }
                            SubstitutionType::Prompt { fallback } => {
                                self.state = AppState::PendingValue {
                                    key: not_found.key,
                                    pending_options,
                                    pending_state: PendingState::Prompt {
                                        fallback: fallback.clone(),
                                    },
                                };
                            }
                        }
                    }
                    e => bail!(e),
                },
            }
        }

        Ok(())
    }
}

async fn make_request(buf: &str) -> Result<(String, String)> {
    let client = build_client()?;

    let mut request = String::new();
    for line in buf.lines() {
        writeln!(request, "> {}", line)?;
    }
    writeln!(request)?;

    let (res, _elapsed) = do_request(&client, &buf).await?;

    let mut response = String::new();
    for (name, value) in res.headers() {
        writeln!(response, "< {}: {}", name, value.to_str()?)?;
    }
    writeln!(response)?;

    if let Ok(json) = res.json::<Value>().await {
        writeln!(response, "{}", serde_json::to_string_pretty(&json)?)?;

        // let options = vec![];
        // let env = load_env(&root_dir, &path, &options)?;
        // let vars = extract_variables(&json, &env)?;
        // update_data(&vars)?;
    }

    Ok((request, response))
}
