use crossterm::event::{Event, KeyCode};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use tui_input::{backend::crossterm::EventHandler, Input};

use super::Component;

pub struct Prompt {
    title: String,
    input: Input,
    has_value: bool,
}

impl Prompt {
    pub fn new(title: String) -> Self {
        Self {
            title,
            input: Input::default(),
            has_value: false,
        }
    }

    pub fn with_value(self, value: String) -> Self {
        Self {
            input: self.input.with_value(value),
            ..self
        }
    }

    pub fn value(&self) -> String {
        self.input.value().into()
    }
}

impl Component for Prompt {
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

        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(self.input.value()).block(block),
            area,
        );

        frame.set_cursor(inner.x + self.input.visual_cursor() as u16, inner.y);
    }

    fn handle_event(&mut self, event: &Event) -> bool {
        // TODO: Follow this pattern in our components:
        // Return a "StateChanged" option form event handlers.

        if let Event::Key(key) = event {
            if let KeyCode::Enter = key.code {
                return false;
            }
        }

        if let Some(state_changed) = self.input.handle_event(event) {
            self.has_value = state_changed.value;
        }
        true
    }
}
