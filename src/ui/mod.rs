use crossterm::event::Event;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

pub mod app;
pub mod output;
pub mod progress;
pub mod prompt;
pub mod select;
pub mod keymap;

pub trait Component {
    type Command;

    fn render_ui(&mut self, frame: &mut Frame, area: Rect);
    fn handle_event(&mut self, event: &Event) -> Option<Self::Command>;
}

pub(crate) fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let vl = Layout::new(
        Direction::Vertical,
        [
            Constraint::Percentage(50),
            Constraint::Length(w),
            Constraint::Percentage(50),
        ],
    )
    .split(area);
    let hl = Layout::new(
        Direction::Horizontal,
        [
            Constraint::Percentage(50),
            Constraint::Length(h),
            Constraint::Percentage(50),
        ],
    )
    .split(vl[1]);

    hl[1]
}
