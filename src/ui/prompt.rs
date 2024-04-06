use crossterm::event::Event;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Clear, Paragraph},
    Frame, text::{Span, Line}, style::Stylize,
};
use tui_input::{backend::crossterm::EventHandler, Input};

use super::{
    keymap::{mapkey, KeyMapping},
    Component,
};

pub struct Prompt {
    title: String,
    fallback: Option<String>,
    input: Input,
}

pub enum PromptIntent {
    Abort,
    Accept(String),
}

impl Prompt {
    pub fn new(title: String) -> Self {
        Self {
            title,
            fallback: None,
            input: Input::default(),
        }
    }

    pub fn with_fallback(self, value: Option<String>) -> Self {
        Self {
            fallback: value,
            ..self
        }
    }

    fn value(&self) -> String {
        let input_value = self.input.value().to_string();
        if input_value.len() > 0 {
            input_value
        } else {
            self.fallback.clone().unwrap_or(input_value)
        }
    }
}

impl Component for Prompt {
    type Intent = PromptIntent;

    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::new(
            Direction::Vertical,
            [
                Constraint::Percentage(50),
                Constraint::Length(3),
                Constraint::Percentage(50),
            ],
        )
        .split(area);

        let area = chunks[1];

        let block = Block::bordered().title(self.title.clone());
        let inner = block.inner(area);

        let input_value = self.input.value();
        let mut spans = Vec::new();
        spans.push(Span::from("> "));
        spans.push(Span::from(input_value));
        let cur = spans[0].width() as u16;

        if input_value.len() == 0 {
            if let Some(value) = &self.fallback {
                spans.push(Span::from(value).dark_gray());
            }
        }

        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(Line::from(spans)).block(block),
            area,
        );

        frame.set_cursor(inner.x + cur + self.input.visual_cursor() as u16, inner.y);
    }

    fn handle_event(&mut self, event: &Event) -> Option<PromptIntent> {
        match mapkey(event) {
            KeyMapping::Accept => {
                return Some(PromptIntent::Accept(self.value()));
            }
            KeyMapping::Abort => {
                return Some(PromptIntent::Abort);
            }
            _ => (),
        }

        self.input.handle_event(event);

        None
    }
}
