use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, HighlightSpacing, List, ListState, Paragraph,
    },
    Frame,
};

pub trait Component {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect);
    fn handle_event(&mut self, event: Event) -> bool;
}

#[derive(Default)]
pub struct RequestSelector {
    search: String,
    items: Vec<String>,
    list_state: ListState,
}

impl RequestSelector {
    pub fn new(reqs: &[String]) -> Self {
        Self {
            search: String::new(),
            items: reqs.into_iter().map(|a| String::from(a)).collect(),
            list_state: ListState::default().with_selected(Some(0)),
        }
    }

    fn get_list_items<'a>(&self) -> Vec<Line<'a>> {
        if self.search.is_empty() {
            self.items.iter().map(|s| Line::from(s.clone())).collect()
        } else {
            let matcher = SkimMatcherV2::default();

            // FIXME: Don't include '.http' in fuzzy match

            let mut items: Vec<_> = self
                .items
                .iter()
                .filter_map(|s| {
                    matcher
                        .fuzzy(&s, &self.search, true)
                        .map(|(score, indexes)| (s, score, indexes))
                })
                .collect();

            items.sort_by_key(|(_, score, _)| -score);

            items
                .into_iter()
                .map(|(s, _, indexes)| format_item(s.clone(), indexes))
                .collect()
        }
    }

    pub fn select_first(&mut self) {
        self.list_state.select(Some(0));
    }

    pub fn select_next(&mut self) {
        let len = self.get_list_items().len();
        match self.list_state.selected() {
            None => self.list_state.select(Some(0)),
            Some(i) => self.list_state.select(Some((i + 1) % len)),
        }
    }

    pub fn select_prev(&mut self) {
        let len = self.get_list_items().len();
        match self.list_state.selected() {
            None => self.list_state.select(Some(len - 1)),
            Some(i) => self.list_state.select(Some((len + i - 1) % len)),
        }
    }

    pub fn selected_path(&self) -> Option<&String> {
        match self.list_state.selected() {
            Some(i) => self.items.get(i),
            None => None,
        }
    }
}

impl Component for RequestSelector {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let layout = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Max(3)],
        )
        .split(area);

        let list_items = self.get_list_items();

        let list = List::new(list_items)
            .block(
                Block::new()
                    .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                    .title("Requests"),
            )
            .highlight_style(Style::new().reversed())
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always);

        frame.render_stateful_widget(list, layout[0], &mut self.list_state);

        let text = Line::from(vec![
            Span::from("  Search: "),
            Span::from(self.search.clone()).yellow(),
        ]);
        let w = text.width() as u16;
        frame.render_widget(
            Paragraph::new(text).block(Block::bordered().border_set(
                symbols::border::Set {
                    top_left: symbols::border::PLAIN.vertical_right,
                    top_right: symbols::border::PLAIN.vertical_left,
                    ..symbols::border::PLAIN
                },
            )),
            layout[1],
        );

        // FIXME: Use tui-input to calculate cursor pos
        frame.set_cursor(layout[1].x + w + 1, layout[1].y + 1);
    }

    fn handle_event(&mut self, event: Event) -> bool {
        if let Event::Key(key) = event {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('j') => {
                        self.select_next();
                        true
                    }
                    KeyCode::Char('k') => {
                        self.select_prev();
                        true
                    }
                    KeyCode::Char('w') => {
                        self.search.clear();
                        true
                    }
                    _ => false,
                }
            } else {
                match key.code {
                    KeyCode::Down => {
                        self.select_next();
                        true
                    }
                    KeyCode::Up => {
                        self.select_prev();
                        true
                    }
                    KeyCode::Char(ch) => {
                        if self.search.len() < 24 {
                            self.search.push(ch);
                            self.select_first();
                        }
                        true
                    }
                    KeyCode::Backspace => {
                        self.search.pop();
                        true
                    }
                    KeyCode::Esc => {
                        self.search.clear();
                        true
                    }
                    _ => false,
                }
            }
        } else {
            false
        }
    }
}

fn format_item<'a>(text: String, indexes: Vec<usize>) -> Line<'a> {
    // FIXME: Make '.http' part dark gray

    Line::from(
        text.chars()
            .enumerate()
            .map(|(i, c)| {
                Span::from(String::from(c)).style(if indexes.contains(&i) {
                    Style::new().yellow()
                } else {
                    Style::new().clone()
                })
            })
            .collect::<Vec<_>>(),
    )
}

#[derive(Default)]
pub struct Select {
    title: String,
    items: Vec<String>,
    list_state: ListState,
}

impl Select {
    pub fn new(title: String, items: Vec<String>) -> Self {
        Self {
            title,
            items,
            list_state: ListState::default().with_selected(Some(0)),
        }
    }
}

impl Select {
    pub fn selected(&self) -> Option<usize> {
        self.list_state.selected()
    }

    pub fn select_next(&mut self) {
        let len = self.items.len();
        match self.list_state.selected() {
            None => self.list_state.select(Some(0)),
            Some(i) => self.list_state.select(Some((i + 1) % len)),
        }
    }

    pub fn select_prev(&mut self) {
        let len = self.items.len();
        match self.list_state.selected() {
            None => self.list_state.select(Some(len - 1)),
            Some(i) => self.list_state.select(Some((len + i - 1) % len)),
        }
    }
}

impl Component for Select {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);

        let list_items = self.items.clone();

        let list = List::new(list_items)
            .highlight_style(Style::new().reversed())
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always)
            .block(Block::bordered().title(self.title.clone()));

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    match key.code {
                        KeyCode::Char('j') => {
                            self.select_next();
                            true
                        }
                        KeyCode::Char('k') => {
                            self.select_prev();
                            true
                        }
                        _ => false,
                    }
                } else {
                    match key.code {
                        KeyCode::Down => {
                            self.select_next();
                            true
                        }
                        KeyCode::Up => {
                            self.select_prev();
                            true
                        }
                        _ => false,
                    }
                }
            }
            _ => false,
        }
    }
}
