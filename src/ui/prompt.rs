use crossterm::event::Event;
use minijinja::Value as JinjaValue;
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
    input: Input,
}

impl SimplePrompt {
    pub fn new(title: String) -> Self {
        Self {
            title,
            input: Input::default(),
        }
    }

    fn value(&self) -> JinjaValue {
        JinjaValue::from(self.input.value().to_string())
    }
}

impl Component for SimplePrompt {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let area = centered(area, 40, 3);

        let block = Block::bordered().cyan().title(self.title.clone());
        let inner = block.inner(area);

        let input_value = self.input.value();
        let spans =
            vec![Span::from("> ").cyan(), Span::from(input_value).white()];
        let cur = spans[0].width() as u16;

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
