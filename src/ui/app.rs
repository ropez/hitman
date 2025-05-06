use std::{
    collections::HashMap,
    fmt::Write,
    fs::read_to_string,
    io,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::Stylize,
    widgets::Paragraph,
    Frame, Terminal,
};
use tokio::task::JoinHandle;
use toml::Value;

use hitman::{
    env::{
        find_available_requests, find_environments, find_root_dir, get_target,
        load_env, set_target, update_data,
    },
    extract::extract_variables,
    request::{build_client, do_request, HitmanRequest},
    resolve::{resolve_path, Resolved},
    scope::Replacement,
    substitute::{
        prepare_request,
        Substitution::{Complete, ValueMissing},
    },
};

use super::{
    centered,
    datepicker::DatePicker,
    help::Help,
    keymap::{mapkey, KeyMapping},
    output::{HttpMessage, HttpRequestMessage, OutputView},
    output::{HttpRequestInfo, RequestStatus},
    progress::Progress,
    prompt::SimplePrompt,
    select::{
        PromptSelectItem, RequestSelector, Select, SelectIntent, SelectItem,
    },
    Component, InteractiveComponent, PromptComponent, PromptIntent,
};

pub trait Screen {
    type B: Backend;

    fn enter(&self) -> io::Result<()>;
    fn leave(&self) -> io::Result<()>;
    fn terminal(&mut self) -> &mut Terminal<Self::B>;
}

pub struct App {
    root_dir: PathBuf,
    target: String,
    request_selector: RequestSelector,
    output_view: OutputView,

    state: AppState,
    error: Option<String>,
    should_quit: bool,
    vsplit: u16,
}

pub enum AppState {
    Idle,

    ShowHelp,

    PendingValue {
        file_path: String,
        key: String,
        pending_vars: HashMap<String, String>,
        component: Box<dyn PromptComponent>,
    },

    NewRequestPrompt {
        prompt: SimplePrompt,
    },

    RunningRequest {
        handle: JoinHandle<HttpRequestInfo>,
        progress: Progress,
    },

    SelectTarget {
        component: Select<String>,
    },
}

pub enum Intent {
    Quit,
    Abort,
    ShowHelp,
    Update(Option<String>),
    PreviewRequest(Option<String>),
    PrepareRequest {
        file_path: String,
        vars: HashMap<String, String>,
    },
    AskForValue {
        key: String,
        file_path: String,
        pending_vars: HashMap<String, String>,
        params: AskForValueParams,
    },
    SendRequest {
        resolved: Resolved,
        prepared_request: HitmanRequest,
    },
    ShowResult(HttpRequestInfo),
    SelectTarget,
    AcceptSelectTarget(String),
    EditRequest,
    NewRequest,
    AcceptNewRequest(String),
    ShowError(String),
}

pub enum AskForValueParams {
    Prompt { fallback: Option<String> },
    Select { values: Vec<Value> },
}

impl App {
    pub fn new() -> Result<Self> {
        let root_dir = find_root_dir()?.context("No hitman.toml found")?;

        let target = get_target(&root_dir);

        let mut app = Self {
            root_dir,
            target,
            request_selector: RequestSelector::new(),
            output_view: OutputView::new(),
            state: AppState::Idle,
            error: None,
            should_quit: false,
            vsplit: 60,
        };

        app.populate_requests()?;
        Ok(app)
    }

    pub async fn run<S>(&mut self, mut screen: S) -> Result<()>
    where
        S: Screen,
    {
        screen.enter()?;

        while !self.should_quit {
            screen
                .terminal()
                .draw(|frame| self.render_ui(frame, frame.area()))?;

            let mut pending_intent = self.process_events().await?;
            while let Some(intent) = pending_intent {
                pending_intent = match self.dispatch(intent, &mut screen) {
                    Ok(it) => it,
                    Err(err) => Some(Intent::ShowError(err.to_string())),
                };
            }
        }

        screen.leave()?;

        Ok(())
    }

