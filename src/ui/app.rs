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
    layout::{Constraint, Direction, Layout, Rect},
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
    centered,
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
    error: Option<String>,
    should_quit: bool,
}

pub enum AppState {
    Idle,

    PendingValue {
        file_path: String,
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

pub enum AppAction {
    Quit,
    TryRequest(String, Vec<(String, String)>),
    ChangeState(AppState),
    SelectTarget,
    AcceptSelectTarget(String),
    ShowError(String),
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
            error: None,
            should_quit: false,
        })
    }

    pub async fn run<B>(&mut self, mut terminal: Terminal<B>) -> Result<()>
    where
        B: Backend,
    {
        while !self.should_quit {
            terminal.draw(|frame| self.render_ui(frame, frame.size()))?;

            if let AppState::RunningRequest { handle, .. } = &mut self.state {
                if handle.is_finished() {
                    let (request, response) = handle.await??;
                    self.output_view.update(request, response);
                    self.state = AppState::Idle;
                }
            }

            let mut pending_action = self.handle_events()?;
            while let Some(action) = pending_action {
                pending_action = match self.dispatch(action) {
                    Ok(it) => it,
                    Err(err) => Some(AppAction::ShowError(err.to_string())),
                };
            }
        }

        Ok(())
    }

    fn dispatch(&mut self, action: AppAction) -> Result<Option<AppAction>> {
        use AppAction::*;

        match action {
            Quit => {
                self.should_quit = true;
            }
            ChangeState(state) => {
                self.error = None;
                self.state = state;
            }
            TryRequest(file_path, options) => {
                let res = self.try_request(file_path, options)?;
                return Ok(res);
            }
            SelectTarget => {
                let envs = find_environments(&self.root_dir)?;
                let component =
                    Select::new("Select target".into(), "target".into(), envs);

                return Ok(Some(ChangeState(AppState::SelectTarget {
                    component,
                })));
            }
            AcceptSelectTarget(s) => {
                set_target(&self.root_dir, &s)?;
                return Ok(Some(ChangeState(AppState::Idle)));
            }
            ShowError(err) => {
                self.error = Some(err);
            }
        }

        Ok(None)
    }

    fn handle_events(&mut self) -> Result<Option<AppAction>> {
        // FIXME: Detect state changes, and avoid rerender

        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;

            return Ok(self.handle_event(&event));
        }

        Ok(None)
    }

    fn try_request(
        &mut self,
        file_path: String,
        options: Vec<(String, String)>,
    ) -> Result<Option<AppAction>> {
        let root_dir = self.root_dir.clone();

        let path = PathBuf::from(file_path.clone());
        let env = load_env(&root_dir, &path, &options)?;

        match substitute(&read_to_string(path.clone())?, &env) {
            Ok(buf) => {
                let root_dir = root_dir.clone();
                let file_path = path.clone();
                let handle = tokio::spawn(async move {
                    make_request(&buf, &root_dir, &file_path).await
                });

                let state = AppState::RunningRequest {
                    handle,
                    progress: Progress,
                };
                return Ok(Some(AppAction::ChangeState(state)));
            }
            Err(err) => match err {
                // FIXME: Return Actions here too ("Prompt", "Select")
                SubstituteError::MultipleValuesFound { key, values } => {
                    let component = Select::new(
                        format!("Select substitution value for {{{{{key}}}}}",),
                        key.clone(),
                        values.clone(),
                    );

                    let state = AppState::PendingValue {
                        key,
                        file_path,
                        pending_options: options,
                        pending_state: PendingState::Select { component },
                    };
                    return Ok(Some(AppAction::ChangeState(state)));
                }
                SubstituteError::ValueNotFound { key, fallback } => {
                    let component =
                        Prompt::new(format!("Enter value for {key}"))
                            .with_fallback(fallback);

                    let state = AppState::PendingValue {
                        key,
                        file_path,
                        pending_options: options,
                        pending_state: PendingState::Prompt { component },
                    };
                    return Ok(Some(AppAction::ChangeState(state)));
                }
                e => bail!(e),
            },
        }
    }
}

