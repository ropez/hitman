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
    type Intent;

    fn render_ui(&mut self, frame: &mut Frame, area: Rect);

    fn handle_event(&mut self, _event: &Event) -> Option<Self::Intent> {
        None
    }
}

pub(crate) fn centered(area: Rect, w: u16, h: u16) -> Rect {
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