    fn dispatch<S>(
        &mut self,
        intent: Intent,
        screen: &mut S,
    ) -> Result<Option<Intent>>
    where
        S: Screen,
    {
        use Intent::*;

        match intent {
            Quit => {
                self.should_quit = true;
            }
            Abort => {
                self.set_state(AppState::Idle);
            }
            ShowHelp => {
                self.set_state(AppState::ShowHelp);
            }
            Update(selected) => {
                self.populate_requests()?;

                return Ok(Some(PreviewRequest(selected)));
            }
            PrepareRequest { file_path, vars } => {
                return self.try_request(file_path, vars);
            }
            PreviewRequest(file_path) => {
                self.preview_request(file_path)?;
                self.set_state(AppState::Idle);
            }
            SendRequest {
                resolved,
                prepared_request,
            } => {
                let req = HttpRequestMessage(prepared_request.clone());
                let info = HttpRequestInfo::new(req, RequestStatus::Running);
                self.output_view.show_request(info);
                self.send_request(resolved, prepared_request);
            }
            AskForValue {
                key,
                file_path,
                pending_vars,
                params,
            } => {
                let component: Box<dyn PromptComponent> = match params {
                    AskForValueParams::Select { values } => {
                        Box::new(Select::new(
                            format!(
                                "Select substitution value for {{{{{key}}}}}",
                            ),
                            key.clone(),
                            values,
                        ))
                    }

                    AskForValueParams::Prompt { fallback } => {
                        if key.ends_with("_date") || key.ends_with("Date") {
                            Box::new(
                                DatePicker::new(format!(
                                    "Select {{{{{key}}}}}"
                                ))
                                .with_fallback(fallback),
                            )
                        } else {
                            Box::new(
                                SimplePrompt::new(format!(
                                    "Enter value for {{{{{key}}}}}"
                                ))
                                .with_fallback(fallback),
                            )
                        }
                    }
                };

                self.set_state(AppState::PendingValue {
                    key,
                    file_path,
                    pending_vars,
                    component,
                });
            }
            ShowResult(info) => {
                self.output_view.show_request(info);
                self.set_state(AppState::Idle);
            }
            SelectTarget => {
                let envs = find_environments(&self.root_dir)?;
                let component =
                    Select::new("Select target".into(), "target".into(), envs);

                self.set_state(AppState::SelectTarget { component });
            }
            AcceptSelectTarget(s) => {
                set_target(&self.root_dir, &s)?;
                self.target = s;
                self.set_state(AppState::Idle);
            }
            EditRequest => {
                let selected_item =
                    self.request_selector.selector.selected_item();
                if let Some(selected) = selected_item {
                    open_in_editor(selected, screen)?;
                }
                return Ok(Some(PreviewRequest(selected_item.cloned())));
            }
            NewRequest => {
                self.set_state(AppState::NewRequestPrompt {
                    prompt: SimplePrompt::new("Name of request".into()),
                });
            }
            AcceptNewRequest(file_path) => {
                open_in_editor(&file_path, screen)?;
                return Ok(Some(Update(Some(file_path))));
            }
            ShowError(err) => {
                self.error = Some(err);
                self.state = AppState::Idle;
            }
        }

        Ok(None)
    }

    fn set_state(&mut self, state: AppState) {
        self.error = None;
        self.state = state;
    }

    fn populate_requests(&mut self) -> Result<()> {
        let reqs = find_available_requests(&self.root_dir)?;
        let reqs: Vec<String> = reqs
            .iter()
            .filter_map(|p| p.to_str())
            .map(String::from)
            .collect();
        self.request_selector.populate(reqs);

        Ok(())
    }

    async fn process_events(&mut self) -> Result<Option<Intent>> {
        // Don't waste so much CPU when idle
        let poll_timeout = match self.state {
            AppState::RunningRequest { .. } => Duration::from_millis(50),
            _ => Duration::from_secs(1),
        };

        if event::poll(poll_timeout)? {
            let event = event::read()?;

            return Ok(self.handle_event(&event));
        }

        if let AppState::RunningRequest { handle, .. } = &mut self.state {
            if handle.is_finished() {
                return Ok(match handle.await {
                    Ok(res) => Some(Intent::ShowResult(res)),
                    Err(err) => Some(Intent::ShowError(err.to_string())),
                });
            }
        }

        Ok(None)
    }

    fn try_request(
        &self,
        file_path: String,
        mut vars: HashMap<String, String>,
    ) -> Result<Option<Intent>> {
        let root_dir = self.root_dir.clone();

        // FIXME Call resolve_path and load_env once, and keep result in state

        let path = PathBuf::from(file_path.clone());

        let resolved = resolve_path(&path)?;
        let intent = match prepare_request(&resolved, &vars)? {
            Complete(prepared_request) => Some(Intent::SendRequest {
                resolved,
                prepared_request,
            }),
            ValueMissing { key, fallback } => {
                let scope = load_env(&root_dir, &self.target, &resolved, &[])?;

                match scope.lookup(&key)? {
                    Replacement::Value(value) => {
                        vars.insert(key, value);
                        Some(Intent::PrepareRequest { file_path, vars })
                    }
                    Replacement::MultipleValuesFound { key, values } => {
                        Some(Intent::AskForValue {
                            key,
                            file_path,
                            pending_vars: vars,
                            params: AskForValueParams::Select { values },
                        })
                    }
                    Replacement::ValueNotFound { key } => {
                        Some(Intent::AskForValue {
                            key,
                            file_path,
                            pending_vars: vars,
                            params: AskForValueParams::Prompt { fallback },
                        })
                    }
                }
            }
        };

        Ok(intent)
    }

