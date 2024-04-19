use std::{
    fmt::Write,
    fs::read_to_string,
    io,
    path::{Path, PathBuf},
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
    request::{build_client, do_request},
    substitute::{substitute, SubstituteError},
};

use super::{
    centered,
    keymap::{mapkey, KeyMapping},
    output::{HttpMessage, OutputView},
    progress::Progress,
    prompt::{Prompt, PromptIntent},
    select::{RequestSelector, Select, SelectIntent, SelectItem},
    Component,
};

pub trait Screen {
    fn enter(&self) -> io::Result<()>;
    fn leave(&self) -> io::Result<()>;
}

pub struct App {
    root_dir: PathBuf,
    target: String,
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

    NewRequestPrompt {
        prompt: Prompt,
    },

    RunningRequest {
        handle: JoinHandle<Result<(HttpMessage, HttpMessage)>>,
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

pub enum Intent {
    Quit,
    Update,
    PreviewRequest(Option<String>),
    PrepareRequest(String, Vec<(String, String)>),
    AskForValue {
        key: String,
        file_path: String,
        pending_options: Vec<(String, String)>,
        params: AskForValueParams,
    },
    SendRequest {
        file_path: String,
        prepared_request: String,
    },
    SelectTarget,
    AcceptSelectTarget(String),
    EditRequest,
    NewRequest,
    AcceptNewRequest(String),
    ShowError(String),

    ChangeState(AppState),
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
            output_view: OutputView::default(),
            state: AppState::Idle,
            error: None,
            should_quit: false,
        };

        app.populate_requests()?;
        Ok(app)
    }

    pub async fn run<B, S>(
        &mut self,
        mut terminal: Terminal<B>,
        mut screen: S,
    ) -> Result<()>
    where
        B: Backend,
        S: Screen,
    {
        screen.enter()?;

        while !self.should_quit {
            terminal.draw(|frame| self.render_ui(frame, frame.size()))?;

            let mut pending_intent = self.process_events().await?;
            while let Some(intent) = pending_intent {
                pending_intent =
                    match self.dispatch(intent, &mut terminal, &mut screen) {
                        Ok(it) => it,
                        Err(err) => Some(Intent::ShowError(err.to_string())),
                    };
            }
        }

        screen.leave()?;

        Ok(())
    }

