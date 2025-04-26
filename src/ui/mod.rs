use crossterm::event::Event;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

pub mod app;
pub mod datepicker;
pub mod keymap;
pub mod output;
pub mod progress;
pub mod prompt;
pub mod select;
pub mod help;

pub trait Component {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect);
}

pub trait InteractiveComponent: Component {
    type Intent;

    fn handle_event(&mut self, event: &Event) -> Option<Self::Intent>;
}

pub enum PromptIntent {
    Abort,
    Accept(String),
}

pub trait PromptComponent: Component {
    fn handle_prompt(&mut self, event: &Event) -> Option<PromptIntent>;
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let vl = Layout::new(
        Direction::Vertical,
        [
            Constraint::Min(0),
            Constraint::Length(h),
            Constraint::Min(0),
        ],
    )
    .split(area);
    let hl = Layout::new(
        Direction::Horizontal,
        [
            Constraint::Min(0),
            Constraint::Length(w),
            Constraint::Min(0),
        ],
    )
    .split(vl[1]);

    hl[1]
}
