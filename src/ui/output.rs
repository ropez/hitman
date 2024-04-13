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
pub struct HttpMessage {
    pub header: String,
    pub body: String,
}

#[derive(Default)]
pub struct OutputView {
    request: HttpMessage,
    response: HttpMessage,
    scroll: (u16, u16),
}

impl OutputView {
    pub fn update(&mut self, request: HttpMessage, response: HttpMessage) {
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
    type Intent = ();

    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let blue = Style::new().blue();
        let req_lines =
            self.request.header.lines().map(|line| Line::styled(line, blue));

        let green = Style::new().green();
        let res_lines =
            self.response.header.lines().map(|line| Line::styled(line, green));

        let normal = Style::new();
        let res_body_lines =
            self.response.body.lines().map(|line| Line::styled(line, normal));

        let lines: Vec<Line> = req_lines.chain(res_lines).chain(res_body_lines).collect();

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
