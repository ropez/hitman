use crossterm::event::Event;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    symbols::{border, line},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, HighlightSpacing, List, ListItem, ListState,
        Paragraph,
    },
    Frame,
};
use tui_input::{backend::crossterm::EventHandler, Input};

use super::{
    keymap::{mapkey, KeyMapping},
    Component, InteractiveComponent, PromptComponent, PromptIntent,
};

#[derive(Default)]
pub struct RequestSelector {
    pub selector: Select<String>,
}

impl RequestSelector {
    pub fn new() -> Self {
        Self {
            selector: Select::new(
                "Requests".into(),
                "Search".into(),
                Vec::new(),
            ),
        }
    }

    pub fn populate(&mut self, reqs: Vec<String>) {
        self.selector.set_items(reqs);
    }

    pub fn try_select(&mut self, selected: &String) {
        self.selector.try_select(selected);
    }
}

impl Component for RequestSelector {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        self.selector.render_ui(frame, area);
    }
}

impl InteractiveComponent for RequestSelector {
    type Intent = SelectIntent<String>;

    fn handle_event(&mut self, event: &Event) -> Option<Self::Intent> {
        self.selector.handle_event(event)
    }
}

fn format_item<'a>(text: &str, indexes: &[usize]) -> ListItem<'a> {
    // FIXME: Make '.http' part dark gray
    // For this, we need to implement SelectItem specifically for request paths

    Line::from(
        text.chars()
            .enumerate()
            .map(|(i, c)| {
                Span::from(String::from(c)).style(if indexes.contains(&i) {
                    Style::new().yellow()
                } else {
                    Style::new()
                })
            })
            .collect::<Vec<_>>(),
    )
    .into()
}

pub trait SelectItem {
    fn text(&self) -> String;

    fn render<'a>(&self) -> ListItem<'a> {
        self.text().into()
    }

    fn render_highlighted<'a>(&self, highlight: &[usize]) -> ListItem<'a> {
        format_item(&self.text(), highlight)
    }
}

pub trait PromptSelectItem: SelectItem {
    fn to_value(&self) -> String;
}

impl SelectItem for String {
    fn text(&self) -> String {
        self.clone()
    }
}

#[derive(Default)]
pub struct Select<T>
where
    T: SelectItem + Clone,
{
    title: String,
    prompt: String,
    items: Vec<T>,
    list_state: ListState,
    search_input: Input,
}

#[derive(Debug, Clone)]
pub enum SelectIntent<T>
where
    T: SelectItem + Clone,
{
    Abort,
    Accept(T),
    Change(Option<T>),
}

impl<T> Select<T>
where
    T: SelectItem + Clone,
{
    pub fn new(title: String, prompt: String, items: Vec<T>) -> Self {
        Self {
            title,
            prompt,
            items,
            list_state: ListState::default().with_selected(Some(0)),
            search_input: Input::default(),
        }
    }

    pub fn set_items(&mut self, items: Vec<T>) {
        self.items = items;
        self.list_state.select(None);
    }

    pub fn selected_item(&self) -> Option<&T> {
        self.list_state.selected().and_then(|i| {
            let filtered = &self.get_filtered_items();
            filtered.get(i).map(|&(i, _)| i)
        })
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

    fn get_filtered_items(&self) -> Vec<(&T, Option<Vec<usize>>)> {
        let term = self.search_input.value();
        if term.is_empty() {
            self.items.iter().map(|i| (i, None)).collect()
        } else {
            let matcher = SkimMatcherV2::default();

            let mut items: Vec<_> = self
                .items
                .iter()
                .filter_map(|s| {
                    matcher
                        .fuzzy(&s.text(), term, true)
                        .map(|(score, indexes)| (s, score, indexes))
                })
                .collect();

            items.sort_by_key(|(_, score, _)| -score);

            items
                .into_iter()
                .map(|(i, _, indexes)| (i, Some(indexes)))
                .collect()
        }
    }

    fn get_list_items<'a>(&self) -> Vec<ListItem<'a>> {
        self.get_filtered_items()
            .into_iter()
            .map(|(i, highlight)| {
                highlight
                    .map_or_else(|| i.render(), |hl| i.render_highlighted(&hl))
            })
            .collect()
    }

    pub fn try_select(&mut self, item: &T)
    where
        T: PartialEq,
    {
        if let Some(pos) = self
            .get_filtered_items()
            .iter()
            .position(|(i, _)| item.eq(i))
        {
            self.list_state.select(Some(pos));
        }
    }
}

impl<T> Component for Select<T>
where
    T: SelectItem + Clone,
{
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);

        let layout = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Length(3)],
        )
        .split(area);

        let list_items = self.get_list_items();

        let list = List::new(list_items)
            .block(
                Block::bordered()
                    .border_set(border::Set {
                        bottom_left: border::PLAIN.bottom_left,
                        bottom_right: border::PLAIN.bottom_right,
                        ..border::ROUNDED
                    })
                    .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                    .title(self.title.clone()),
            )
            .highlight_style(Style::new().reversed())
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always);

        frame.render_stateful_widget(list, layout[0], &mut self.list_state);

        let label = Span::from(format!(" {}: ", self.prompt));
        let cur = self.search_input.visual_cursor() + label.width();
        let text = Line::from(vec![
            label,
            Span::from(self.search_input.value()).yellow(),
        ]);
        let block = Block::bordered().border_set(border::Set {
            top_left: line::NORMAL.vertical_right,
            top_right: line::NORMAL.vertical_left,
            ..border::ROUNDED
        });
        let inner = block.inner(layout[1]);
        frame.render_widget(Paragraph::new(text).block(block), layout[1]);

        frame.set_cursor_position((inner.x + cur as u16, inner.y));
    }
}

impl<T> InteractiveComponent for Select<T>
where
    T: SelectItem + Clone,
{
    type Intent = SelectIntent<T>;

    fn handle_event(&mut self, event: &Event) -> Option<Self::Intent> {
        match mapkey(event) {
            KeyMapping::Up => {
                self.select_prev();
                return Some(SelectIntent::Change(
                    self.selected_item().cloned(),
                ));
            }
            KeyMapping::Down => {
                self.select_next();
                return Some(SelectIntent::Change(
                    self.selected_item().cloned(),
                ));
            }
            KeyMapping::Accept => {
                if let Some(item) = self.selected_item() {
                    return Some(SelectIntent::Accept(item.clone()));
                }
            }
            KeyMapping::Abort => {
                return Some(SelectIntent::Abort);
            }
            KeyMapping::None => {
                if let Some(change) = self.search_input.handle_event(event) {
                    if change.value {
                        self.select_first();
                        return Some(SelectIntent::Change(
                            self.selected_item().cloned(),
                        ));
                    }
                }
            }
            _ => (),
        }

        None
    }
}

impl<T> PromptComponent for Select<T>
where
    T: PromptSelectItem + Clone,
{
    fn handle_prompt(&mut self, event: &Event) -> Option<PromptIntent> {
        self.handle_event(event).and_then(|intent| match intent {
            SelectIntent::Abort => Some(PromptIntent::Abort),
            SelectIntent::Accept(item) => {
                Some(PromptIntent::Accept(item.to_value()))
            }
            SelectIntent::Change(_) => None,
        })
    }
}
