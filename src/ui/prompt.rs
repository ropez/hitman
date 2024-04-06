use crossterm::event::Event;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use tui_input::{backend::crossterm::EventHandler, Input};

use super::{
    keymap::{mapkey, KeyMapping},
    Component,
};

pub struct Prompt {
    title: String,
    input: Input,
    has_value: bool,
}

pub enum PromptCommand {
    Abort,
    Accept(String),
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
    type Command = PromptCommand;

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

    fn handle_event(&mut self, event: &Event) -> Option<PromptCommand> {
        match mapkey(event) {
            KeyMapping::Accept => {
                return Some(PromptCommand::Accept(self.value()));
            }
            KeyMapping::Abort => {
                return Some(PromptCommand::Abort);
            }
            _ => (),
        }

        if let Some(state_changed) = self.input.handle_event(event) {
            self.has_value = state_changed.value;
        }

        None
    }
}
