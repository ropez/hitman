use std::{fmt::Write, fs::read_to_string, path::PathBuf, time::Duration};

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Style, Stylize},
    widgets::{Block, BorderType, Clear, List, Paragraph},
    Frame, Terminal,
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
    select::{Component, RequestSelector, Select},
};

pub struct App {
    root_dir: PathBuf,
    request_selector: RequestSelector,
    output_state: OutputViewState,

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
        component: Select,
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

        let request_selector = RequestSelector::new(&reqs);
        let output_state = OutputViewState::default();

        Ok(Self {
            root_dir: root_dir.into(),
            request_selector,
            output_state,
            state: AppState::Normal,
        })
    }

    pub async fn run<B>(&mut self, mut terminal: Terminal<B>) -> Result<()>
    where
        B: Backend,
    {
        let mut should_quit = false;
        while !should_quit {
            terminal.draw(|frame| self.render_ui(frame))?;

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

    fn render_ui(&mut self, frame: &mut Frame) {
        let layout = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Length(1)],
        )
        .split(frame.size());

        self.render_main(frame, layout[0]);
        self.render_status(frame, layout[1]);
        self.render_popup(frame);
    }

    fn render_main(&mut self, frame: &mut Frame, area: Rect) {
        let layout = Layout::new(
            Direction::Horizontal,
            [Constraint::Max(60), Constraint::Min(1)],
        )
        .split(area);

        self.render_left(frame, layout[0]);

        frame.render_stateful_widget(
            OutputView::default(),
            layout[1],
            &mut self.output_state,
        );
    }

    fn render_left(&mut self, frame: &mut Frame, area: Rect) {
        self.request_selector.render_ui(frame, area);
    }

    fn render_status(&mut self, frame: &mut Frame, area: Rect) {
        let status_line = Paragraph::new("S: Select environment")
            .style(Style::new().dark_gray());
        frame.render_widget(status_line, area);
    }

    fn render_popup(&mut self, frame: &mut Frame) {
        let area = frame.size();
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
                        frame.render_widget(Clear, inner_area);
                        frame.render_widget(
                            Block::bordered()
                                .title(format!("Enter value for {key}")),
                            inner_area,
                        );
                    }
                    PendingState::Select { component, .. } => {
                        let inner_area = area.inner(&Margin::new(42, 10));
                        component.render_ui(frame, inner_area);
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
                frame.render_widget(Clear, inner_area);
                frame.render_widget(list, inner_area);
            }

            AppState::RunningRequest { .. } => {
                // TODO: Progress spinner component
                let loading = Paragraph::new("...waiting...")
                    .centered()
                    .block(Block::bordered().border_type(BorderType::Rounded));

                let inner_area = area.inner(&Margin::new(72, 15));
                frame.render_widget(Clear, inner_area);
                frame.render_widget(loading, inner_area);
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
                                PendingState::Select { component, .. } => {
                                    // XXX Pass the event and the state to a function,
                                    // rather than calling a method on state?
                                    if component.handle_event(event) {
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
                            if self.request_selector.handle_event(event) {
                                return Ok(false);
                            }

                            if key.modifiers.contains(KeyModifiers::CONTROL) {
                                match key.code {
                                    KeyCode::Char('c') | KeyCode::Char('q') => {
                                        return Ok(true);
                                    }
                                    KeyCode::Char('p') => {
                                        self.output_state.scroll_up();
                                    }
                                    KeyCode::Char('n') => {
                                        self.output_state.scroll_down();
                                    }
                                    KeyCode::Char('s') => {
                                        self.state =
                                            AppState::SelectEnvironment;
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
        if let Some(file_path) = self.request_selector.selected_path() {
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
                        component: select_component,
                    } => {
                        // FIXME: Index is not correct when filtered
                        if let Some(selected) = select_component.selected() {
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
                                // FIXME: We some redundency in the state. Can
                                // we somehow make the Select component hold on
                                // to the `values`, and use something like Into<String>?
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

                                let select = Select::new(
                                    format!("Select substitution value for {{{{{}}}}}", &not_found.key),
                                    items.collect(),
                                );

                                self.state = AppState::PendingValue {
                                    key: not_found.key,
                                    pending_options,
                                    pending_state: PendingState::Select {
                                        values: values.clone(),
                                        component: select,
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
