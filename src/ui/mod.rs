use crossterm::event::Event;
use ratatui::{Frame, layout::Rect};

pub mod app;
pub mod select;
pub mod prompt;
pub mod output;

pub trait Component {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect);
    fn handle_event(&mut self, event: &Event) -> bool;
}
