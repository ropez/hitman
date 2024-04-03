use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, HighlightSpacing, List, ListState, StatefulWidget},
};

#[derive(Default)]
pub struct RequestSelector<'l> {
    search: &'l str,
}

impl<'l> RequestSelector<'l> {
    pub fn new(search: &'l str) -> Self {
        Self { search }
    }

    fn get_list_items<'a>(&self, state: &RequestSelectorState) -> Vec<Line<'a>> {
        if self.search.is_empty() {
            state.items.iter().map(|s| Line::from(s.clone())).collect()
        } else {
            let matcher = SkimMatcherV2::default();

            // FIXME: Don't include '.http' in fuzzy match

            let mut items: Vec<_> = state
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
}

impl<'l> StatefulWidget for RequestSelector<'l> {
    type State = RequestSelectorState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let list_items = self.get_list_items(state);

        let list = List::new(list_items)
            .block(
                Block::new()
                    .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                    .title("Requests"),
            )
            .highlight_style(Style::new().reversed())
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always);

        list.render(area, buf, &mut state.list_state);
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

    pub fn select_first(&mut self) {
        self.list_state.select(Some(0));
    }

    pub fn select_next(&mut self) {
        // FIXME: Can select outside filtered range,
        // should probably constraint during rendering
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