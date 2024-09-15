use crossterm::event::Event;
use ratatui::{
    layout::Rect,
    style::Stylize,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use tui_input::{backend::crossterm::EventHandler, Input};

use super::{
    centered,
    keymap::{mapkey, KeyMapping},
    Component, PromptComponent, PromptIntent,
};

pub struct SimplePrompt {
    title: String,
    fallback: Option<String>,
    input: Input,
}

impl SimplePrompt {
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

impl Component for SimplePrompt {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let area = centered(area, 40, 3);

        let block = Block::bordered().cyan().title(self.title.clone());
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
            Paragraph::new(Line::from(spans)).white().block(block),
            area,
        );

        frame.set_cursor_position((
            inner.x + cur + self.input.visual_cursor() as u16,
            inner.y,
        ));
    }
}

impl PromptComponent for SimplePrompt {
    fn handle_prompt(&mut self, event: &Event) -> Option<PromptIntent> {
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
