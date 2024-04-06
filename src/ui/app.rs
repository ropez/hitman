use std::{
    fmt::Write,
    fs::read_to_string,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Style, Stylize},
    widgets::Paragraph,
    Frame, Terminal,
};
use tokio::task::JoinHandle;
use toml::Value;

use hitman::{
    env::{
        find_available_requests, find_environments, find_root_dir, load_env,
        set_target, update_data,
    },
    extract::extract_variables,
    request::{build_client, do_request},
    substitute::{substitute, SubstituteError},
};

use super::{
    keymap::{mapkey, KeyMapping},
    output::OutputView,
    progress::Progress,
    prompt::{Prompt, PromptCommand},
    select::{RequestSelector, Select, SelectCommand, SelectItem},
    Component,
};

pub struct App {
    root_dir: PathBuf,
    request_selector: RequestSelector,
    output_view: OutputView,

    state: AppState,
}

pub enum AppState {
    Idle,

    PendingValue {
        key: String,
        pending_options: Vec<(String, String)>,

        pending_state: PendingState,
    },

    RunningRequest {
        handle: JoinHandle<Result<(String, String)>>,
        progress: Progress,
    },

    SelectTarget {
        component: Select<String>,
    },
}

pub enum PendingState {
    Prompt { component: Prompt },
    Select { component: Select<Value> },
}

impl App {
    pub fn new() -> Result<Self> {
        let root_dir = find_root_dir()?.context("No hitman.toml found")?;

        // FIXME: Need to live update requests

        let reqs = find_available_requests(&root_dir)?;
        let reqs: Vec<String> = reqs
            .iter()
            .filter_map(|p| p.to_str())
            .map(String::from)
            .collect();

        let request_selector = RequestSelector::new(&reqs);

        Ok(Self {
            root_dir,
            request_selector,
            output_view: OutputView::default(),
            state: AppState::Idle,
        })
    }

