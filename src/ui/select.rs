use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style, Stylize},
    text::Text,
    widgets::{
        Block, BorderType, Borders, HighlightSpacing, List, ListState, Paragraph, StatefulWidget,
    },
    Frame,
};

#[derive(Default)]
pub struct RequestSelector;

impl StatefulWidget for RequestSelector {
    type State = RequestSelectorState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let list = List::new(state.get_items())
            .block(Block::bordered().title("Requests"))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol(">> ")
            .highlight_spacing(HighlightSpacing::Always);

        list.render(area, buf, &mut state.list_state);

        // XXX Search "inlay"
        let rect = Rect::new(
            area.x + area.width - 20,
            area.y + area.height - 2,
            20 - 1,
            1,
        );
        let s = Paragraph::new(state.search.clone()).style(Style::new().cyan());
        ratatui::widgets::Widget::render(s, rect, buf);
    }
}

pub struct RequestSelectorState {
    items: Vec<String>,
    list_state: ListState,
    search: String,
}

impl RequestSelectorState {
    pub fn new(reqs: &[String]) -> Self {
        Self {
            items: reqs.into_iter().map(|a| String::from(a)).collect(),
            list_state: ListState::default().with_selected(Some(0)),
            search: String::new(),
        }
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

    pub fn selected_path(&self) -> Option<&String> {
        match self.list_state.selected() {
            Some(i) => self.items.get(i),
            None => None,
        }
    }

    pub fn get_items(&self) -> Vec<String> {
        if self.search.is_empty() {
            self.items.clone()
        } else {
            self.items.clone().into_iter().filter(|s| s.contains(&self.search)).collect()
        }
    }

    pub fn input(&mut self, ch: char) {
        self.search.push(ch);
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
    }
}