impl Component for App {
    type Command = AppAction;

    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let layout = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Length(1)],
        )
        .split(area);

        self.render_main(frame, layout[0]);
        self.render_status(frame, layout[1]);
        self.render_popup(frame);
    }

    fn handle_event(&mut self, event: &Event) -> Option<Self::Command> {
        use AppAction::*;

        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match &mut self.state {
                    AppState::PendingValue {
                        key,
                        file_path,
                        pending_options,
                        pending_state,
                        ..
                    } => {
                        match pending_state {
                            PendingState::Select { component } => {
                                if let Some(command) =
                                    component.handle_event(&event)
                                {
                                    match command {
                                        SelectCommand::Abort => {
                                            return Some(ChangeState(
                                                AppState::Idle,
                                            ));
                                        }
                                        SelectCommand::Accept(selected) => {
                                            let value = match selected {
                                                Value::Table(t) => match t.get("value") {
                                                    Some(Value::String(value)) => value.clone(),
                                                    Some(value) => value.to_string(),
                                                    _ => return Some(ShowError(
                                                    format!("Replacement not found: {key}"),
)),
                                                },
                                                other => other.to_string(),
                                            };

                                            // XXX Maybe we can simplify this by emitting
                                            // the pending_state
                                            pending_options
                                                .push((key.clone(), value));
                                            return Some(TryRequest(
                                                file_path.clone(),
                                                pending_options.clone(),
                                            ));
                                        }
                                    }
                                }
                                return None;
                            }
                            PendingState::Prompt { component } => {
                                if let Some(command) =
                                    component.handle_event(&event)
                                {
                                    match command {
                                        PromptCommand::Abort => {
                                            return Some(ChangeState(
                                                AppState::Idle,
                                            ));
                                        }
                                        PromptCommand::Accept(value) => {
                                            pending_options
                                                .push((key.clone(), value));
                                            return Some(TryRequest(
                                                file_path.clone(),
                                                pending_options.clone(),
                                            ));
                                        }
                                    }
                                }
                                return None;
                            }
                        }
                    }

                    AppState::Idle => {
                        if let Some(command) =
                            self.request_selector.handle_event(&event)
                        {
                            match command {
                                SelectCommand::Abort => (),
                                SelectCommand::Accept(file_path) => {
                                    // FIXME: Get selection here
                                    return Some(TryRequest(
                                        file_path,
                                        Vec::new(),
                                    ));
                                }
                            }
                        }

                        self.output_view.handle_event(&event);

                        match mapkey(&event) {
                            KeyMapping::Abort => return Some(AppAction::Quit),
                            KeyMapping::SelectTarget => {
                                return Some(AppAction::SelectTarget);
                            }
                            _ => (),
                        }
                    }

                    AppState::RunningRequest { handle, .. } => {
                        if let KeyMapping::Abort = mapkey(&event) {
                            handle.abort();
                            return Some(ChangeState(AppState::Idle));
                        }
                    }

                    AppState::SelectTarget { component } => {
                        if let Some(command) = component.handle_event(&event) {
                            match command {
                                SelectCommand::Abort => {
                                    return Some(ChangeState(AppState::Idle));
                                }
                                SelectCommand::Accept(s) => {
                                    return Some(AcceptSelectTarget(s));
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
}

impl App {
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
        let status_line = match &self.error {
            Some(msg) => Paragraph::new(msg.clone()).white().on_red(),
            None => Paragraph::new("Ctrl+S: Select target")
                .style(Style::new().dark_gray()),
        };

        frame.render_widget(status_line, area);
    }

    fn render_popup(&mut self, frame: &mut Frame) {
        let area = frame.size();
        match &mut self.state {
            AppState::PendingValue { pending_state, .. } => match pending_state
            {
                PendingState::Prompt { component, .. } => {
                    let inner_area = centered(area, 30, 30);
                    component.render_ui(frame, inner_area);
                }
                PendingState::Select { component, .. } => {
                    let inner_area = centered(area, 60, 20);
                    component.render_ui(frame, inner_area);
                }
            },

            AppState::SelectTarget { component } => {
                let inner_area = centered(area, 30, 20);
                component.render_ui(frame, inner_area);
            }

            AppState::RunningRequest { progress, .. } => {
                progress.render_ui(frame, frame.size());
            }

            _ => (),
        }
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