    pub async fn run<B>(&mut self, mut terminal: Terminal<B>) -> Result<()>
    where
        B: Backend,
    {
        let mut should_quit = false;
        while !should_quit {
            terminal.draw(|frame| self.render_ui(frame))?;

            if let AppState::RunningRequest { handle, .. } = &mut self.state {
                if handle.is_finished() {
                    let (request, response) = handle.await??;
                    self.output_view.update(request, response);
                    self.state = AppState::Idle;
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

        self.output_view.render_ui(frame, layout[1]);
    }

    fn render_left(&mut self, frame: &mut Frame, area: Rect) {
        self.request_selector.render_ui(frame, area);
    }

    fn render_status(&mut self, frame: &mut Frame, area: Rect) {
        let status_line = Paragraph::new("Ctrl+S: Select target")
            .style(Style::new().dark_gray());
        frame.render_widget(status_line, area);
    }

    fn render_popup(&mut self, frame: &mut Frame) {
        let area = frame.size();
        match &mut self.state {
            AppState::PendingValue { pending_state, .. } => match pending_state
            {
                PendingState::Prompt { component, .. } => {
                    let inner_area = area.inner(&Margin::new(42, 10));
                    component.render_ui(frame, inner_area);
                }
                PendingState::Select { component, .. } => {
                    let inner_area = area.inner(&Margin::new(42, 10));
                    component.render_ui(frame, inner_area);
                }
            },

            AppState::SelectTarget { component } => {
                let inner_area = area.inner(&Margin::new(42, 10));
                component.render_ui(frame, inner_area);
            }

            AppState::RunningRequest { progress, .. } => {
                progress.render_ui(frame, frame.size());
            }

            _ => (),
        }
    }

    async fn handle_events(&mut self) -> Result<bool> {
        // FIXME: Detect state changes, and avoid rerender
        // FIXME: No async (return "commands" instead)

        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            if let Event::Key(key) = event {
                if key.kind == KeyEventKind::Press {
                    match &mut self.state {
                        AppState::PendingValue { pending_state, .. } => {
                            match pending_state {
                                PendingState::Select { component } => {
                                    if let Some(command) =
                                        component.handle_event(&event)
                                    {
                                        match command {
                                            SelectCommand::Abort => {
                                                self.state = AppState::Idle;
                                            }
                                            SelectCommand::Accept(_) => {
                                                // FIXME: Get result here
                                                self.try_request()?;
                                            }
                                        }
                                    }
                                    return Ok(false);
                                }
                                PendingState::Prompt { component } => {
                                    if let Some(command) =
                                        component.handle_event(&event)
                                    {
                                        match command {
                                            PromptCommand::Abort => {
                                                self.state = AppState::Idle;
                                            }
                                            PromptCommand::Accept(_) => {
                                                // FIXME: Get result here
                                                self.try_request()?;
                                            }
                                        }
                                    }
                                    return Ok(false);
                                }
                            }
                        }

                        AppState::Idle => {
                            if let Some(command) =
                                self.request_selector.handle_event(&event)
                            {
                                match command {
                                    SelectCommand::Abort => (),
                                    SelectCommand::Accept(_) => {
                                        // FIXME: Get selection here
                                        self.try_request()?;
                                    }
                                }
                            }

                            self.output_view.handle_event(&event);

                            match mapkey(&event) {
                                KeyMapping::Abort => return Ok(true),
                                KeyMapping::SelectTarget => {
                                    let envs =
                                        find_environments(&self.root_dir)?;

                                    let component = Select::new(
                                        "Select environment".into(),
                                        envs,
                                    );

                                    self.state =
                                        AppState::SelectTarget { component };
                                }
                                _ => (),
                            }
                        }

                        AppState::RunningRequest { handle, .. } => {
                            if let KeyMapping::Abort = mapkey(&event) {
                                handle.abort();
                                self.state = AppState::Idle;
                            }
                        }

                        AppState::SelectTarget { component } => {
                            if let Some(command) =
                                component.handle_event(&event)
                            {
                                match command {
                                    SelectCommand::Abort => {
                                        self.state = AppState::Idle;
                                    }
                                    SelectCommand::Accept(s) => {
                                        set_target(&self.root_dir, &s)?;
                                        self.state = AppState::Idle;
                                    }
                                }
                            }
                        }
                    }
                }
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
                    PendingState::Prompt { component } => {
                        let value = component.value();
                        pending_options.push((key.clone(), value));
                    }
                    PendingState::Select { component } => {
                        if let Some(selected) = component.selected_item() {
                            let value = match selected {
                                Value::Table(t) => match t.get("value") {
                                    Some(Value::String(value)) => value.clone(),
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

            let path = PathBuf::from(file_path);
            let env = load_env(&root_dir, &path, &pending_options)?;

            match substitute(&read_to_string(path.clone())?, &env) {
                Ok(buf) => {
                    let root_dir = root_dir.clone();
                    let file_path = path.clone();
                    let handle = tokio::spawn(async move {
                        make_request(&buf, &root_dir, &file_path).await
                    });

                    self.state = AppState::RunningRequest {
                        handle,
                        progress: Progress,
                    };
                }
                Err(err) => match err {
                    SubstituteError::MultipleValuesFound { key, values } => {
                        let component = Select::new(
                            format!(
                                "Select substitution value for {{{{{}}}}}",
                                &key
                            ),
                            values.clone(),
                        );

                        self.state = AppState::PendingValue {
                            key,
                            pending_options,
                            pending_state: PendingState::Select { component },
                        };
                    }
                    SubstituteError::ValueNotFound { key, fallback } => {
                        let component =
                            Prompt::new(format!("Enter value for {key}"));
                        let component = if let Some(value) = fallback.clone() {
                            component.with_value(value)
                        } else {
                            component
                        };
                        self.state = AppState::PendingValue {
                            key,
                            pending_options,
                            pending_state: PendingState::Prompt { component },
                        };
                    }
                    e => bail!(e),
                },
            }
        }

        Ok(())
    }
}

async fn make_request(
    buf: &str,
    root_dir: &Path,
    file_path: &Path,
) -> Result<(String, String)> {
    let client = build_client()?;

    let mut request = String::new();
    for line in buf.lines() {
        writeln!(request, "> {}", line)?;
    }
    writeln!(request)?;

    let (res, _elapsed) = do_request(&client, buf).await?;

    let mut response = String::new();
    for (name, value) in res.headers() {
        writeln!(response, "< {}: {}", name, value.to_str()?)?;
    }
    writeln!(response)?;

    if let Ok(json) = res.json::<serde_json::Value>().await {
        writeln!(response, "{}", serde_json::to_string_pretty(&json)?)?;

        let options = vec![];
        let env = load_env(root_dir, file_path, &options)?;
        let vars = extract_variables(&json, &env)?;
        update_data(&vars)?;
    }

    Ok((request, response))
}

impl SelectItem for Value {
    fn text(&self) -> String {
        match self {
            Value::Table(t) => match t.get("name") {
                Some(Value::String(value)) => value.clone(),
                Some(value) => value.to_string(),
                None => t.to_string(),
            },
            other => other.to_string(),
        }
    }
}
