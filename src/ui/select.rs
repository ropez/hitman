use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders, HighlightSpacing, List, ListState, StatefulWidget},
    Frame,
};

#[derive(Default)]
pub struct RequestSelector;

impl StatefulWidget for RequestSelector {
    type State = RequestSelectorState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let list = List::new(state.items.clone())
            .block(Block::default().title("Requests").borders(Borders::ALL))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol(">> ")
            .highlight_spacing(HighlightSpacing::Always);

        list.render(area, buf, &mut state.list_state);
    }
}

pub struct RequestSelectorState {
    items: Vec<String>,
    list_state: ListState,
}

impl RequestSelectorState {
    pub fn new(reqs: &[String]) -> Self {
        Self {
            items: reqs.into_iter().map(|a| String::from(a)).collect(),
            list_state: ListState::default().with_selected(Some(0)),
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
}