    fn dispatch<B, S>(
        &mut self,
        intent: Intent,
        terminal: &mut Terminal<B>,
        screen: &mut S,
    ) -> Result<Option<Intent>>
    where
        B: Backend,
        S: Screen,
    {
        use Intent::*;

        Ok(match intent {
            Quit => {
                self.should_quit = true;
                None
            }
            Update => {
                self.populate_requests()?;

                Some(ChangeState(AppState::Idle))
            }
            ChangeState(state) => {
                self.error = None;
                self.state = state;
                None
            }
            PrepareRequest(file_path, options) => {
                self.try_request(file_path, options)?
            }
            PreviewRequest(file_path) => {
                self.preview_request(file_path)?;
                None
            }
            SendRequest {
                file_path,
                prepared_request,
            } => {
                let mut req = HttpMessage::default();
                for line in prepared_request.lines() {
                    writeln!(req.header, "> {}", line)?;
                }
                self.output_view.update(req, HttpMessage::default());
                self.send_request(file_path, prepared_request)?
            }
            AskForValue {
                key,
                file_path,
                pending_options,
                params,
            } => match params {
                AskForValueParams::Select { values } => {
                    let component = Select::new(
                        format!("Select substitution value for {{{{{key}}}}}",),
                        key.clone(),
                        values.clone(),
                    );

                    let state = AppState::PendingValue {
                        key,
                        file_path,
                        pending_options,
                        pending_state: PendingState::Select { component },
                    };
                    Some(Intent::ChangeState(state))
                }

                AskForValueParams::Prompt { fallback } => {
                    let component =
                        Prompt::new(format!("Enter value for {{{{{key}}}}}"))
                            .with_fallback(fallback);

                    let state = AppState::PendingValue {
                        key,
                        file_path,
                        pending_options,
                        pending_state: PendingState::Prompt { component },
                    };
                    Some(Intent::ChangeState(state))
                }
            },
            SelectTarget => {
                let envs = find_environments(&self.root_dir)?;
                let component =
                    Select::new("Select target".into(), "target".into(), envs);

                Some(ChangeState(AppState::SelectTarget { component }))
            }
            AcceptSelectTarget(s) => {
                set_target(&self.root_dir, &s)?;
                self.target = s;
                Some(ChangeState(AppState::Idle))
            }
            EditRequest => {
                let selected_item =
                    self.request_selector.selector.selected_item();
                if let Some(selected) = selected_item {
                    open_in_editor(selected, terminal, screen)?;
                }
                Some(PreviewRequest(selected_item.cloned()))
            }
            NewRequest => Some(ChangeState(AppState::NewRequestPrompt {
                prompt: Prompt::new("Name of request".into()),
            })),
            AcceptNewRequest(file_path) => {
                open_in_editor(&file_path, terminal, screen)?;
                Some(Update)
            }
            ShowError(err) => {
                self.error = Some(err);
                self.state = AppState::Idle;
                None
            }
        })
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
                    Ok(res) => match res {
                        Ok((request, response)) => {
                            // FIXME: Intent to update output
                            self.output_view.update(request, response);
                            Some(Intent::ChangeState(AppState::Idle))
                        }
                        Err(err) => {
                            self.output_view.show_error(err.to_string());
                            Some(Intent::ChangeState(AppState::Idle))
                        }
                    },
                    Err(err) => Some(Intent::ShowError(err.to_string())),
                });
            }
        }

        Ok(None)
    }

    fn try_request(
        &mut self,
        file_path: String,
        options: Vec<(String, String)>,
    ) -> Result<Option<Intent>> {
        let root_dir = self.root_dir.clone();

        let path = PathBuf::from(file_path.clone());
        let env = load_env(&root_dir, &path, &options)?;

        let intent = match substitute(&read_to_string(path.clone())?, &env) {
            Ok(prepared_request) => Some(Intent::SendRequest {
                file_path,
                prepared_request,
            }),
            Err(err) => match err {
                SubstituteError::MultipleValuesFound { key, values } => {
                    Some(Intent::AskForValue {
                        key,
                        file_path,
                        pending_options: options,
                        params: AskForValueParams::Select { values },
                    })
                }
                SubstituteError::ValueNotFound { key, fallback } => {
                    Some(Intent::AskForValue {
                        key,
                        file_path,
                        pending_options: options,
                        params: AskForValueParams::Prompt { fallback },
                    })
                }
                other_err => Some(Intent::ShowError(other_err.to_string())),
            },
        };

        Ok(intent)
    }

    fn preview_request(&mut self, file_path: Option<String>) -> Result<()> {
        if let Some(file_path) = file_path {
            let path = PathBuf::from(file_path.clone());

            // TODO: Highlight substitutions and current values

            let f = read_to_string(path.clone())?;
            let mut req = HttpMessage::default();
            for line in f.lines() {
                writeln!(req.header, "{}", line)?;
            }
            self.output_view.update(req, HttpMessage::default());
        } else {
            self.output_view.reset();
        }

        Ok(())
    }

    fn send_request(
        &mut self,
        file_path: String,
        prepared_request: String,
    ) -> Result<Option<Intent>> {
        let root_dir = self.root_dir.clone();
        let file_path = PathBuf::from(file_path);

        let handle = tokio::spawn(async move {
            make_request(&prepared_request, &root_dir, &file_path).await
        });

        let state = AppState::RunningRequest {
            handle,
            progress: Progress,
        };

        Ok(Some(Intent::ChangeState(state)))
    }
}

