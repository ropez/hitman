use crossterm::event::{Event, KeyCode, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, HighlightSpacing, List, ListItem, ListState,
        Paragraph,
    },
    Frame,
};

pub trait Component {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect);
    fn handle_event(&mut self, event: Event) -> bool;
}

#[derive(Default)]
pub struct RequestSelector {
    selector: Select<String>,
}

impl RequestSelector {
    pub fn new(reqs: &[String]) -> Self {
        let items = Vec::from(reqs);

        Self {
            selector: Select::new("Requests".into(), items),
        }
    }

    pub fn selected_path(&self) -> Option<&String> {
        self.selector.selected_item()
    }
}

impl Component for RequestSelector {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        self.selector.render_ui(frame, area);
    }

    fn handle_event(&mut self, event: Event) -> bool {
        self.selector.handle_event(event)
    }
}

fn format_item<'a>(text: String, indexes: &Vec<usize>) -> ListItem<'a> {
    // FIXME: Make '.http' part dark gray
    // For this, we need to implement SelectItem specifically for request paths

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
    ).into()
}

pub trait SelectItem {
    fn text(&self) -> String;

    fn render<'a>(&self) -> ListItem<'a> {
        self.text().into()
    }

    fn render_highlighted<'a>(&self, highlight: &Vec<usize>) -> ListItem<'a> {
        format_item(self.text(), highlight)
    }
}

impl SelectItem for String {
    fn text(&self) -> String {
        self.clone()
    }
}

#[derive(Default)]
pub struct Select<T>
where
    T: SelectItem,
{
    title: String,
    items: Vec<T>,
    list_state: ListState,
    search: String,
}

impl<T> Select<T>
where
    T: SelectItem
{
    pub fn new(title: String, items: Vec<T>) -> Self {
        Self {
            title,
            items,
            list_state: ListState::default().with_selected(Some(0)),
            search: String::new(),
        }
    }
}

impl<T> Select<T>
where
    T: SelectItem
{
    pub fn selected_item(&self) -> Option<&T> {
        if let Some(i) = self.list_state.selected() {
            let filtered = &self.get_filtered_items();
            filtered.get(i).map(|&(i, _)| i)
        } else {
            None
        }
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

    pub fn select_first(&mut self) {
        self.list_state.select(Some(0));
    }

    fn get_filtered_items<'a>(&self) -> Vec<(&T, Option<Vec<usize>>)> {
        if self.search.is_empty() {
            self.items.iter().map(|i| (i, None)).collect()
        } else {
            let matcher = SkimMatcherV2::default();

            let mut items: Vec<_> = self
                .items
                .iter()
                .filter_map(|s| {
                    matcher
                        .fuzzy(&s.text(), &self.search, true)
                        .map(|(score, indexes)| (s, score, indexes))
                })
                .collect();

            items.sort_by_key(|(_, score, _)| -score);

            items.into_iter().map(|(i, _, indexes)| (i, Some(indexes))).collect()
        }
    }

    fn get_list_items<'a>(&self) -> Vec<ListItem<'a>> {
        self.get_filtered_items().into_iter().map(|(i, highlight)| {
            if let Some(hl) = highlight {
                i.render_highlighted(&hl)
            } else {
                i.render()
            }
        }).collect()
    }
}

impl<T> Component for Select<T>
where
    T: SelectItem
{
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);

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
                    .title(self.title.clone()),
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