    fn preview_request(&mut self, file_path: Option<String>) -> Result<()> {
        if let Some(file_path) = file_path {
            let path = PathBuf::from(file_path.clone());

            // TODO: Preview separate from output?
            // TODO: Highlight substitutions and current values

            let f = read_to_string(path)?;

            self.request_selector.try_select(&file_path);

            self.output_view.show_preview(f);
        } else {
            self.output_view.reset();
        }

        Ok(())
    }

    fn send_request(
        &mut self,
        resolved: Resolved,
        prepared_request: HitmanRequest,
    ) {
        let root_dir = self.root_dir.clone();

        let target = self.target.clone();
        let handle = tokio::spawn(async move {
            make_request(prepared_request, target, &root_dir, &resolved).await
        });

        let state = AppState::RunningRequest {
            handle,
            progress: Progress,
        };
        self.set_state(state);
    }
}

fn open_in_editor<S>(
    file_path: &String,
    screen: &mut S,
) -> Result<(), anyhow::Error>
where
    S: Screen,
{
    let editor = std::env::var("EDITOR")
        .context("EDITOR environment variable not set")?;
    screen.leave()?;
    let _ = Command::new(editor).arg(file_path).status();
    screen.enter()?;
    screen.terminal().clear()?;
    Ok(())
}

impl Component for App {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let layout = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Length(1)],
        )
        .split(area);

        self.render_main(frame, layout[0]);
        self.render_status(frame, layout[1]);
        self.render_help(frame);
        self.render_popup(frame);
    }
}

impl InteractiveComponent for App {
    type Intent = Intent;