fn open_in_editor<B, S>(
    file_path: &String,
    terminal: &mut Terminal<B>,
    screen: &mut S,
) -> Result<(), anyhow::Error>
where
    B: Backend,
    S: Screen,
{
    let editor = std::env::var("EDITOR")
        .context("EDITOR environment variable not set")?;
    screen.leave()?;
    let _ = std::process::Command::new(editor).arg(file_path).status();
    screen.enter()?;
    terminal.clear()?;
    Ok(())
}

impl Component for App {
    type Intent = Intent;

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

    fn handle_event(&mut self, event: &Event) -> Option<Self::Intent> {
        use Intent::*;

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
                                if let Some(intent) =
                                    component.handle_event(&event)
                                {
                                    match intent {
                                        SelectIntent::Abort => {
                                            return Some(ChangeState(
                                                AppState::Idle,
                                            ));
                                        }
                                        SelectIntent::Accept(selected) => {
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
                                            return Some(PrepareRequest(
                                                file_path.clone(),
                                                pending_options.clone(),
                                            ));
                                        }
                                        SelectIntent::Change(_) => (),
                                    }
                                }
                                return None;
                            }
                            PendingState::Prompt { component } => {
                                if let Some(intent) =
                                    component.handle_event(&event)
                                {
                                    match intent {
                                        PromptIntent::Abort => {
                                            return Some(ChangeState(
                                                AppState::Idle,
                                            ));
                                        }
                                        PromptIntent::Accept(value) => {
                                            pending_options
                                                .push((key.clone(), value));
                                            return Some(PrepareRequest(
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
                        if let Some(intent) =
                            self.request_selector.handle_event(&event)
                        {
                            match intent {
                                SelectIntent::Abort => (),
                                SelectIntent::Accept(file_path) => {
                                    return Some(PrepareRequest(
                                        file_path,
                                        Vec::new(),
                                    ));
                                }
                                SelectIntent::Change(file_path) => {
                                    return Some(PreviewRequest(file_path));
                                }
                            }
                        }

                        self.output_view.handle_event(&event);

                        match mapkey(&event) {
                            KeyMapping::Editor => {
                                return Some(Intent::EditRequest)
                            }
                            KeyMapping::New => return Some(Intent::NewRequest),
                            KeyMapping::Abort => return Some(Intent::Quit),
                            KeyMapping::SelectTarget => {
                                return Some(Intent::SelectTarget);
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

                    AppState::NewRequestPrompt { prompt } => {
                        if let Some(intent) = prompt.handle_event(&event) {
                            match intent {
                                PromptIntent::Abort => {
                                    return Some(ChangeState(AppState::Idle));
                                }
                                PromptIntent::Accept(s) => {
                                    return Some(AcceptNewRequest(s));
                                }
                            }
                        }
                    }

                    AppState::SelectTarget { component } => {
                        if let Some(intent) = component.handle_event(&event) {
                            match intent {
                                SelectIntent::Abort => {
                                    return Some(ChangeState(AppState::Idle));
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
        let area = area.inner(&Margin::new(1, 0));

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

        let status_line = match &self.error {
            Some(msg) => Paragraph::new(msg.clone()).red().reversed(),
            None => Paragraph::new(
                "Ctrl+S: Select target, Ctrl+E: Edit selected request, Ctrl+R: New request",
            )
            .dark_gray(),
        };

        frame.render_widget(status_line, layout[2]);
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

            AppState::NewRequestPrompt { prompt } => {
                let inner_area = centered(area, 30, 30);
                prompt.render_ui(frame, inner_area);
            }

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
) -> Result<(HttpMessage, HttpMessage)> {
    let client = build_client()?;

    let mut request = HttpMessage::default();
    for line in buf.lines() {
        writeln!(request.header, "> {}", line)?;
    }
    writeln!(request.header)?;

    let (res, _elapsed) = do_request(&client, buf).await?;

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
