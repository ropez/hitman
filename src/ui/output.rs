use crossterm::event::Event;
use ratatui::{
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::{
    keymap::{mapkey, KeyMapping},
    Component,
};

#[derive(Default)]
pub struct OutputView {
    request: String,
    response: String,
    scroll: (u16, u16),
}

impl OutputView {
    pub fn update(&mut self, request: String, response: String) {
        self.scroll = (0, 0);
        self.request = request;
        self.response = response;
    }

    pub fn scroll_up(&mut self) {
        if self.scroll.0 <= 5 {
            self.scroll.0 = 0;
        } else {
            self.scroll.0 -= 5;
        }
    }

    pub fn scroll_down(&mut self) {
        self.scroll.0 += 5;
    }
}

impl Component for OutputView {
    type Command = ();

    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let blue = Style::new().blue();
        let req_lines =
            self.request.lines().map(|line| Line::styled(line, blue));

        let yellow = Style::new().yellow();
        let res_lines =
            self.response.lines().map(|line| Line::styled(line, yellow));

        let lines: Vec<Line> = req_lines.chain(res_lines).collect();

        let para = Paragraph::new(Text::from(lines))
            .scroll(self.scroll)
            .block(Block::default().title("Output").borders(Borders::ALL));

        frame.render_widget(para, area);
    }

    fn handle_event(&mut self, event: &Event) -> Option<()> {
        match mapkey(event) {
            KeyMapping::ScrollUp => {
                self.scroll_up();
            }
            KeyMapping::ScrollDown => {
                self.scroll_down();
            }
            _ => (),
        }

        None
    }
}