    fn handle_event(&mut self, event: &Event) -> Option<Self::Intent> {
        use Intent::*;

        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match &mut self.state {
                    AppState::PendingValue {
                        key,
                        file_path,
                        pending_vars,
                        component,
                        ..
                    } => {
                        if let Some(intent) = component.handle_prompt(event) {
                            match intent {
                                PromptIntent::Abort => {
                                    return Some(Abort);
                                }
                                PromptIntent::Accept(value) => {
                                    pending_vars.insert(key.clone(), value);
                                    return Some(PrepareRequest {
                                        file_path: file_path.clone(),
                                        vars: pending_vars.clone(),
                                    });
                                }
                            }
                        }
                        return None;
                    }
                    AppState::ShowHelp => match mapkey(event) {
                        KeyMapping::Abort
                        | KeyMapping::Accept
                        | KeyMapping::ToggleHelp => {
                            return Some(Intent::Abort);
                        }
                        _ => (),
                    },
                    AppState::Idle => {
                        if let Some(intent) =
                            self.request_selector.handle_event(event)
                        {
                            match intent {
                                SelectIntent::Abort => (),
                                SelectIntent::Accept(file_path) => {
                                    return Some(PrepareRequest {
                                        file_path,
                                        vars: HashMap::new(),
                                    });
                                }
                                SelectIntent::Change(file_path) => {
                                    return Some(PreviewRequest(file_path));
                                }
                            }
                        }

                        self.output_view.handle_event(event);

                        match mapkey(event) {
                            KeyMapping::Editor => {
                                return Some(Intent::EditRequest)
                            }
                            KeyMapping::Reload => {
                                let selected_item = self
                                    .request_selector
                                    .selector
                                    .selected_item();
                                return Some(Intent::Update(
                                    selected_item.cloned(),
                                ));
                            }
                            KeyMapping::New => return Some(Intent::NewRequest),
                            KeyMapping::Abort => return Some(Intent::Quit),
                            KeyMapping::SelectTarget => {
                                return Some(Intent::SelectTarget);
                            }
                            KeyMapping::IncreaseWidth => {
                                self.vsplit += 5;
                            }
                            KeyMapping::DecreaseWitdh => {
                                if self.vsplit <= 20 {
                                    self.vsplit = 15;
                                } else {
                                    self.vsplit -= 5;
                                }
                            }
                            KeyMapping::ToggleHelp => {
                                return Some(Intent::ShowHelp);
                            }
                            _ => (),
                        }
                    }

                    AppState::RunningRequest { handle, .. } => {
                        if let KeyMapping::Abort = mapkey(event) {
                            handle.abort();
                            return Some(Abort);
                        }
                    }

                    AppState::NewRequestPrompt { prompt } => {
                        if let Some(intent) = prompt.handle_prompt(event) {
                            match intent {
                                PromptIntent::Abort => {
                                    return Some(Abort);
                                }
                                PromptIntent::Accept(s) => {
                                    return Some(AcceptNewRequest(s));
                                }
                            }
                        }
                    }

                    AppState::SelectTarget { component } => {
                        if let Some(intent) = component.handle_event(event) {
                            match intent {
                                SelectIntent::Abort => {
                                    return Some(Abort);
                                }
                                SelectIntent::Accept(s) => {
                                    return Some(AcceptSelectTarget(s));
                                }
                                SelectIntent::Change(_) => (),
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
            [Constraint::Max(self.vsplit), Constraint::Min(1)],
        )
        .split(area);

        self.render_left(frame, layout[0]);

        self.output_view.render_ui(frame, layout[1]);
    }

    fn render_left(&mut self, frame: &mut Frame, area: Rect) {
        self.request_selector.render_ui(frame, area);
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let area = area.inner(Margin::new(1, 0));

        let layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(24),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .split(area);

        frame.render_widget(
            Paragraph::new(self.target.as_str())
                .centered()
                .black()
                .on_cyan(),
            layout[0],
        );

        // FIXME: <Ctrl+?> opens key mapping window
        let status_line = self.error.as_ref().map_or_else(
            || {
                Paragraph::new(
                    "Ctrl+S: Select target, ? - Show keyboard shortcuts",
                )
                .dark_gray()
            },
            |msg| Paragraph::new(msg.clone()).red().reversed(),
        );

        frame.render_widget(status_line, layout[2]);
    }

    fn render_help(&self, frame: &mut Frame) {
        if matches!(self.state, AppState::ShowHelp) {
            Help.render_ui(frame, frame.area());
        }
    }

    fn render_popup(&mut self, frame: &mut Frame) {
        let area = frame.area();
        match &mut self.state {
            AppState::PendingValue { component, .. } => {
                let inner_area = centered(area, 48, 30);
                component.render_ui(frame, inner_area);
            }

            AppState::NewRequestPrompt { prompt } => {
                let inner_area = centered(area, 48, 30);
                prompt.render_ui(frame, inner_area);
            }

            AppState::SelectTarget { component } => {
                let inner_area = centered(area, 30, 20);
                component.render_ui(frame, inner_area);
            }

            AppState::RunningRequest { progress, .. } => {
                progress.render_ui(frame, frame.area());
            }

            _ => (),
        }
    }
}

async fn make_request(
    req: HitmanRequest,
    target: String,
    root_dir: &Path,
    resolved: &Resolved,
) -> HttpRequestInfo {
    let request = HttpRequestMessage(req.clone());
    let status = match do_make_request(req, target, root_dir, resolved).await {
        Ok((response, elapsed)) => {
            RequestStatus::Complete { response, elapsed }
        }
        Err(err) => RequestStatus::Failed {
            error: err.to_string(),
        },
    };

    HttpRequestInfo::new(request, status)
}

// FIXME: DRY request.rs
async fn do_make_request(
    req: HitmanRequest,
    target: String,
    root_dir: &Path,
    resolved: &Resolved,
) -> Result<(HttpMessage, Duration)> {
    let client = build_client()?;
    let options = vec![];
    let scope = load_env(root_dir, &target, resolved, &options)?;

    let (res, elapsed) = do_request(&client, &req).await?;

    let mut response = HttpMessage::default();
    writeln!(
        response.header,
        "< HTTP/1.1 {} {}",
        res.status().as_u16(),
        res.status().canonical_reason().unwrap_or("")
    )?;
    for (name, value) in res.headers() {
        writeln!(response.header, "< {}: {}", name, value.to_str()?)?;
    }
    writeln!(response.header)?;

    if let Ok(json) = res.json::<serde_json::Value>().await {
        writeln!(response.body, "{}", serde_json::to_string_pretty(&json)?)?;

        let vars = extract_variables(&json, &scope)?;
        update_data(&vars)?;
    }

    Ok((response, elapsed))
}

impl SelectItem for Value {
    fn text(&self) -> String {
        match self {
            Self::Table(t) => match t.get("name") {
                Some(Self::String(value)) => value.clone(),
                Some(value) => value.to_string(),
                None => t.to_string(),
            },
            other => other.to_string(),
        }
    }
}

impl PromptSelectItem for Value {
    fn to_value(&self) -> String {
        match self {
            Self::Table(t) => match t.get("value") {
                Some(Self::String(value)) => value.clone(),
                Some(value) => value.to_string(),
                _ => t.to_string(),
            },
            other => other.to_string(),
        }
    }
}
