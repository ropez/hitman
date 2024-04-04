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
    widgets::{Block, BorderType, Clear, List, Paragraph, StatefulWidget, Widget},
    Terminal,
};
use serde_json::Value;

use hitman::{
    env::{find_available_requests, find_root_dir, load_env, update_data},
    extract::extract_variables,
    request::{build_client, do_request},
    substitute::{substitute, PendingSubstitution, SubstituteError, SubstitutionType},
};

use super::{
    output::{OutputView, OutputViewState},
    select::{RequestSelector, RequestSelectorState, Select, SelectState},
};

pub struct App {
    root_dir: PathBuf,
    selector_state: RequestSelectorState,
    output_state: OutputViewState,
    environment_state: Option<bool>,
    substitution_state: Option<SubstituteState>,
    search: String,
}

pub struct SubstituteState {
    pending_substitution: PendingSubstitution,
    pending_options: Vec<(String, String)>,
    select_state: SelectState,
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
            substitution_state: None,
            search: String::new(),
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
            .block(Block::bordered().border_set(symbols::border::Set {
                top_left: symbols::border::PLAIN.vertical_right,
                top_right: symbols::border::PLAIN.vertical_left,
                ..symbols::border::PLAIN
            })),
            layout[1],
            buf,
        );
    }

    fn render_status(&mut self, area: Rect, buf: &mut Buffer) {
        let status_line = Paragraph::new("S: Select environment").style(Style::new().dark_gray());
        status_line.render(area, buf);
    }

    fn render_popup(&mut self, area: Rect, buf: &mut Buffer) {
        if let Some(_state) = self.environment_state {
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

        if let Some(ref mut state) = &mut self.substitution_state {
            let key = &state.pending_substitution.key;
            match &state.pending_substitution.substitution_type {
                SubstitutionType::Prompt { fallback } => {
                    // TODO: Check out using tui-input

                    let inner_area = area.inner(&Margin::new(42, 10));
                    Widget::render(Clear, inner_area, buf);
                    Widget::render(
                        Block::bordered().title(format!("Enter value for {key}")),
                        inner_area,
                        buf,
                    );
                }
                SubstitutionType::Select { values } => {
                    let values = values.iter().map(|v| match v {
                        toml::Value::Table(t) => match t.get("name") {
                            Some(toml::Value::String(value)) => value.clone(),
                            Some(value) => value.to_string(),
                            None => t.to_string(),
                        },
                        other => other.to_string(),
                    });

                    // XXX Code smell!
                    state.select_state.set_items(values.collect());

                    let select = Select::default().block(
                        Block::bordered()
                            .title(format!("Select substitution value for {{{{{key}}}}}")),
                    );

                    let inner_area = area.inner(&Margin::new(42, 10));
                    Widget::render(Clear, inner_area, buf);
                    StatefulWidget::render(select, inner_area, buf, &mut state.select_state);
                }
            }
        }
    }

    async fn handle_events(&mut self) -> Result<bool> {
        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Some(ref mut state) = &mut self.substitution_state {
                        if state.select_state.handle_event(event) {
                            return Ok(false);
                        }
                    }

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
                                if let Some(file_path) = self.selector_state.selected_path() {
                                    let path = PathBuf::try_from(file_path)?;

                                    let root_dir = self.root_dir.clone();

                                    let mut pending_options = self
                                        .substitution_state
                                        .as_ref()
                                        .map(|s| s.pending_options.clone())
                                        .unwrap_or(Vec::new());

                                    if let Some(ref state) = &self.substitution_state {
                                        let key = state.pending_substitution.key.clone();
                                        match &state.pending_substitution.substitution_type {
                                            SubstitutionType::Prompt { fallback } => {
                                                let value = fallback.as_deref().unwrap_or("");
                                                pending_options.push((key, value.into()));
                                            }
                                            SubstitutionType::Select { values } => {
                                                if let Some(selected) =
                                                    state.select_state.selected()
                                                {
                                                    let value = match &values[selected] {
                                                        toml::Value::Table(t) => {
                                                            match t.get("value") {
                                                                Some(toml::Value::String(
                                                                    value,
                                                                )) => value.clone(),
                                                                Some(value) => value.to_string(),
                                                                _ => bail!(
                                                                    "Replacement not found: {key}"
                                                                ),
                                                            }
                                                        }
                                                        other => other.to_string(),
                                                    };

                                                    pending_options.push((key, value));
                                                }
                                            }
                                        }
                                    }

                                    self.substitution_state = None;
                                    let env = load_env(&root_dir, &path, &pending_options)?;

                                    match substitute(&read_to_string(path)?, &env) {
                                        Ok(buf) => {
                                            let handle =
                                                tokio::spawn(
                                                    async move { make_request(&buf).await },
                                                );

                                            let (request, response) = handle.await??;
                                            self.output_state.update(request, response);
                                        }
                                        Err(err) => match err {
                                            SubstituteError::ReplacementNotFound(pending) => {
                                                self.substitution_state = Some(SubstituteState {
                                                    pending_options,
                                                    pending_substitution: pending,
                                                    select_state: SelectState::new(Vec::new()),
                                                });
                                            }
                                            e => bail!(e),
                                        },
                                    }
                                }
                            }
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
